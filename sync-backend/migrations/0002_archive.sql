CREATE TABLE archive_transcript_lines (
    seq BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    device_id UUID NOT NULL REFERENCES devices(id),
    project_slug TEXT NOT NULL,
    session_id TEXT NOT NULL,
    jsonl_path TEXT NOT NULL,
    line_no BIGINT NOT NULL,
    raw_line TEXT NOT NULL,
    ingested_at BIGINT NOT NULL,
    UNIQUE (device_id, jsonl_path, line_no)
);
CREATE INDEX idx_archive_transcript_lines_user_seq ON archive_transcript_lines(user_id, seq);

CREATE TABLE archive_file_snapshots (
    seq BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    device_id UUID NOT NULL REFERENCES devices(id),
    source_path TEXT NOT NULL,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    captured_at BIGINT NOT NULL,
    UNIQUE (device_id, source_path, content_hash)
);
CREATE INDEX idx_archive_file_snapshots_user_seq ON archive_file_snapshots(user_id, seq);
