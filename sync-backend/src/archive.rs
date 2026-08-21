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
    let rows: Vec<(i64, String, String, String, i64, String, i64)> = sqlx::query_as(
        "SELECT seq, project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at
         FROM archive_transcript_lines
         WHERE user_id = $1 AND device_id != $2 AND seq > $3
         ORDER BY seq
         LIMIT $4",
    )
    .bind(device.user_id)
    .bind(device.device_id)
    .bind(q.since_transcript_seq)
    .bind(q.limit)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let transcript_seq_high_water = rows.last().map(|r| r.0).unwrap_or(q.since_transcript_seq);
    let transcript_lines: Vec<SyncTranscriptLine> = rows
        .into_iter()
        .map(|(_, project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)| SyncTranscriptLine {
            project_slug,
            session_id,
            jsonl_path,
            line_no,
            raw_line,
            ingested_at,
        })
        .collect();

    let rows: Vec<(i64, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT seq, source_path, kind, content, content_hash, captured_at
         FROM archive_file_snapshots
         WHERE user_id = $1 AND device_id != $2 AND seq > $3
         ORDER BY seq
         LIMIT $4",
    )
    .bind(device.user_id)
    .bind(device.device_id)
    .bind(q.since_snapshot_seq)
    .bind(q.limit)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let snapshot_seq_high_water = rows.last().map(|r| r.0).unwrap_or(q.since_snapshot_seq);
    let file_snapshots: Vec<SyncFileSnapshot> = rows
        .into_iter()
        .map(|(_, source_path, kind, content, content_hash, captured_at)| SyncFileSnapshot {
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
