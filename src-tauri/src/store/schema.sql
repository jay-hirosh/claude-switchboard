-- v2 schema (see store/migrations/ for upgrades from older versions).
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;

CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    display_name TEXT,
    last_seen_at INTEGER NOT NULL,
    warmup_enabled INTEGER NOT NULL DEFAULT 0,
    schedule TEXT NOT NULL DEFAULT '{"type":"Off"}',
    last_warmup_at INTEGER
);

CREATE TABLE IF NOT EXISTS api_snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL,
    fetched_at INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    FOREIGN KEY (account_id) REFERENCES accounts(id)
);
CREATE INDEX IF NOT EXISTS idx_snapshots_account_time
    ON api_snapshots(account_id, fetched_at DESC);

CREATE TABLE IF NOT EXISTS session_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,
    project TEXT NOT NULL,
    model TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_5m_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_1h_tokens INTEGER NOT NULL DEFAULT 0,
    cost_usd REAL NOT NULL DEFAULT 0,
    source_file TEXT NOT NULL,
    source_line INTEGER NOT NULL,
    -- Stable per-Claude-API-call identifier. Prefer "{requestId}:{message.id}"
    -- when both are present in the JSONL (modern Claude Code); fall back to
    -- "{source_file}:{source_line}" for older formats. The UNIQUE constraint
    -- here is what dedupes the same response written to multiple offsets in
    -- the same file (Claude Code does this on retries / partial rewinds —
    -- the (source_file, source_line) constraint we used in v1 missed it,
    -- inflating cost on busy days by 40%+).
    event_id TEXT NOT NULL,
    UNIQUE (event_id)
);
CREATE INDEX IF NOT EXISTS idx_events_ts ON session_events(ts DESC);
CREATE INDEX IF NOT EXISTS idx_events_project ON session_events(project);
CREATE INDEX IF NOT EXISTS idx_events_model ON session_events(model);

CREATE TABLE IF NOT EXISTS jsonl_cursors (
    file_path TEXT PRIMARY KEY,
    last_mtime_ns INTEGER NOT NULL,
    byte_offset INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS notification_state (
    account_id TEXT NOT NULL,
    bucket TEXT NOT NULL,
    threshold INTEGER NOT NULL,
    last_fired_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, bucket, threshold)
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Seed default settings rows (idempotent — safe to re-run on existing DBs).
INSERT OR IGNORE INTO settings (key, value) VALUES ('migration_completed', '0');
INSERT OR IGNORE INTO settings (key, value) VALUES ('warmup_consent_granted', '0');

CREATE TABLE IF NOT EXISTS providers (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    kind         TEXT NOT NULL DEFAULT 'third_party',
    base_url     TEXT,
    auth_token   TEXT,
    env_json     TEXT NOT NULL DEFAULT '{}',
    extra_args   TEXT NOT NULL DEFAULT '[]',
    preset_id    TEXT,
    sort_index   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS provider_default (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    provider_id   TEXT REFERENCES providers(id) ON DELETE SET NULL,
    managed_env   TEXT NOT NULL DEFAULT '{}',
    applied_at    INTEGER
);
