-- v9 → v10: track per-window rate-limit peaks for the limit-hit analytics
-- report (F5). Additive only — no existing table changes, no re-ingest.
CREATE TABLE IF NOT EXISTS window_peaks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id   TEXT NOT NULL,
    bucket       TEXT NOT NULL,
    resets_at    INTEGER NOT NULL,
    window_start INTEGER NOT NULL,
    peak_pct     REAL NOT NULL,
    peak_at      INTEGER NOT NULL,
    FOREIGN KEY (account_id) REFERENCES accounts(id)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_window_peaks_identity
    ON window_peaks(account_id, bucket, resets_at);
