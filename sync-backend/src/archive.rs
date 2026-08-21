use archive_sync_types::{PullQuery, PullResponse, PushRequest, PushResponse, SyncFileSnapshot, SyncTranscriptLine};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use std::sync::Arc;

use crate::auth::AuthedDevice;
use crate::AppState;

pub async fn push(
    State(state): State<Arc<AppState>>,
    device: AuthedDevice,
    Json(req): Json<PushRequest>,
) -> Result<Json<PushResponse>, (StatusCode, String)> {
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Serialize pushes per account: BIGSERIAL `seq` values are allocated at
    // INSERT time but only become visible to other transactions at COMMIT
    // time, so without this lock, commit order can differ from seq
    // allocation order under concurrency. That would let a pull's cursor
    // advance past a still-in-flight lower seq, permanently and silently
    // skipping it once it finally commits. This transaction-scoped
    // advisory lock ensures no other push transaction for the same account
    // can be concurrently open, so commit order always matches seq order.
    // Released automatically on commit or rollback.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(device.user_id.to_string())
        .execute(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut transcript_lines_accepted = 0usize;
    for line in &req.transcript_lines {
        let result = sqlx::query(
            "INSERT INTO archive_transcript_lines
             (user_id, device_id, project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (device_id, jsonl_path, line_no) DO NOTHING",
        )
        .bind(device.user_id)
        .bind(device.device_id)
        .bind(&line.project_slug)
        .bind(&line.session_id)
        .bind(&line.jsonl_path)
        .bind(line.line_no)
        .bind(&line.raw_line)
        .bind(line.ingested_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if result.rows_affected() == 1 {
            transcript_lines_accepted += 1;
        }
    }

    let mut file_snapshots_accepted = 0usize;
    for snap in &req.file_snapshots {
        let result = sqlx::query(
            "INSERT INTO archive_file_snapshots
             (user_id, device_id, source_path, kind, content, content_hash, captured_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (device_id, source_path, content_hash) DO NOTHING",
        )
        .bind(device.user_id)
        .bind(device.device_id)
        .bind(&snap.source_path)
        .bind(&snap.kind)
        .bind(&snap.content)
        .bind(&snap.content_hash)
        .bind(snap.captured_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if result.rows_affected() == 1 {
            file_snapshots_accepted += 1;
        }
    }

    tx.commit()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(PushResponse { transcript_lines_accepted, file_snapshots_accepted }))
}

pub async fn pull(
    State(state): State<Arc<AppState>>,
    device: AuthedDevice,
    Query(q): Query<PullQuery>,
) -> Result<Json<PullResponse>, (StatusCode, String)> {
    // Clamp server-side: an unbounded or negative page size from the client
    // must never translate into an unbounded query/response size.
    let limit = q.limit.clamp(1, 1000);

    // Note: this query is NOT filtered by device_id in SQL. It selects the
    // full window of rows for the account (including the caller's own),
    // so the high-water mark below reflects everything actually scanned —
    // not just what happened to survive a device_id filter. That's what
    // guarantees the cursor advances by up to `limit` rows on every pull,
    // even when a window happens to be dominated by the caller's own rows
    // (otherwise a device that just pushed a lot and is now idle would
    // rescan the same full range on every poll, forever). The caller's own
    // rows are excluded afterwards, in Rust, when building the response.
    let rows: Vec<(i64, uuid::Uuid, String, String, String, i64, String, i64)> = sqlx::query_as(
        "SELECT seq, device_id, project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at
         FROM archive_transcript_lines
         WHERE user_id = $1 AND seq > $2
         ORDER BY seq
         LIMIT $3",
    )
    .bind(device.user_id)
    .bind(q.since_transcript_seq)
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let transcript_seq_high_water = rows.last().map(|r| r.0).unwrap_or(q.since_transcript_seq);
    let transcript_lines: Vec<SyncTranscriptLine> = rows
        .into_iter()
        .filter(|r| r.1 != device.device_id)
        .map(|(_, _, project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)| SyncTranscriptLine {
            project_slug,
            session_id,
            jsonl_path,
            line_no,
            raw_line,
            ingested_at,
        })
        .collect();

    // Same restructuring as above, applied identically to file_snapshots.
    let rows: Vec<(i64, uuid::Uuid, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT seq, device_id, source_path, kind, content, content_hash, captured_at
         FROM archive_file_snapshots
         WHERE user_id = $1 AND seq > $2
         ORDER BY seq
         LIMIT $3",
    )
    .bind(device.user_id)
    .bind(q.since_snapshot_seq)
    .bind(limit)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let snapshot_seq_high_water = rows.last().map(|r| r.0).unwrap_or(q.since_snapshot_seq);
    let file_snapshots: Vec<SyncFileSnapshot> = rows
        .into_iter()
        .filter(|r| r.1 != device.device_id)
        .map(|(_, _, source_path, kind, content, content_hash, captured_at)| SyncFileSnapshot {
            source_path,
            kind,
            content,
            content_hash,
            captured_at,
        })
        .collect();

    Ok(Json(PullResponse {
        transcript_lines,
        file_snapshots,
        transcript_seq_high_water,
        snapshot_seq_high_water,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn register_device(pool: &PgPool, name: &str) -> (String, String) {
        let device_id = Uuid::new_v4().to_string();
        let request = Request::builder()
            .method("POST")
            .uri("/v1/accounts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({ "device_id": device_id, "device_name": name })).unwrap(),
            ))
            .unwrap();
        let response = app(pool.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (device_id, body["api_key"].as_str().unwrap().to_string())
    }

    async fn push_lines(pool: &PgPool, api_key: &str, lines: Vec<SyncTranscriptLine>) -> PushResponse {
        let body = PushRequest { transcript_lines: lines, file_snapshots: vec![] };
        let request = Request::builder()
            .method("POST")
            .uri("/v1/archive/push")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let response = app(pool.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn pull_lines(pool: &PgPool, api_key: &str) -> PullResponse {
        let request = Request::builder()
            .method("GET")
            .uri("/v1/archive/pull?since_transcript_seq=0&since_snapshot_seq=0&limit=500")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(pool.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn sample_line(n: i64) -> SyncTranscriptLine {
        SyncTranscriptLine {
            project_slug: "proj".into(),
            session_id: "sess".into(),
            jsonl_path: "proj/sess.jsonl".into(),
            line_no: n,
            raw_line: format!("{{\"n\":{n}}}"),
            ingested_at: 0,
        }
    }

    #[sqlx::test]
    async fn a_device_never_pulls_back_its_own_pushed_rows(pool: PgPool) {
        let (_device_a, key_a) = register_device(&pool, "A").await;
        let accepted = push_lines(&pool, &key_a, vec![sample_line(0), sample_line(1)]).await;
        assert_eq!(accepted.transcript_lines_accepted, 2);

        let pulled = pull_lines(&pool, &key_a).await;
        assert!(pulled.transcript_lines.is_empty(), "a device must not see its own rows in pull");
    }

    #[sqlx::test]
    async fn a_second_device_pulls_the_first_devices_pushed_rows(pool: PgPool) {
        let (_device_a, key_a) = register_device(&pool, "A").await;
        push_lines(&pool, &key_a, vec![sample_line(0), sample_line(1)]).await;

        // Device B joins the SAME account via the pairing flow, matching
        // real usage — a lone second /v1/accounts call would create a
        // second, unrelated user, which pull correctly scopes away from.
        let pair_request = Request::builder()
            .method("POST")
            .uri("/v1/devices/pair-code")
            .header("authorization", format!("Bearer {key_a}"))
            .body(Body::empty())
            .unwrap();
        let pair_response = app(pool.clone()).oneshot(pair_request).await.unwrap();
        let bytes = to_bytes(pair_response.into_body(), usize::MAX).await.unwrap();
        let code = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["pairing_code"]
            .as_str()
            .unwrap()
            .to_string();

        let device_b_id = Uuid::new_v4().to_string();
        let join_request = Request::builder()
            .method("POST")
            .uri("/v1/devices/join")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({ "pairing_code": code, "device_id": device_b_id, "device_name": "B" })).unwrap(),
            ))
            .unwrap();
        let join_response = app(pool.clone()).oneshot(join_request).await.unwrap();
        let bytes = to_bytes(join_response.into_body(), usize::MAX).await.unwrap();
        let key_b = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["api_key"]
            .as_str()
            .unwrap()
            .to_string();

        let pulled = pull_lines(&pool, &key_b).await;
        assert_eq!(pulled.transcript_lines.len(), 2, "device B must see device A's rows");
        assert_eq!(pulled.transcript_seq_high_water, 2);
    }

    #[sqlx::test]
    async fn re_pushing_the_same_batch_is_idempotent(pool: PgPool) {
        let (_device_a, key_a) = register_device(&pool, "A").await;
        let first = push_lines(&pool, &key_a, vec![sample_line(0)]).await;
        assert_eq!(first.transcript_lines_accepted, 1);
        let second = push_lines(&pool, &key_a, vec![sample_line(0)]).await;
        assert_eq!(second.transcript_lines_accepted, 0, "re-pushing an identical row must not duplicate it");
    }

    fn sample_snapshot(hash: &str) -> SyncFileSnapshot {
        SyncFileSnapshot {
            source_path: "/home/a/.claude/settings.json".into(),
            kind: "settings".into(),
            content: "{\"x\":1}".into(),
            content_hash: hash.into(),
            captured_at: 0,
        }
    }

    async fn push_snapshots(pool: &PgPool, api_key: &str, snapshots: Vec<SyncFileSnapshot>) -> PushResponse {
        let body = PushRequest { transcript_lines: vec![], file_snapshots: snapshots };
        let request = Request::builder()
            .method("POST")
            .uri("/v1/archive/push")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let response = app(pool.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // Fix 10: mirrors `re_pushing_the_same_batch_is_idempotent` but for
    // file_snapshots, proving the (device_id, source_path, content_hash)
    // ON CONFLICT target actually works and isn't just exercised for
    // transcript_lines.
    #[sqlx::test]
    async fn re_pushing_the_same_file_snapshot_is_idempotent(pool: PgPool) {
        let (_device_a, key_a) = register_device(&pool, "A").await;
        let first = push_snapshots(&pool, &key_a, vec![sample_snapshot("hash1")]).await;
        assert_eq!(first.file_snapshots_accepted, 1);
        let second = push_snapshots(&pool, &key_a, vec![sample_snapshot("hash1")]).await;
        assert_eq!(second.file_snapshots_accepted, 0, "re-pushing an identical snapshot must not duplicate it");
    }

    async fn pull_lines_since(pool: &PgPool, api_key: &str, since_transcript_seq: i64) -> PullResponse {
        let uri = format!(
            "/v1/archive/pull?since_transcript_seq={since_transcript_seq}&since_snapshot_seq=0&limit=500"
        );
        let request = Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(pool.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // Fix 9: proves Fix 1 (per-account advisory lock serializing push commit
    // order) and Fix 2 (high-water mark computed from the full scanned
    // window, not just the filtered response) actually work together. This
    // is the test that would have caught the original cursor-computation
    // bug, where the high-water mark was derived only from the (other
    // devices') filtered rows and so never advanced past a batch dominated
    // by the caller's own pushes.
    #[sqlx::test]
    async fn a_second_devices_cursor_advances_and_never_redelivers(pool: PgPool) {
        let (_device_a, key_a) = register_device(&pool, "A").await;

        let pair_request = Request::builder()
            .method("POST")
            .uri("/v1/devices/pair-code")
            .header("authorization", format!("Bearer {key_a}"))
            .body(Body::empty())
            .unwrap();
        let pair_response = app(pool.clone()).oneshot(pair_request).await.unwrap();
        let bytes = to_bytes(pair_response.into_body(), usize::MAX).await.unwrap();
        let code = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["pairing_code"]
            .as_str()
            .unwrap()
            .to_string();

        let device_b_id = Uuid::new_v4().to_string();
        let join_request = Request::builder()
            .method("POST")
            .uri("/v1/devices/join")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({ "pairing_code": code, "device_id": device_b_id, "device_name": "B" })).unwrap(),
            ))
            .unwrap();
        let join_response = app(pool.clone()).oneshot(join_request).await.unwrap();
        let bytes = to_bytes(join_response.into_body(), usize::MAX).await.unwrap();
        let key_b = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["api_key"]
            .as_str()
            .unwrap()
            .to_string();

        // Device A pushes 2 lines (seq 1, 2).
        push_lines(&pool, &key_a, vec![sample_line(0), sample_line(1)]).await;

        // Device B pulls from scratch and gets both.
        let first_pull = pull_lines_since(&pool, &key_b, 0).await;
        assert_eq!(first_pull.transcript_lines.len(), 2);
        assert_eq!(first_pull.transcript_seq_high_water, 2);

        // Device A pushes one more line (seq 3).
        push_lines(&pool, &key_a, vec![sample_line(2)]).await;

        // Device B pulls again using the high-water mark from its first
        // pull as the cursor — it must receive ONLY the new line, not a
        // re-delivery of the first two.
        let second_pull = pull_lines_since(&pool, &key_b, first_pull.transcript_seq_high_water).await;
        assert_eq!(second_pull.transcript_lines.len(), 1, "must receive only the new line, not a re-delivery");
        assert_eq!(second_pull.transcript_lines[0].line_no, 2);
        assert_eq!(second_pull.transcript_seq_high_water, 3);
    }

    #[sqlx::test]
    async fn push_and_pull_require_auth(pool: PgPool) {
        let request = Request::builder()
            .method("GET")
            .uri("/v1/archive/pull?since_transcript_seq=0&since_snapshot_seq=0&limit=10")
            .body(Body::empty())
            .unwrap();
        let response = app(pool.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        // The test's own name promises both endpoints are covered — the
        // brief only exercised pull. Push stamps device identity from
        // AuthedDevice, so an unauthenticated push must be rejected before
        // ever touching the database.
        let push_request = Request::builder()
            .method("POST")
            .uri("/v1/archive/push")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&PushRequest { transcript_lines: vec![], file_snapshots: vec![] }).unwrap(),
            ))
            .unwrap();
        let push_response = app(pool).oneshot(push_request).await.unwrap();
        assert_eq!(push_response.status(), StatusCode::UNAUTHORIZED);
    }
}
