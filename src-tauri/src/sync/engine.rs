use crate::store::{Db, StoredFileSnapshot, StoredTranscriptLine};
use crate::sync::{SyncClient, SyncOutcome};
use archive_sync_types::{PushRequest, SyncFileSnapshot, SyncTranscriptLine};

const PUSH_BATCH_SIZE: i64 = 500;
const PULL_PAGE_SIZE: i64 = 500;
/// Cap a single push batch's cumulative `raw_line`/`content` payload at this
/// many bytes — a generous safety margin under the backend's 32MB body
/// limit. Individual file snapshots can be up to 5MB
/// (`archive_watcher::MAX_SNAPSHOT_BYTES`), so a batch of up to
/// `PUSH_BATCH_SIZE` rows could otherwise exceed the server's limit even
/// well under the row cap, causing a 413 that would retry identically
/// forever (the watermark never advances past a failed batch).
const PUSH_MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, PartialEq)]
pub enum SyncCycleResult {
    Ok { pushed: usize, pulled: usize },
    Unauthorized,
    Transient(String),
}

/// Turns a cycle result into the summary shape both `sync_now` and the
/// periodic background task display — one definition so the two call
/// sites can never drift on how a result maps to a status string. Also the
/// single place that logs a cycle's outcome: `SyncCycleResult::Transient`'s
/// message would otherwise be discarded the moment it's turned into a
/// summary, leaving no record of *why* a cycle failed.
pub fn summarize_cycle_result(result: SyncCycleResult, at: chrono::DateTime<chrono::Utc>) -> crate::app_state::SyncCycleSummary {
    let (outcome, pushed, pulled, last_error) = match result {
        SyncCycleResult::Ok { pushed, pulled } => {
            tracing::info!(pushed, pulled, "sync cycle completed");
            ("ok", pushed as u32, pulled as u32, None)
        }
        SyncCycleResult::Unauthorized => {
            tracing::warn!("sync cycle stopped: API key unauthorized (401) — needs re-pairing");
            ("unauthorized", 0, 0, None)
        }
        SyncCycleResult::Transient(msg) => {
            // Network-unreachable / server-error: expected and transient
            // for a personal backend that isn't always up — info, not warn.
            tracing::info!("sync cycle failed transiently (will retry next cycle): {msg}");
            ("transient", 0, 0, Some(msg))
        }
    };
    crate::app_state::SyncCycleSummary {
        last_run_at: at.to_rfc3339(),
        outcome: outcome.to_string(),
        pushed,
        pulled,
        last_error,
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

        // Cap this batch by cumulative payload size, not just row count —
        // see PUSH_MAX_BATCH_BYTES. Always include at least one row (the
        // `any_included` guard) so a single large row still makes forward
        // progress rather than stalling the cycle forever.
        let mut budget = PUSH_MAX_BATCH_BYTES;
        let mut any_included = false;
        let mut batch_lines = Vec::with_capacity(lines.len());
        for (id, line) in lines {
            let size = line.raw_line.len();
            if any_included && size > budget {
                break;
            }
            budget = budget.saturating_sub(size);
            any_included = true;
            batch_lines.push((id, line));
        }
        let mut batch_snaps = Vec::with_capacity(snaps.len());
        for (id, snap) in snaps {
            let size = snap.content.len();
            if any_included && size > budget {
                break;
            }
            budget = budget.saturating_sub(size);
            any_included = true;
            batch_snaps.push((id, snap));
        }

        let max_line_id = batch_lines.last().map(|(id, _)| *id);
        let max_snap_id = batch_snaps.last().map(|(id, _)| *id);

        let req = PushRequest {
            transcript_lines: batch_lines
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
            file_snapshots: batch_snaps
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
                //
                // A failure to persist the watermark itself must not be
                // silently swallowed: if it were, the next iteration would
                // re-read the unchanged watermark and re-push this exact
                // batch, potentially looping without bound.
                if let Some(id) = max_line_id {
                    if let Err(e) = db.set_sync_push_watermark("transcript_lines", id) {
                        tracing::warn!("failed to persist push watermark for transcript_lines: {e:#}");
                        return SyncCycleResult::Transient(e.to_string());
                    }
                }
                if let Some(id) = max_snap_id {
                    if let Err(e) = db.set_sync_push_watermark("file_snapshots", id) {
                        tracing::warn!("failed to persist push watermark for file_snapshots: {e:#}");
                        return SyncCycleResult::Transient(e.to_string());
                    }
                }
            }
            SyncOutcome::Unauthorized => {
                tracing::warn!("sync push unauthorized (401) — API key no longer valid");
                return SyncCycleResult::Unauthorized;
            }
            SyncOutcome::Transient(e) => {
                tracing::info!("sync push failed transiently: {e}");
                return SyncCycleResult::Transient(e);
            }
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

                if let Err(e) = db.set_sync_pull_watermark("transcript_lines", resp.transcript_seq_high_water) {
                    tracing::warn!("failed to persist pull watermark for transcript_lines: {e:#}");
                    return SyncCycleResult::Transient(e.to_string());
                }
                if let Err(e) = db.set_sync_pull_watermark("file_snapshots", resp.snapshot_seq_high_water) {
                    tracing::warn!("failed to persist pull watermark for file_snapshots: {e:#}");
                    return SyncCycleResult::Transient(e.to_string());
                }

                // The backend computes its LIMIT over the full scanned
                // window (including the caller's own rows) and only filters
                // those out afterward — so "fewer filtered rows came back
                // than the page limit" does NOT mean "caught up": a window
                // dominated by this device's own rows can legitimately
                // return 0 filtered rows while more data still exists past
                // it. The high-water mark is the only reliable "nothing
                // left" signal: stop only once neither cursor advanced.
                if resp.transcript_seq_high_water <= since_t && resp.snapshot_seq_high_water <= since_s {
                    break;
                }
            }
            SyncOutcome::Unauthorized => {
                tracing::warn!("sync pull unauthorized (401) — API key no longer valid");
                return SyncCycleResult::Unauthorized;
            }
            SyncOutcome::Transient(e) => {
                tracing::info!("sync pull failed transiently: {e}");
                return SyncCycleResult::Transient(e);
            }
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

    /// Regression test for the critical pagination bug: the real backend
    /// computes its LIMIT over the full scanned window (including the
    /// caller's own rows) and only filters those out server-side afterward
    /// — so a window dominated by the caller's own rows can legitimately
    /// return very few (or zero) filtered rows while much more data still
    /// exists past it. "Fewer rows returned than the limit" is therefore
    /// NOT a valid "caught up" signal on its own.
    ///
    /// Here the first page returns only 2 filtered rows but a HIGH
    /// transcript_seq_high_water (500) — simulating a window mostly made of
    /// the caller's own rows. Under the OLD buggy condition
    /// (`got_lines < PULL_PAGE_SIZE`), the loop would have stopped right
    /// there, since 2 < 500 — meaning the second mock below would never be
    /// hit. The fix must keep looping because the high-water mark (500)
    /// still exceeds the watermark before this page (0).
    #[tokio::test]
    async fn cycle_keeps_pulling_while_high_water_mark_advances_even_with_few_filtered_rows() {
        let (_dir, db) = test_db();

        let mut server = mockito::Server::new_async().await;
        // First call: since_transcript_seq=0. Only 2 filtered rows come
        // back, but the high-water mark is 500 — proof that a lot more
        // exists behind this window that the server filtered out.
        let first_pull = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"^/v1/archive/pull\?since_transcript_seq=0&since_snapshot_seq=0".to_string(),
                ),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"transcript_lines":[
                    {"project_slug":"p","session_id":"s0","jsonl_path":"p/s0.jsonl","line_no":0,"raw_line":"{}","ingested_at":0},
                    {"project_slug":"p","session_id":"s1","jsonl_path":"p/s1.jsonl","line_no":0,"raw_line":"{}","ingested_at":0}
                ],"file_snapshots":[],"transcript_seq_high_water":500,"snapshot_seq_high_water":0}"#,
            )
            .create_async()
            .await;
        // Second call: since_transcript_seq=500 (the advanced watermark).
        // Zero rows AND the high-water mark stays at 500 — this is the
        // genuine "caught up, nothing left" signal that must stop the loop.
        let second_pull = server
            .mock(
                "GET",
                mockito::Matcher::Regex(
                    r"^/v1/archive/pull\?since_transcript_seq=500&since_snapshot_seq=0".to_string(),
                ),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"transcript_lines":[],"file_snapshots":[],"transcript_seq_high_water":500,"snapshot_seq_high_water":0}"#,
            )
            .create_async()
            .await;

        let client = test_client(&server.url());
        let result = run_sync_cycle(&db, &client, "test-key").await;

        // The key assertion: BOTH mocks must have been hit. Under the old
        // buggy termination condition, only the first would ever fire.
        first_pull.assert_async().await;
        second_pull.assert_async().await;
        assert_eq!(result, SyncCycleResult::Ok { pushed: 0, pulled: 2 });
        assert_eq!(db.sync_pull_watermark("transcript_lines").unwrap(), 500);
    }

    /// Regression test for the push-batch payload-size cap: a handful of
    /// large rows (here, ~3MB `raw_line` values each) must not all be
    /// crammed into a single push call just because they're well under the
    /// 500-row cap — that could exceed the backend's 32MB body limit and
    /// cause a 413 that (before this fix) would retry identically forever.
    #[tokio::test]
    async fn push_batches_split_by_cumulative_payload_size_not_just_row_count() {
        let (_dir, db) = test_db();
        let big = "x".repeat(3 * 1024 * 1024); // ~3MB per row
        db.insert_transcript_lines(&[
            StoredTranscriptLine {
                project_slug: "p".into(), session_id: "s0".into(),
                jsonl_path: "p/s0.jsonl".into(), line_no: 0, raw_line: big.clone(),
            },
            StoredTranscriptLine {
                project_slug: "p".into(), session_id: "s1".into(),
                jsonl_path: "p/s1.jsonl".into(), line_no: 0, raw_line: big.clone(),
            },
            StoredTranscriptLine {
                project_slug: "p".into(), session_id: "s2".into(),
                jsonl_path: "p/s2.jsonl".into(), line_no: 0, raw_line: big,
            },
        ])
        .unwrap();

        let mut server = mockito::Server::new_async().await;
        // With an 8MB budget and ~3MB rows, the first batch fits 2 rows
        // (~6MB) and the third row must be deferred to a second push call
        // — proving the split happened on payload size, not row count
        // (3 rows is nowhere near the 500-row PUSH_BATCH_SIZE cap).
        let push_mock = server
            .mock("POST", "/v1/archive/push")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"transcript_lines_accepted":1,"file_snapshots_accepted":0}"#)
            .expect(2)
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
        assert!(matches!(result, SyncCycleResult::Ok { .. }), "expected Ok, got {result:?}");
        assert_eq!(
            db.sync_push_watermark("transcript_lines").unwrap(),
            Some(3),
            "all 3 rows must eventually be pushed across the split batches"
        );
    }
}
