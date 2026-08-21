-- v12 -> v13: durable local archive of ~/.claude + per-repo history.
-- Additive only — no existing table changes, no re-ingest. See
-- docs/superpowers/specs/2026-08-21-durable-claude-archive-design.md.
CREATE TABLE IF NOT EXISTS transcript_lines (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    project_slug  TEXT NOT NULL,
    session_id    TEXT NOT NULL,
    jsonl_path    TEXT NOT NULL,
    line_no       INTEGER NOT NULL,
    raw_line      TEXT NOT NULL,
    ingested_at   INTEGER NOT NULL,
    UNIQUE (jsonl_path, line_no)
);
CREATE INDEX IF NOT EXISTS idx_transcript_lines_path ON transcript_lines(jsonl_path);

CREATE TABLE IF NOT EXISTS file_snapshots (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    source_path  TEXT NOT NULL,
    kind         TEXT NOT NULL,
    content      TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    captured_at  INTEGER NOT NULL,
    UNIQUE (source_path, content_hash)
);
CREATE INDEX IF NOT EXISTS idx_file_snapshots_path ON file_snapshots(source_path);

-- Every pre-existing jsonl_cursors row was written by the pre-archive walker,
-- which cursors a file to its end without ever having called
-- insert_transcript_lines. ingest_file's own short-circuit (unchanged mtime +
-- unchanged length -> return early) means those files would otherwise never
-- be re-read again, so transcript_lines would silently stay empty for a
-- user's entire pre-existing history. Clearing jsonl_cursors forces a re-read
-- from byte 0 on the next backfill, which archives it. session_events is NOT
-- deleted: event_id is stable and UNIQUE, so re-reading is idempotent for
-- rows already stored (same pattern as 0006, 0008, 0009).
DELETE FROM jsonl_cursors;
