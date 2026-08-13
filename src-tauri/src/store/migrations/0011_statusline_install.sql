-- v10 → v11: track Switchboard's own statusLine install (F7). Additive
-- only — no existing table changes.
CREATE TABLE IF NOT EXISTS statusline_install (
    id                 INTEGER PRIMARY KEY CHECK (id = 1),
    prior_value        TEXT,
    installed_command  TEXT NOT NULL,
    installed_at       INTEGER NOT NULL
);
