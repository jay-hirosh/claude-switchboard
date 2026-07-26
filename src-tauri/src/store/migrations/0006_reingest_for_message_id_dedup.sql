-- v5 → v6: re-ingest all session_events so relay-model turns dedupe correctly.
--
-- Third-party relays (GLM, k3, MiniMax, kimi) omit `requestId`, so the v2
-- dedup key "{requestId}:{message.id}" could not be built and the walker
-- fell back to a per-line "{source_file}:{byte_offset}" key. Those relays
-- also write each response's usage to multiple JSONL lines (one per content
-- block — up to 8x observed), so every duplicate line was counted separately,
-- inflating relay-model token and cost totals roughly 2-8x.
--
-- The parser now keys on `message.id` alone when `requestId` is absent. We
-- cannot rewrite the old line-based event_ids for rows already stored, so we
-- drop them and re-ingest from the JSONL source of truth. Clearing
-- jsonl_cursors forces the walker to re-read every file from byte 0 on the
-- next backfill. Non-destructive: the .jsonl files on disk are authoritative.
DELETE FROM session_events;
DELETE FROM jsonl_cursors;
