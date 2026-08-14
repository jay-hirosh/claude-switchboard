-- v11 -> v12: track which managed account was active over time, so local
-- session data (which carries no account identity of its own) can be
-- attributed after the fact by matching event timestamps against these
-- intervals. Additive only.
CREATE TABLE IF NOT EXISTS account_intervals (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    account_uuid TEXT NOT NULL,
    started_at   INTEGER NOT NULL,
    ended_at     INTEGER,
    FOREIGN KEY (account_uuid) REFERENCES accounts(id)
);
CREATE INDEX IF NOT EXISTS idx_account_intervals_span
    ON account_intervals(started_at, ended_at);
