use claude_switchboard_lib::jsonl_parser::PricingTable;
use claude_switchboard_lib::store::{Db, StoredTranscriptLine};
use claude_switchboard_lib::sync::{engine::run_sync_cycle, SyncClient, SyncOutcome};
use std::sync::Arc;

/// Requires a real sync-backend instance reachable at SYNC_E2E_BASE_URL
/// (point this at a LOCAL TEST instance only — e.g. the same
/// `sync-backend-dev-pg`-backed server used during that plan's own
/// development, run via `cargo run -p sync-backend` with DATABASE_URL set
/// to a test database. NEVER point this at a real production backend).
/// Skips (passes trivially) if the env var isn't set, so this doesn't
/// break `cargo test` for anyone who hasn't started a local server.
#[tokio::test]
async fn desktop_app_can_push_and_pull_against_a_real_backend() {
    let Ok(base_url) = std::env::var("SYNC_E2E_BASE_URL") else {
        eprintln!("SYNC_E2E_BASE_URL not set — skipping (this is expected in normal `cargo test` runs)");
        return;
    };

    let client = SyncClient::new(Arc::new(reqwest::Client::new()), base_url);
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path()).unwrap();
    let device_id = db.device_id().unwrap();

    let api_key = match client.create_account(device_id, "e2e-test-device".to_string()).await {
        SyncOutcome::Ok(resp) => resp.api_key,
        other => panic!("account creation failed: {other:?}"),
    };
    db.set_sync_api_key(&api_key).unwrap();

    db.insert_transcript_lines(&[StoredTranscriptLine {
        project_slug: "e2e".into(),
        session_id: "session-1".into(),
        jsonl_path: "e2e/session-1.jsonl".into(),
        line_no: 0,
        raw_line: "{\"hello\":\"world\"}".into(),
    }])
    .unwrap();

    let pricing = PricingTable::bundled().unwrap();
    let result = run_sync_cycle(&db, &client, &api_key, &pricing).await;
    match result {
        claude_switchboard_lib::sync::engine::SyncCycleResult::Ok { pushed, .. } => {
            assert_eq!(pushed, 1);
        }
        other => panic!("sync cycle failed: {other:?}"),
    }
}
