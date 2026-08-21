use crate::store::{Db, StoredFileSnapshot, StoredTranscriptLine};
use crate::sync::{SyncClient, SyncOutcome};
use archive_sync_types::{PushRequest, SyncFileSnapshot, SyncTranscriptLine};

const PUSH_BATCH_SIZE: i64 = 500;
const PULL_PAGE_SIZE: i64 = 500;

#[derive(Debug, PartialEq)]
pub enum SyncCycleResult {
    Ok { pushed: usize, pulled: usize },
    Unauthorized,
    Transient(String),
}

/// Turns a cycle result into the summary shape both `sync_now` and the
/// periodic background task display — one definition so the two call
/// sites can never drift on how a result maps to a status string.
pub fn summarize_cycle_result(result: SyncCycleResult, at: chrono::DateTime<chrono::Utc>) -> crate::app_state::SyncCycleSummary {
    let (outcome, pushed, pulled) = match result {
        SyncCycleResult::Ok { pushed, pulled } => ("ok", pushed as u32, pulled as u32),
        SyncCycleResult::Unauthorized => ("unauthorized", 0, 0),
        SyncCycleResult::Transient(_) => ("transient", 0, 0),
    };
    crate::app_state::SyncCycleSummary {
        last_run_at: at.to_rfc3339(),
        outcome: outcome.to_string(),
        pushed,
        pulled,
    }
}

/// Runs one full push-then-pull cycle: pushes every not-yet-pushed local
/// row (in batches, advancing the push watermark only after each batch
/// fully succeeds — a partial batch failure must not skip rows), then
/// pulls every not-yet-seen remote row (paginating on the cursor until a
/// page comes back under the limit) and inserts it via the existing,
/// idempotent `Db::insert_transcript_lines`/`insert_file_snapshot`.
pub async fn run_sync_cycle(db: &Db, client: &SyncClient, api_key: &str) -> SyncCycleResult {
    let mut pushed = 0usize;
    loop {
        let since = db.sync_push_watermark("transcript_lines").unwrap_or(None).unwrap_or(0);
        let lines = match db.local_transcript_lines_since(since, PUSH_BATCH_SIZE) {
            Ok(l) => l,
            Err(e) => return SyncCycleResult::Transient(e.to_string()),
        };
        let since_snap = db.sync_push_watermark("file_snapshots").unwrap_or(None).unwrap_or(0);
        let snaps = match db.local_file_snapshots_since(since_snap, PUSH_BATCH_SIZE) {
            Ok(s) => s,
            Err(e) => return SyncCycleResult::Transient(e.to_string()),
        };
        if lines.is_empty() && snaps.is_empty() {
            break;
        }

        let max_line_id = lines.last().map(|(id, _)| *id);
        let max_snap_id = snaps.last().map(|(id, _)| *id);

        let req = PushRequest {
            transcript_lines: lines
                .into_iter()
                .map(|(_, l)| SyncTranscriptLine {
                    project_slug: l.project_slug,
                    session_id: l.session_id,
                    jsonl_path: l.jsonl_path,
                    line_no: l.line_no,
                    raw_line: l.raw_line,
                    ingested_at: 0,
                })
                .collect(),
            file_snapshots: snaps
                .into_iter()
                .map(|(_, s)| SyncFileSnapshot {
                    source_path: s.source_path,
                    kind: s.kind,
                    content: s.content,
                    content_hash: s.content_hash,
                    captured_at: 0,
                })
                .collect(),
        };

        match client.push(api_key, req).await {
            SyncOutcome::Ok(resp) => {
                pushed += resp.transcript_lines_accepted + resp.file_snapshots_accepted;
                // Watermark advances by "rows attempted," not "rows
                // accepted" — a row already accepted in a prior, partially
                // successful run must not be resent forever just because
                // the server correctly no-op'd it as a duplicate.
                if let Some(id) = max_line_id {
                    let _ = db.set_sync_push_watermark("transcript_lines", id);
                }
                if let Some(id) = max_snap_id {
                    let _ = db.set_sync_push_watermark("file_snapshots", id);
                }
            }
            SyncOutcome::Unauthorized => return SyncCycleResult::Unauthorized,
            SyncOutcome::Transient(e) => return SyncCycleResult::Transient(e),
        }
    }

    let mut pulled = 0usize;
    loop {
        let since_t = db.sync_pull_watermark("transcript_lines").unwrap_or(0);
        let since_s = db.sync_pull_watermark("file_snapshots").unwrap_or(0);
        match client.pull(api_key, since_t, since_s, PULL_PAGE_SIZE).await {
            SyncOutcome::Ok(resp) => {
                let got_lines = resp.transcript_lines.len();
                let got_snaps = resp.file_snapshots.len();
                pulled += got_lines + got_snaps;

                let to_insert: Vec<StoredTranscriptLine> = resp
                    .transcript_lines
                    .into_iter()
                    .map(|l| StoredTranscriptLine {
                        project_slug: l.project_slug,
                        session_id: l.session_id,
                        jsonl_path: l.jsonl_path,
                        line_no: l.line_no,
                        raw_line: l.raw_line,
                    })
                    .collect();
                // `insert_pulled_*`, not the plain `insert_transcript_lines`/
                // `insert_file_snapshot` the JSONL watcher uses: those stamp
                // rows with THIS device's own id, which would make a
                // pulled-in row indistinguishable from a local one and get
                // it pushed straight back (and duplicated, since the
                // backend's uniqueness key includes device_id) next cycle.
                if let Err(e) = db.insert_pulled_transcript_lines(&to_insert) {
                    return SyncCycleResult::Transient(e.to_string());
                }
                for snap in resp.file_snapshots {
                    if let Err(e) = db.insert_pulled_file_snapshot(&StoredFileSnapshot {
                        source_path: snap.source_path,
                        kind: snap.kind,
                        content: snap.content,
                        content_hash: snap.content_hash,
                    }) {
                        return SyncCycleResult::Transient(e.to_string());
                    }
                }

                let _ = db.set_sync_pull_watermark("transcript_lines", resp.transcript_seq_high_water);
                let _ = db.set_sync_pull_watermark("file_snapshots", resp.snapshot_seq_high_water);

                if got_lines < PULL_PAGE_SIZE as usize && got_snaps < PULL_PAGE_SIZE as usize {
                    break;
                }
            }
            SyncOutcome::Unauthorized => return SyncCycleResult::Unauthorized,
            SyncOutcome::Transient(e) => return SyncCycleResult::Transient(e),
        }
    }

    SyncCycleResult::Ok { pushed, pulled }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Db, StoredAccount};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn test_db() -> (tempfile::TempDir, Db) {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        db.upsert_account(&StoredAccount { id: "a".into(), email: "e".into(), display_name: None }).unwrap();
        (dir, db)
    }

    fn test_client(base_url: &str) -> SyncClient {
        SyncClient::new(Arc::new(reqwest::Client::new()), base_url.to_string())
    }

    #[tokio::test]
    async fn cycle_pushes_new_local_rows_and_advances_watermark() {
        let (_dir, db) = test_db();
        db.insert_transcript_lines(&[StoredTranscriptLine {
            project_slug: "p".into(), session_id: "s".into(),
            jsonl_path: "p/s.jsonl".into(), line_no: 0, raw_line: "{}".into(),
        }]).unwrap();

        let mut server = mockito::Server::new_async().await;
        let push_mock = server
            .mock("POST", "/v1/archive/push")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"transcript_lines_accepted":1,"file_snapshots_accepted":0}"#)
            .create_async()
            .await;
        let pull_mock = server
            .mock("GET", mockito::Matcher::Regex(r"^/v1/archive/pull".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"transcript_lines":[],"file_snapshots":[],"transcript_seq_high_water":0,"snapshot_seq_high_water":0}"#)
            .create_async()
            .await;

        let client = test_client(&server.url());
        let result = run_sync_cycle(&db, &client, "test-key").await;

        push_mock.assert_async().await;
        pull_mock.assert_async().await;
        assert_eq!(result, SyncCycleResult::Ok { pushed: 1, pulled: 0 });
        assert_eq!(db.sync_push_watermark("transcript_lines").unwrap(), Some(1));
    }

    #[tokio::test]
    async fn cycle_returns_unauthorized_and_does_not_advance_watermark_on_401() {
        let (_dir, db) = test_db();
        db.insert_transcript_lines(&[StoredTranscriptLine {
            project_slug: "p".into(), session_id: "s".into(),
            jsonl_path: "p/s.jsonl".into(), line_no: 0, raw_line: "{}".into(),
        }]).unwrap();

        let mut server = mockito::Server::new_async().await;
        server.mock("POST", "/v1/archive/push").with_status(401).create_async().await;

        let client = test_client(&server.url());
        let result = run_sync_cycle(&db, &client, "bad-key").await;

        assert_eq!(result, SyncCycleResult::Unauthorized);
        assert_eq!(db.sync_push_watermark("transcript_lines").unwrap(), None, "watermark must not advance on auth failure");
    }

    #[tokio::test]
    async fn cycle_paginates_pull_until_a_page_comes_back_under_the_limit() {
        // Nothing local to push, so the push loop breaks immediately without
        // ever calling the backend — this test is pull-pagination only.
        let (_dir, db) = test_db();

        let page_size = PULL_PAGE_SIZE as usize;
        let first_page: Vec<_> = (0..page_size)
            .map(|i| {
                serde_json::json!({
                    "project_slug": "p", "session_id": format!("s{i}"),
                    "jsonl_path": format!("p/s{i}.jsonl"), "line_no": 0,
                    "raw_line": "{}", "ingested_at": 0
                })
            })
            .collect();
        let first_body = serde_json::json!({
            "transcript_lines": first_page,
            "file_snapshots": [],
            "transcript_seq_high_water": page_size,
            "snapshot_seq_high_water": 0
        })
        .to_string();

        let mut server = mockito::Server::new_async().await;
        // First call starts from watermark 0 and gets back a FULL page
        // (exactly PULL_PAGE_SIZE rows) — the loop must not treat this as
        // "everything," and must call again from the new watermark.
        let first_pull = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"^/v1/archive/pull\?since_transcript_seq=0&since_snapshot_seq=0".to_string(),
                ),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(first_body)
            .create_async()
            .await;
        // Second call starts from the advanced watermark and gets back an
        // under-the-limit (empty) page — this is what must stop the loop.
        let second_pull = server
            .mock(
                "GET",
                mockito::Matcher::Regex(format!(
                    r"^/v1/archive/pull\?since_transcript_seq={page_size}&since_snapshot_seq=0"
                )),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(
                r#"{{"transcript_lines":[],"file_snapshots":[],"transcript_seq_high_water":{page_size},"snapshot_seq_high_water":0}}"#
            ))
            .create_async()
            .await;

        let client = test_client(&server.url());
        let result = run_sync_cycle(&db, &client, "test-key").await;

        first_pull.assert_async().await;
        second_pull.assert_async().await;
        assert_eq!(result, SyncCycleResult::Ok { pushed: 0, pulled: page_size });
        assert_eq!(db.sync_pull_watermark("transcript_lines").unwrap(), page_size as i64);
        assert_eq!(
            db.local_transcript_lines_since(0, 10_000).unwrap().len(),
            0,
            "pulled-in rows are stamped with the OTHER device's id, never re-offered for push"
        );
    }
}
