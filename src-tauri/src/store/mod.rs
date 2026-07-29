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
    /// schema_version=7 so that migrate() skips steps meant for older upgrades.
    fn create_fresh_db(db_path: &Path) -> Result<Connection> {
        let conn = Connection::open(db_path).context("open sqlite")?;
        conn.execute_batch(include_str!("schema.sql")).context("apply schema")?;
        conn.execute(
            "INSERT OR REPLACE INTO schema_version (version) VALUES (?1)",
            [7_i64],
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

        conn.execute(
            "INSERT OR REPLACE INTO schema_version (version) VALUES (?1)",
            [7_i64],
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

    #[test]
    fn fresh_database_is_stamped_at_version_7() {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        let version: i64 = db
            .conn()
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 7, "create_fresh_db and migrate() must both stamp 7");
    }
}
