use anyhow::{Context, Result};
use fs2::FileExt;
use rusqlite::Connection;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Db {
    conn: Mutex<Connection>,
    _lock: Option<File>, // None in --tick mode; Some(file) in GUI mode
    /// True when the DB was corrupt on startup and had to be recreated.
    pub recovered: bool,
}

impl Db {
    /// GUI mode. Holds the exclusive file lock for process lifetime —
    /// prevents two GUI instances from racing on the DB.
    ///
    /// Returns `Ok(db)` in all non-fatal cases:
    ///   - clean open: `db.recovered == false`
    ///   - corruption detected + file renamed + DB recreated: `db.recovered == true`
    ///
    /// Returns `Err` only if the directory or lockfile cannot be created, or if
    /// another instance holds the process lock.
    pub fn open(dir: &Path) -> Result<Self> {
        Self::open_inner(dir, /*lock = */ true)
    }

    /// Headless `--tick` mode. Opens without the file lock — relies on
    /// SQLite WAL (schema.sql sets `journal_mode = WAL`) and the
    /// transactional claim in `scheduler::claim::try_claim` for cross-process
    /// correctness. Used by `claude-switchboard --tick` so the dispatcher
    /// can run alongside a running GUI without lock contention.
    pub fn open_for_tick(dir: &Path) -> Result<Self> {
        Self::open_inner(dir, /*lock = */ false)
    }

    fn open_inner(dir: &Path, lock: bool) -> Result<Self> {
        std::fs::create_dir_all(dir).context("create db dir")?;

        let lock_file = if lock {
            let lock_path = dir.join(crate::branding::DB_LOCKFILE_NAME);
            let lf = File::create(&lock_path).context("create lockfile")?;
            lf.try_lock_exclusive()
                .context("another instance holds the DB lock")?;
            Some(lf)
        } else {
            None
        };

        let db_path = dir.join("data.db");
        let (conn, recovered) = Self::open_or_recover(&db_path)?;

        let mut db = Db { conn: Mutex::new(conn), _lock: lock_file, recovered };
        db.migrate()?;
        Ok(db)
    }

    /// Try to open `db_path` and verify its integrity. On failure (open error
    /// or `PRAGMA integrity_check` ≠ "ok"), rename the corrupt file and create
    /// a fresh DB in its place. Returns `(connection, was_recovered)`.
    fn open_or_recover(db_path: &Path) -> Result<(Connection, bool)> {
        // No file yet — fresh install. Create and return directly.
        if !db_path.exists() {
            let conn = Self::create_fresh_db(db_path).context("create fresh sqlite")?;
            return Ok((conn, false));
        }

        // Existing file: open once and probe integrity — avoid opening twice.
        if let Ok(conn) = Connection::open(db_path) {
            let health: rusqlite::Result<String> =
                conn.query_row("PRAGMA integrity_check", [], |r| r.get(0));
            if matches!(health, Ok(ref s) if s == "ok") {
                // Healthy existing DB: apply schema (IF NOT EXISTS — safe no-op
                // on v2 DBs; adds missing tables on v1 DBs) and let migrate()
                // handle version advancement.
                conn.execute_batch(include_str!("schema.sql")).context("apply schema")?;
                return Ok((conn, false));
            }
        }

        // File exists but is corrupt — rename it and recreate.
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let backup = db_path.with_file_name(format!(
            "{}.corrupt-{ts}",
            db_path.file_name().and_then(|n| n.to_str()).unwrap_or("data.db")
        ));
        tracing::warn!(
            "corrupt DB detected — renaming {:?} to {:?} and recreating",
            db_path,
            backup,
        );
        let _ = std::fs::rename(db_path, &backup);
        let conn = Self::create_fresh_db(db_path).context("create fresh sqlite after recovery")?;
        Ok((conn, true))
    }

    /// Create a brand-new SQLite database with the current schema and stamp
    /// schema_version=14 so that migrate() skips steps meant for older upgrades.
    fn create_fresh_db(db_path: &Path) -> Result<Connection> {
        let conn = Connection::open(db_path).context("open sqlite")?;
        conn.execute_batch(include_str!("schema.sql")).context("apply schema")?;
        conn.execute(
            "INSERT OR REPLACE INTO schema_version (version) VALUES (?1)",
            [14_i64],
        )
        .context("stamp schema version")?;
        Ok(conn)
    }

    /// Brings the DB up to the current schema version. Each block is
    /// idempotent (guarded by the schema_version row) so it's safe to run
    /// on fresh DBs too.
    fn migrate(&mut self) -> Result<()> {
        let conn = self.conn.get_mut().unwrap();
        let current: i64 = conn
            .query_row("SELECT COALESCE(MAX(version), 0) FROM schema_version", [], |r| r.get(0))
            .unwrap_or(0);

        if current < 2 {
            tracing::info!("migrating session_events schema v1 -> v2 (event_id dedup)");
            conn.execute_batch(include_str!("migrations/0002_event_id_dedup.sql"))
                .context("apply migration 0002")?;
        }

        if current < 3 {
            tracing::info!("migrating notification_state v2 -> v3 (drop placeholder account_ids)");
            conn.execute_batch(include_str!(
                "migrations/0003_truncate_notification_placeholders.sql"
            ))
            .context("apply migration 0003")?;
        }

        if current < 4 {
            tracing::info!("migrating settings v3 -> v4 (insert migration_completed flag)");
            conn.execute_batch(include_str!("migrations/0004_migration_state.sql"))
                .context("apply migration 0004")?;
        }

        if current < 5 {
            tracing::info!("migrating accounts v4 -> v5 (warmup columns + consent setting)");
            conn.execute_batch(include_str!("migrations/0005_warmup.sql"))
                .context("apply migration 0005")?;
        }

        if current < 6 {
            tracing::info!("migrating session_events v5 -> v6 (re-ingest for relay message.id dedup)");
            conn.execute_batch(include_str!(
                "migrations/0006_reingest_for_message_id_dedup.sql"
            ))
            .context("apply migration 0006")?;
        }

        if current < 7 {
            tracing::info!("migrating v6 -> v7 (providers + provider_default tables)");
            conn.execute_batch(include_str!("migrations/0007_providers.sql"))
                .context("apply migration 0007")?;
        }

        if current < 8 {
            tracing::info!("migrating v7 -> v8 (re-ingest to backfill subagent transcripts)");
            conn.execute_batch(include_str!("migrations/0008_reingest_subagents.sql"))
                .context("apply migration 0008")?;
        }

        if current < 9 {
            tracing::info!("migrating v8 -> v9 (session_compactions + re-ingest to backfill)");
            conn.execute_batch(include_str!("migrations/0009_session_compactions.sql"))
                .context("apply migration 0009")?;
        }

        if current < 10 {
            tracing::info!("migrating v9 -> v10 (window_peaks for limit-hit analytics)");
            conn.execute_batch(include_str!("migrations/0010_window_peaks.sql"))
                .context("apply migration 0010")?;
        }

        if current < 11 {
            tracing::info!("migrating v10 -> v11 (statusline_install for F7)");
            conn.execute_batch(include_str!("migrations/0011_statusline_install.sql"))
                .context("apply migration 0011")?;
        }

        if current < 12 {
            tracing::info!("migrating v11 -> v12 (account_intervals for session attribution)");
            conn.execute_batch(include_str!("migrations/0012_account_intervals.sql"))
                .context("apply migration 0012")?;
        }

        if current < 13 {
            tracing::info!("migrating v12 -> v13 (transcript_lines + file_snapshots archive tables)");
            conn.execute_batch(include_str!("migrations/0013_archive_tables.sql"))
                .context("apply migration 0013")?;
        }

        if current < 14 {
            tracing::info!("migrating v13 -> v14 (device_id for sync)");
            conn.execute_batch(include_str!("migrations/0014_sync_device_id.sql"))
                .context("apply migration 0014")?;

            // Backfill: pre-existing rows (device_id='' from the ALTER TABLE
            // default / the file_snapshots rebuild) need this install's own
            // device_id — otherwise the sync engine's "rows to push" query
            // (which filters by device_id) can never find them, and
            // archive_watcher's re-snapshot-on-every-launch behavior would
            // create a duplicate file_snapshots row per file (the new
            // UNIQUE constraint is device_id-inclusive and no longer
            // matches the old ''-tagged row). This migration block only
            // ever runs once per database (gated by schema_version), so
            // INSERT OR IGNORE followed by a plain read is safe — it
            // guarantees a 'sync_device_id' settings row exists afterward
            // (ours, since nothing could have seeded it before this
            // feature existed) without needing OptionalExtension.
            let device_id = uuid::Uuid::new_v4().to_string();
            conn.execute(
                "INSERT OR IGNORE INTO settings (key, value) VALUES ('sync_device_id', ?1)",
                rusqlite::params![device_id],
            )?;
            let device_id: String = conn.query_row(
                "SELECT value FROM settings WHERE key = 'sync_device_id'",
                [],
                |r| r.get(0),
            )?;
            conn.execute(
                "UPDATE transcript_lines SET device_id = ?1 WHERE device_id = ''",
                rusqlite::params![device_id],
            )
            .context("backfill transcript_lines device_id")?;
            conn.execute(
                "UPDATE file_snapshots SET device_id = ?1 WHERE device_id = ''",
                rusqlite::params![device_id],
            )
            .context("backfill file_snapshots device_id")?;
        }

        conn.execute(
            "INSERT OR REPLACE INTO schema_version (version) VALUES (?1)",
            [14_i64],
        )?;
        Ok(())
    }

    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub mod queries;
pub use queries::*;

pub fn default_dir() -> PathBuf {
    use crate::branding::{
        PROJECT_DIRS_APP, PROJECT_DIRS_ORG, PROJECT_DIRS_QUALIFIER,
    };
    directories::ProjectDirs::from(
        PROJECT_DIRS_QUALIFIER,
        PROJECT_DIRS_ORG,
        PROJECT_DIRS_APP,
    )
    .map(|p| p.data_local_dir().to_path_buf())
    .unwrap_or_else(|| PathBuf::from(".claude-monitor"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn opens_fresh_db_and_applies_schema() {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).expect("open db");
        assert!(!db.recovered, "fresh open should not set recovered");
        let conn = db.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(count >= 6, "expected >=6 tables, got {count}");
    }

    #[test]
    fn rejects_second_instance() {
        let dir = tempdir().unwrap();
        let _first = Db::open(dir.path()).expect("first open");
        let second = Db::open(dir.path());
        assert!(second.is_err(), "second open should fail");
    }

    /// Write a deliberately-truncated (non-SQLite) file as `data.db`, then call
    /// `Db::open`.  The recovery path must:
    ///   1. Rename the corrupt file to `data.db.corrupt-<timestamp>`
    ///   2. Create a fresh, schema-applied DB at `data.db`
    ///   3. Set `db.recovered = true`
    #[test]
    fn recovers_from_corrupt_db() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("data.db");

        // Write garbage — not a valid SQLite file.
        let mut f = std::fs::File::create(&db_path).unwrap();
        f.write_all(b"this is not a sqlite database\x00\x01\x02").unwrap();
        drop(f);

        let db = Db::open(dir.path()).expect("open should succeed via recovery");
        assert!(db.recovered, "recovered flag must be set");

        // The new DB must have the schema applied.
        let conn = db.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(count >= 6, "recovered DB should have >=6 tables, got {count}");

        // The corrupt file must have been renamed (a .corrupt-<ts> sibling exists).
        let corrupt_files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .contains(".corrupt-")
            })
            .collect();
        assert!(
            !corrupt_files.is_empty(),
            "corrupt file should be renamed to *.corrupt-<timestamp>"
        );

        // The fresh DB file must exist at the original path.
        assert!(db_path.exists(), "fresh data.db must exist after recovery");
    }

    #[test]
    fn default_dir_uses_branding_constants() {
        let path = default_dir();
        let path_str = path.to_string_lossy();
        // macOS: ~/Library/Application Support/com.claude-switchboard.ClaudeSwitchboard
        // Windows: %LOCALAPPDATA%\claude-switchboard\ClaudeSwitchboard\data
        // Linux: ~/.local/share/claudeswitchboard (directories crate lowercases
        // and strips hyphens for the XDG project name).
        assert!(
            path_str.contains("claude-switchboard")
                || path_str.contains("ClaudeSwitchboard")
                || path_str.contains("claudeswitchboard"),
            "default_dir should reference branding constants, got: {path_str}",
        );
        assert!(
            !path_str.contains("claude-limits"),
            "default_dir should NOT reference legacy claude-limits, got: {path_str}",
        );
    }

    #[test]
    fn lockfile_name_comes_from_branding() {
        // The lockfile is created in Db::open(); we verify the constant routes
        // through correctly by spot-checking the branding module value.
        assert_eq!(crate::branding::DB_LOCKFILE_NAME, "claude-switchboard.lock");
    }

    #[test]
    fn migration_0004_inserts_migration_completed_setting() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path()).expect("open");
        let conn = db.conn();
        let value: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'migration_completed'",
                [],
                |r| r.get(0),
            )
            .expect("migration_completed row should exist");
        assert_eq!(value, "0", "default value is '0' (false)");
    }

    /// Verify that `0004_migration_state.sql` is actually executed and inserts
    /// the `migration_completed` row.
    ///
    /// The existing test above covers the fresh-DB path (schema.sql seed), but
    /// never invokes the migration file itself.  Re-opening an existing DB is
    /// insufficient to isolate the migration because `open_or_recover` re-runs
    /// schema.sql on every existing-file open (which seeds the row via
    /// `INSERT OR IGNORE` before `migrate()` runs).
    ///
    /// Strategy — direct `execute_batch` against a minimal in-memory-style DB:
    ///   1. Open a real DB so the `settings` table exists.
    ///   2. Delete the seed row so the table looks like a pre-migration state.
    ///   3. Execute `0004_migration_state.sql` via `execute_batch` directly.
    ///   4. Assert the row was inserted with value `'0'`.
    ///   5. Execute again — confirm idempotency (ON CONFLICT DO NOTHING).
    #[test]
    fn migration_0004_inserts_row_when_upgrading_from_v3() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path()).expect("open fresh db");
        let conn = db.conn();

        // Step 2: remove the schema.sql seed row to simulate a pre-0004 DB.
        conn.execute("DELETE FROM settings WHERE key = 'migration_completed'", [])
            .expect("remove seed row");
        let absent: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key = 'migration_completed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(absent, 0, "seed row must be absent before running the migration");

        // Step 3: run the migration SQL directly — this is the code under test.
        conn.execute_batch(include_str!("migrations/0004_migration_state.sql"))
            .expect("0004_migration_state.sql should execute without error");

        // Step 4: row must now exist with value '0'.
        let value: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'migration_completed'",
                [],
                |r| r.get(0),
            )
            .expect("migration_completed row must exist after 0004 migration SQL");
        assert_eq!(value, "0", "migration_completed default value must be '0'");

        // Step 5: re-run is idempotent (ON CONFLICT DO NOTHING).
        conn.execute_batch(include_str!("migrations/0004_migration_state.sql"))
            .expect("re-running 0004 should be a no-op, not an error");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key = 'migration_completed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "idempotent re-run must not duplicate the row");
    }

    /// 0006 clears the inflated relay-model rows and resets JSONL cursors so
    /// the walker re-ingests every file from byte 0 with the corrected
    /// message.id-based dedup. Verify it empties both tables regardless of
    /// prior contents.
    #[test]
    fn migration_0006_clears_events_and_resets_cursors() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path()).expect("open fresh db");
        let conn = db.conn();

        // Seed inflated history: a few session_events + a cursor, mimicking
        // the pre-fix state where relay responses were counted per-line.
        conn.execute(
            "INSERT INTO session_events
             (ts, project, model, input_tokens, output_tokens, cache_read_tokens,
              cache_creation_5m_tokens, cache_creation_1h_tokens, cost_usd,
              source_file, source_line, event_id)
             VALUES (1,'p','glm-5.2',1,1,1,0,0,0.1,'f.jsonl',0,'f.jsonl:0')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO jsonl_cursors (file_path, last_mtime_ns, byte_offset)
             VALUES ('f.jsonl', 100, 999)",
            [],
        )
        .unwrap();
        let events_before: i64 =
            conn.query_row("SELECT COUNT(*) FROM session_events", [], |r| r.get(0))
                .unwrap();
        let cursors_before: i64 =
            conn.query_row("SELECT COUNT(*) FROM jsonl_cursors", [], |r| r.get(0))
                .unwrap();
        assert_eq!(events_before, 1);
        assert_eq!(cursors_before, 1);

        // Run the migration SQL directly — this is the code under test.
        conn.execute_batch(include_str!("migrations/0006_reingest_for_message_id_dedup.sql"))
            .expect("0006 SQL should execute without error");

        let events_after: i64 =
            conn.query_row("SELECT COUNT(*) FROM session_events", [], |r| r.get(0))
                .unwrap();
        let cursors_after: i64 =
            conn.query_row("SELECT COUNT(*) FROM jsonl_cursors", [], |r| r.get(0))
                .unwrap();
        assert_eq!(events_after, 0, "0006 must clear all session_events rows");
        assert_eq!(
            cursors_after, 0,
            "0006 must clear all jsonl_cursors so the walker re-reads every file"
        );
    }

    #[test]
    fn migration_0005_adds_warmup_columns_and_consent_setting() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path()).expect("open");
        let conn = db.conn();

        conn.execute(
            "INSERT INTO accounts (id, email, last_seen_at) VALUES (?1, ?2, ?3)",
            rusqlite::params!["acct-1", "test@example.com", 0i64],
        )
        .unwrap();

        let warmup_enabled: i64 = conn
            .query_row(
                "SELECT warmup_enabled FROM accounts WHERE id = 'acct-1'",
                [],
                |r| r.get(0),
            )
            .expect("warmup_enabled column exists with default");
        assert_eq!(warmup_enabled, 0);

        let schedule: String = conn
            .query_row(
                "SELECT schedule FROM accounts WHERE id = 'acct-1'",
                [],
                |r| r.get(0),
            )
            .expect("schedule column exists with default");
        assert_eq!(schedule, r#"{"type":"Off"}"#);

        let last: Option<i64> = conn
            .query_row(
                "SELECT last_warmup_at FROM accounts WHERE id = 'acct-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(last, None);

        let consent: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'warmup_consent_granted'",
                [],
                |r| r.get(0),
            )
            .expect("warmup_consent_granted setting row exists");
        assert_eq!(consent, "0");
    }

    #[test]
    fn open_for_tick_does_not_take_exclusive_lock() {
        let dir = tempfile::tempdir().unwrap();
        // Acquire the exclusive lock as if a GUI is running.
        let _gui_db = Db::open(dir.path()).expect("gui open");

        // A second `Db::open(...)` would fail because the lockfile is held.
        let conflict = Db::open(dir.path());
        assert!(
            conflict.is_err(),
            "Db::open while another holds the lock should fail",
        );

        // But Db::open_for_tick should succeed — it doesn't take the file lock.
        let tick_db = Db::open_for_tick(dir.path()).expect("tick open should succeed");
        assert!(!tick_db.recovered);

        // Both connections can read & write (SQLite WAL handles concurrency).
        let conn = tick_db.conn();
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(n > 0);
    }

    #[test]
    fn migration_0005_inserts_columns_when_upgrading() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // Build the v4 schema shape (accounts WITHOUT the new columns).
        conn.execute_batch(
            "CREATE TABLE accounts ( \
               id TEXT PRIMARY KEY, \
               email TEXT NOT NULL, \
               display_name TEXT, \
               last_seen_at INTEGER NOT NULL \
             ); \
             CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL); \
             INSERT INTO settings (key, value) VALUES ('migration_completed', '1');",
        )
        .unwrap();

        // Apply only 0005 directly.
        conn.execute_batch(include_str!("migrations/0005_warmup.sql")).unwrap();

        // Now insert an account and verify defaults.
        conn.execute(
            "INSERT INTO accounts (id, email, last_seen_at) VALUES ('a', 'x@y.z', 0)",
            [],
        )
        .unwrap();
        let warmup: i64 = conn
            .query_row("SELECT warmup_enabled FROM accounts WHERE id='a'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(warmup, 0);
        let consent: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key='warmup_consent_granted'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(consent, "0");
    }

    /// Mirrors `migration_0004_inserts_row_when_upgrading_from_v3`: build a
    /// pre-migration database by hand, run the migration SQL directly, and
    /// assert its effect. `Db::open` cannot be used here because it applies
    /// `schema.sql`, which already contains the tables under test.
    #[test]
    fn migration_0007_creates_provider_tables_on_upgrade() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("v6.db");
        let conn = Connection::open(&db_path).unwrap();

        // Simulate a v6 database: full schema, then drop what v7 introduces.
        conn.execute_batch(include_str!("schema.sql")).unwrap();
        conn.execute_batch("DROP TABLE providers; DROP TABLE provider_default;")
            .unwrap();
        conn.execute("INSERT OR REPLACE INTO schema_version (version) VALUES (6)", [])
            .unwrap();

        let before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('providers','provider_default')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, 0, "precondition: the v6 database has neither table");

        conn.execute_batch(include_str!("migrations/0007_providers.sql"))
            .unwrap();

        let after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('providers','provider_default')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, 2, "migration 0007 must create both provider tables");
    }

    /// 0008 must clear cursors (forcing a re-read from byte 0) without
    /// deleting events: `event_id` is stable and UNIQUE, so re-reading is
    /// idempotent for rows already stored and only adds the missing ones.
    /// Deleting events here would throw away history the transcripts on disk
    /// can no longer supply once Claude Code prunes them.
    #[test]
    fn migration_0008_clears_cursors_but_keeps_events() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("v7.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("schema.sql")).unwrap();

        conn.execute(
            "INSERT INTO jsonl_cursors (file_path, last_mtime_ns, byte_offset)
             VALUES ('/a/b.jsonl', 1, 4096)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_events
               (ts, project, model, input_tokens, output_tokens,
                cache_read_tokens, cost_usd, source_file, source_line, event_id)
             VALUES (0, 'p', 'm', 1, 1, 0, 0.0, '/a/b.jsonl', 1, 'evt-1')",
            [],
        )
        .unwrap();

        conn.execute_batch(include_str!("migrations/0008_reingest_subagents.sql"))
            .unwrap();

        let cursors: i64 = conn
            .query_row("SELECT COUNT(*) FROM jsonl_cursors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cursors, 0, "cursors must be cleared to force a re-read");

        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 1, "events must survive — re-ingest is idempotent");
    }

    #[test]
    fn fresh_database_is_stamped_at_version_11() {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        let version: i64 = db
            .conn()
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 14, "create_fresh_db and migrate() must both stamp 14");
    }

    /// 0009 adds the compactions table and, like 0008, clears cursors so the
    /// walker re-reads every transcript and backfills it. Events must survive:
    /// deleting them would throw away history the transcripts on disk can no
    /// longer supply once Claude Code prunes them.
    #[test]
    fn migration_0009_adds_compactions_table_and_reingests() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("v8.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("schema.sql")).unwrap();
        conn.execute_batch("DROP TABLE session_compactions;").unwrap();
        conn.execute(
            "INSERT INTO jsonl_cursors (file_path, last_mtime_ns, byte_offset)
             VALUES ('/a/b.jsonl', 1, 4096)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_events
               (ts, project, model, input_tokens, output_tokens,
                cache_read_tokens, cost_usd, source_file, source_line, event_id)
             VALUES (0, 'p', 'm', 1, 1, 0, 0.0, '/a/b.jsonl', 1, 'evt-1')",
            [],
        )
        .unwrap();

        conn.execute_batch(include_str!("migrations/0009_session_compactions.sql"))
            .unwrap();

        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='session_compactions'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 1, "migration 0009 must create session_compactions");

        let cursors: i64 = conn
            .query_row("SELECT COUNT(*) FROM jsonl_cursors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cursors, 0, "cursors cleared so the new table is backfilled");

        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 1, "events must survive — re-ingest is idempotent");
    }

    /// The compaction dedup key is the record's own uuid, so re-reading a
    /// transcript (which 0009 deliberately forces) must not duplicate rows.
    #[test]
    fn compactions_dedupe_on_uuid() {
        use crate::store::StoredCompaction;
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        let c = StoredCompaction {
            ts: chrono::Utc::now(),
            source_file: "p/s.jsonl".into(),
            trigger: "manual".into(),
            pre_tokens: 495_927,
            post_tokens: 16_608,
            uuid: "u-1".into(),
        };
        db.ingest_atomic("p/s.jsonl", &[], std::slice::from_ref(&c), 1, 10).unwrap();
        db.ingest_atomic("p/s.jsonl", &[], std::slice::from_ref(&c), 2, 20).unwrap();
        let n: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM session_compactions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "same uuid must not be stored twice");

        let from = chrono::Utc::now() - chrono::Duration::days(1);
        let got = db.compactions_between(from, chrono::Utc::now() + chrono::Duration::days(1)).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].trigger, "manual");
        assert_eq!(got[0].pre_tokens, 495_927);
    }

    #[test]
    fn migration_0010_adds_window_peaks_table() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("v9.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("schema.sql")).unwrap();
        conn.execute_batch("DROP TABLE window_peaks;").unwrap();

        conn.execute_batch(include_str!("migrations/0010_window_peaks.sql"))
            .unwrap();

        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='window_peaks'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 1, "migration 0010 must create window_peaks");

        // The unique index is the window-rollover detector — prove it exists and
        // actually enforces the identity key, not just that the table exists.
        conn.execute(
            "INSERT INTO accounts (id, email, last_seen_at) VALUES ('a1', 'a@x.com', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO window_peaks (account_id, bucket, resets_at, window_start, peak_pct, peak_at)
             VALUES ('a1', 'five_hour', 100, 90, 50.0, 95)",
            [],
        )
        .unwrap();
        let dup = conn.execute(
            "INSERT INTO window_peaks (account_id, bucket, resets_at, window_start, peak_pct, peak_at)
             VALUES ('a1', 'five_hour', 100, 90, 60.0, 96)",
            [],
        );
        assert!(dup.is_err(), "duplicate (account_id, bucket, resets_at) must be rejected");
    }

    #[test]
    fn migrates_to_v12_with_account_intervals_table() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path()).expect("open");
        let conn = db.conn();

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 14);

        conn.execute(
            "INSERT INTO accounts (id, email, last_seen_at) VALUES ('a1', 'a@x.com', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO account_intervals (account_uuid, started_at, ended_at) VALUES ('a1', 100, NULL)",
            [],
        )
        .expect("account_intervals table exists and accepts a row");

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM account_intervals", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn migration_0013_adds_archive_tables() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("v12.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("schema.sql")).unwrap();
        conn.execute_batch("DROP TABLE transcript_lines; DROP TABLE file_snapshots;")
            .unwrap();

        conn.execute_batch(include_str!("migrations/0013_archive_tables.sql")).unwrap();

        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('transcript_lines','file_snapshots')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 2, "migration 0013 must create both archive tables");

        conn.execute(
            "INSERT INTO transcript_lines (project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)
             VALUES ('p', 's', 'p/s.jsonl', 0, '{}', 0)",
            [],
        )
        .unwrap();
        let dup = conn.execute(
            "INSERT INTO transcript_lines (project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)
             VALUES ('p', 's', 'p/s.jsonl', 0, '{\"different\":true}', 1)",
            [],
        );
        assert!(dup.is_err(), "duplicate (jsonl_path, line_no) must be rejected");
    }

    /// The headline archive bug: every pre-existing jsonl_cursors row was
    /// written by the pre-archive walker, which never called
    /// insert_transcript_lines. ingest_file's unchanged-mtime/unchanged-length
    /// short-circuit means those files would otherwise never be re-read, so
    /// transcript_lines would silently stay empty for a user's entire
    /// pre-existing history. Migration 0013 must clear jsonl_cursors to force
    /// a full re-read on the next backfill — same pattern as 0008/0009.
    #[test]
    fn migration_0013_clears_cursors_to_force_archive_backfill() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("v12.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("schema.sql")).unwrap();
        conn.execute_batch("DROP TABLE transcript_lines; DROP TABLE file_snapshots;")
            .unwrap();

        conn.execute(
            "INSERT INTO jsonl_cursors (file_path, last_mtime_ns, byte_offset)
             VALUES ('/a/b.jsonl', 1, 4096)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_events
               (ts, project, model, input_tokens, output_tokens,
                cache_read_tokens, cost_usd, source_file, source_line, event_id)
             VALUES (0, 'p', 'm', 1, 1, 0, 0.0, '/a/b.jsonl', 1, 'evt-1')",
            [],
        )
        .unwrap();

        conn.execute_batch(include_str!("migrations/0013_archive_tables.sql")).unwrap();

        let cursors: i64 = conn
            .query_row("SELECT COUNT(*) FROM jsonl_cursors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cursors, 0, "cursors must be cleared to force transcript archiving on re-read");

        let events: i64 = conn
            .query_row("SELECT COUNT(*) FROM session_events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 1, "events must survive — re-ingest is idempotent");
    }

    #[test]
    fn migration_0014_adds_device_id_and_rebuilds_file_snapshots() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("v13.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("schema.sql")).unwrap();
        // Simulate a pre-v14 shape: drop device_id from both tables by
        // rebuilding them without it, matching what a real v13 DB looked like.
        conn.execute_batch(
            "DROP TABLE transcript_lines;
             CREATE TABLE transcript_lines (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 project_slug TEXT NOT NULL, session_id TEXT NOT NULL,
                 jsonl_path TEXT NOT NULL, line_no INTEGER NOT NULL,
                 raw_line TEXT NOT NULL, ingested_at INTEGER NOT NULL,
                 UNIQUE (jsonl_path, line_no)
             );
             DROP TABLE file_snapshots;
             CREATE TABLE file_snapshots (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 source_path TEXT NOT NULL, kind TEXT NOT NULL,
                 content TEXT NOT NULL, content_hash TEXT NOT NULL,
                 captured_at INTEGER NOT NULL,
                 UNIQUE (source_path, content_hash)
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_snapshots (source_path, kind, content, content_hash, captured_at)
             VALUES ('/x', 'misc', 'hi', 'h1', 0)",
            [],
        )
        .unwrap();

        conn.execute_batch(include_str!("migrations/0014_sync_device_id.sql")).unwrap();

        let device_id: String = conn
            .query_row("SELECT device_id FROM file_snapshots WHERE source_path = '/x'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(device_id, "", "pre-existing rows default to empty device_id until backfilled");

        // Prove the new UNIQUE constraint actually includes device_id: two
        // rows with the same (source_path, content_hash) but different
        // device_id must both be insertable.
        conn.execute(
            "INSERT INTO file_snapshots (device_id, source_path, kind, content, content_hash, captured_at)
             VALUES ('device-a', '/y', 'misc', 'hi', 'h2', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_snapshots (device_id, source_path, kind, content, content_hash, captured_at)
             VALUES ('device-b', '/y', 'misc', 'hi', 'h2', 0)",
            [],
        )
        .expect("same (source_path, content_hash) from a DIFFERENT device_id must not collide");
    }

    /// Regression test for the critical migration bug: rows archived before
    /// this install ever ran the device_id migration must be backfilled
    /// with this install's own device_id, not left at the '' default
    /// forever. Unlike `migration_0014_adds_device_id_and_rebuilds_file_snapshots`
    /// above (which calls `execute_batch` on the raw migration SQL directly),
    /// this test must go through the REAL `Db::open()` -> `migrate()` path,
    /// since the backfill logic lives in Rust code inside `migrate()`, not
    /// in the migration's `.sql` file.
    #[test]
    fn migration_0014_backfills_device_id_on_preexisting_rows_via_real_open() {
        let dir = tempdir().unwrap();
        // `Db::open` always opens `<dir>/data.db` — build the pre-upgrade
        // (v13-shaped) database at that exact path so the real open path
        // picks it up as an existing file to migrate, not a fresh install.
        let db_path = dir.path().join("data.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(include_str!("schema.sql")).unwrap();
            // Simulate a pre-v14 shape: drop device_id from both tables by
            // rebuilding them without it, matching what a real v13 DB looked like.
            conn.execute_batch(
                "DROP TABLE transcript_lines;
                 CREATE TABLE transcript_lines (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     project_slug TEXT NOT NULL, session_id TEXT NOT NULL,
                     jsonl_path TEXT NOT NULL, line_no INTEGER NOT NULL,
                     raw_line TEXT NOT NULL, ingested_at INTEGER NOT NULL,
                     UNIQUE (jsonl_path, line_no)
                 );
                 DROP TABLE file_snapshots;
                 CREATE TABLE file_snapshots (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     source_path TEXT NOT NULL, kind TEXT NOT NULL,
                     content TEXT NOT NULL, content_hash TEXT NOT NULL,
                     captured_at INTEGER NOT NULL,
                     UNIQUE (source_path, content_hash)
                 );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO transcript_lines (project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)
                 VALUES ('p', 's', 'p/s.jsonl', 0, '{}', 0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO file_snapshots (source_path, kind, content, content_hash, captured_at)
                 VALUES ('/x', 'misc', 'hi', 'h1', 0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO schema_version (version) VALUES (13)",
                [],
            )
            .unwrap();
            // conn dropped at end of this block, releasing the sqlite handle
            // before Db::open (which takes its own separate lockfile) below.
        }

        // The real open/migrate path — this is what actually exercises the
        // backfill code inside migrate(), not just the raw migration SQL.
        let db = Db::open(dir.path()).expect("open should upgrade v13 -> v14 with backfill");
        let my_id = db.device_id().unwrap();
        assert!(!my_id.is_empty());
        assert!(uuid::Uuid::parse_str(&my_id).is_ok(), "device_id must be a real UUID, not the '' sentinel");

        let conn = db.conn();
        let line_device_id: String = conn
            .query_row(
                "SELECT device_id FROM transcript_lines WHERE jsonl_path = 'p/s.jsonl'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            line_device_id, my_id,
            "pre-existing transcript_lines row must be backfilled with this install's device_id"
        );

        let snap_device_id: String = conn
            .query_row(
                "SELECT device_id FROM file_snapshots WHERE source_path = '/x'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            snap_device_id, my_id,
            "pre-existing file_snapshots row must be backfilled with this install's device_id"
        );
    }
}
