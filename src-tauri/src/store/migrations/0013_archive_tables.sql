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
