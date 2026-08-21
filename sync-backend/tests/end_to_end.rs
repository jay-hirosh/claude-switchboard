use archive_sync_types::{PullResponse, PushRequest, PushResponse, SyncFileSnapshot, SyncTranscriptLine};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

async fn call(pool: &PgPool, method: &str, uri: &str, api_key: Option<&str>, body: Option<serde_json::Value>) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(key) = api_key {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    let request = if let Some(b) = body {
        builder.header("content-type", "application/json").body(Body::from(serde_json::to_vec(&b).unwrap())).unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };
    let response = sync_backend::app(pool.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    // Error responses from this service are plain text (the blanket
    // `(StatusCode, String)` IntoResponse impl), not JSON. Every call in
    // this test asserts StatusCode::OK immediately afterward, but if a
    // regression ever made one of these calls fail, we want that surfaced
    // as a clear "expected OK, got 400/401/500" assertion failure — not a
    // confusing serde_json panic from trying to parse plain text as JSON.
    let json = if bytes.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| serde_json::json!({ "raw": String::from_utf8_lossy(&bytes).to_string() }))
    };
    (status, json)
}

/// Full lifecycle across two devices: create an account on device A, pair
/// and join device B onto the same account, push a mix of transcript
/// lines and file snapshots from A, pull them from B, verify content and
/// the high-water cursors, then push a second batch from B and confirm A
/// picks up exactly the new rows (not a re-delivery of what it already
/// pushed itself).
#[sqlx::test]
async fn two_device_full_sync_lifecycle(pool: PgPool) {
    let device_a = Uuid::new_v4().to_string();
    let (status, body) = call(&pool, "POST", "/v1/accounts", None, Some(serde_json::json!({ "device_id": device_a, "device_name": "A" }))).await;
    assert_eq!(status, StatusCode::OK);
    let key_a = body["api_key"].as_str().unwrap().to_string();

    let (status, body) = call(&pool, "POST", "/v1/devices/pair-code", Some(&key_a), Some(serde_json::json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    let code = body["pairing_code"].as_str().unwrap().to_string();

    let device_b = Uuid::new_v4().to_string();
    let (status, body) = call(&pool, "POST", "/v1/devices/join", None, Some(serde_json::json!({ "pairing_code": code, "device_id": device_b, "device_name": "B" }))).await;
    assert_eq!(status, StatusCode::OK);
    let key_b = body["api_key"].as_str().unwrap().to_string();

    let push_body = PushRequest {
        transcript_lines: vec![SyncTranscriptLine {
            project_slug: "proj".into(),
            session_id: "sess".into(),
            jsonl_path: "proj/sess.jsonl".into(),
            line_no: 0,
            raw_line: "{}".into(),
            ingested_at: 100,
        }],
        file_snapshots: vec![SyncFileSnapshot {
            source_path: "/home/a/.claude/settings.json".into(),
            kind: "settings".into(),
            content: "{\"x\":1}".into(),
            content_hash: "hash1".into(),
            captured_at: 100,
        }],
    };
    let (status, body) = call(&pool, "POST", "/v1/archive/push", Some(&key_a), Some(serde_json::to_value(&push_body).unwrap())).await;
    assert_eq!(status, StatusCode::OK);
    let pushed: PushResponse = serde_json::from_value(body).unwrap();
    assert_eq!(pushed.transcript_lines_accepted, 1);
    assert_eq!(pushed.file_snapshots_accepted, 1);

    let (status, body) = call(&pool, "GET", "/v1/archive/pull?since_transcript_seq=0&since_snapshot_seq=0&limit=500", Some(&key_b), None).await;
    assert_eq!(status, StatusCode::OK);
    let pulled: PullResponse = serde_json::from_value(body).unwrap();
    assert_eq!(pulled.transcript_lines.len(), 1);
    assert_eq!(pulled.transcript_lines[0].raw_line, "{}");
    assert_eq!(pulled.file_snapshots.len(), 1);
    assert_eq!(pulled.file_snapshots[0].content_hash, "hash1");
    assert_eq!(pulled.transcript_seq_high_water, 1);
    assert_eq!(pulled.snapshot_seq_high_water, 1);

    // Device B pushes its own new row; device A, pulling from its own
    // last-seen cursor (1, since it already has seq 1 — its own row),
    // should see only B's new row, not a re-delivery of its own.
    let push_body_b = PushRequest {
        transcript_lines: vec![SyncTranscriptLine {
            project_slug: "proj".into(),
            session_id: "sess-b".into(),
            jsonl_path: "proj/sess-b.jsonl".into(),
            line_no: 0,
            raw_line: "{\"from\":\"b\"}".into(),
            ingested_at: 200,
        }],
        file_snapshots: vec![],
    };
    let (status, _) = call(&pool, "POST", "/v1/archive/push", Some(&key_b), Some(serde_json::to_value(&push_body_b).unwrap())).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = call(&pool, "GET", "/v1/archive/pull?since_transcript_seq=1&since_snapshot_seq=1&limit=500", Some(&key_a), None).await;
    assert_eq!(status, StatusCode::OK);
    let pulled: PullResponse = serde_json::from_value(body).unwrap();
    assert_eq!(pulled.transcript_lines.len(), 1);
    assert_eq!(pulled.transcript_lines[0].raw_line, "{\"from\":\"b\"}");
}
