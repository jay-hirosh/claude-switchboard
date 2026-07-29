-- v6 -> v7: custom model providers.
--
-- Identical to the two statements appended to schema.sql; both are
-- IF NOT EXISTS, so running them on a fresh database is a harmless no-op.
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
