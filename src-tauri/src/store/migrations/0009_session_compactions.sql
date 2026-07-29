-- v8 → v9: record session compactions.
--
-- Compaction is written as a `type:"system"` line with a compactMetadata
-- object, appended mid-transcript. The walker previously skipped every
-- non-assistant line, so these were never captured.
--
-- Clearing jsonl_cursors forces a re-read from byte 0 on the next backfill,
-- which backfills the new table. session_events is NOT deleted: event_id is
-- stable and UNIQUE, so the re-read is idempotent for rows already stored
-- (same reasoning as 0008).
CREATE TABLE IF NOT EXISTS session_compactions (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ts           INTEGER NOT NULL,
    source_file  TEXT NOT NULL,
    trigger_kind TEXT NOT NULL,
    pre_tokens   INTEGER NOT NULL DEFAULT 0,
    post_tokens  INTEGER NOT NULL DEFAULT 0,
    uuid         TEXT NOT NULL,
    UNIQUE (uuid)
);
CREATE INDEX IF NOT EXISTS idx_compactions_ts ON session_compactions(ts DESC);

DELETE FROM jsonl_cursors;
