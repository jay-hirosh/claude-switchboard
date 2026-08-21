-- v13 -> v14: add device_id so locally-recorded rows carry provenance, and
-- so pulled-in rows from other devices (phase 2 sync) can be distinguished
-- from this install's own. transcript_lines' existing UNIQUE(jsonl_path,
-- line_no) already disambiguates correctly across devices (jsonl_path
-- embeds Claude Code's own globally-unique session UUID), so it's a plain
-- ADD COLUMN. file_snapshots' source_path is a literal absolute path that
-- CAN collide across two different machines (e.g. both have
-- "/Users/x/.claude/settings.json" as different, unrelated files) — its
-- UNIQUE constraint must include device_id, which SQLite can only do via a
-- table rebuild (no ALTER TABLE ... ADD CONSTRAINT in SQLite).
ALTER TABLE transcript_lines ADD COLUMN device_id TEXT NOT NULL DEFAULT '';

CREATE TABLE file_snapshots_v14 (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id    TEXT NOT NULL DEFAULT '',
    source_path  TEXT NOT NULL,
    kind         TEXT NOT NULL,
    content      TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    captured_at  INTEGER NOT NULL,
    UNIQUE (device_id, source_path, content_hash)
);
INSERT INTO file_snapshots_v14 (id, source_path, kind, content, content_hash, captured_at)
    SELECT id, source_path, kind, content, content_hash, captured_at FROM file_snapshots;
DROP TABLE file_snapshots;
ALTER TABLE file_snapshots_v14 RENAME TO file_snapshots;
CREATE INDEX IF NOT EXISTS idx_file_snapshots_path ON file_snapshots(source_path);
