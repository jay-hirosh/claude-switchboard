# Durable Claude Archive Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist raw Claude Code session transcripts and a small set of config/memory files into new `store.db` tables so this history survives `~/.claude`, per-repo `.remember/`, and/or the repos themselves being deleted.

**Architecture:** Two new tables (`transcript_lines`, `file_snapshots`) added to the existing `store.db` via a new migration, never pruned. `jsonl_parser::walker::ingest_file` is extended to archive every raw transcript line alongside its existing analytics ingestion. A new `archive_watcher` module discovers every repo ever touched (by reusing the existing `sessions::recap` cwd resolution) and watches a fixed `~/.claude` surface plus each repo's `CLAUDE.md`/`.remember/*.md` for changes, snapshotting on a content-hash basis.

**Tech Stack:** Rust, `rusqlite` (bundled SQLite), `notify` + `notify-debouncer-full` (already a dependency), `sha2` (already a dependency), `tauri::async_runtime`.

**Spec:** `docs/superpowers/specs/2026-08-21-durable-claude-archive-design.md` — read it alongside this plan; this plan does not restate its rationale, only the concrete steps.

## Global Constraints

- Reuse the existing `store.db` file, `Db` handle, migration chain, and file lock — no second database, no second connection.
- `transcript_lines` and `file_snapshots` are never pruned by any existing or new code path. `api_snapshots`/`session_events` keep their current 30/90-day pruning unchanged.
- Archive ingestion failures (transcript lines or file snapshots) are logged and swallowed, never propagated in a way that blocks `session_events`/`api_snapshots` ingestion, and vice versa.
- Any file over 5MB, or that fails UTF-8 validation, is skipped with a logged warning — never stored, never crashes ingestion.
- No recursive scan of `~/.claude` — only the fixed file list in the spec plus each discovered repo's `CLAUDE.md`/`.remember/*.md`. This is what keeps `security/`, `session-env/`, `shell-snapshots/`, `file-history/`, `backups/`, `ide/`, and the app's own `settings.json.switchboard-*` backups out of scope, by construction rather than by an exclusion list.
- No search/FTS, no encryption beyond what the app already relies on (disk encryption + file permissions) — storage only, this phase.
- Repo discovery reuses `sessions::recap::parse_session`'s `cwd` field — never de-slugify `~/.claude/projects/<slug>` directory names.

---

### Task 1: Schema migration — `transcript_lines` + `file_snapshots`

**Files:**
- Modify: `src-tauri/src/store/schema.sql`
- Create: `src-tauri/src/store/migrations/0013_archive_tables.sql`
- Modify: `src-tauri/src/store/mod.rs:104-199` (`create_fresh_db`, `migrate`), `src-tauri/src/store/mod.rs:655-663` and `:786-794` (existing version-stamp tests)

**Interfaces:**
- Produces: two new tables, `transcript_lines(id, project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)` with `UNIQUE(jsonl_path, line_no)`, and `file_snapshots(id, source_path, kind, content, content_hash, captured_at)` with `UNIQUE(source_path, content_hash)`. Schema version becomes 13.

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/src/store/mod.rs`, inside the existing `#[cfg(test)] mod tests { ... }` block (near the other `migration_00NN_*` tests, e.g. after `migrates_to_v12_with_account_intervals_table`):

```rust
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
```

This will fail to compile/run because `migrations/0013_archive_tables.sql` does not exist yet.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test migration_0013_adds_archive_tables`
Expected: FAIL — `include_str!` error, file not found.

- [ ] **Step 3: Add the tables to `schema.sql`**

Append to the end of `src-tauri/src/store/schema.sql`:

```sql

-- Durable local archive of ~/.claude + per-repo history — never pruned,
-- exists so deleting the source files loses nothing already ingested.
-- See docs/superpowers/specs/2026-08-21-durable-claude-archive-design.md.
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
```

- [ ] **Step 4: Create the migration file**

Create `src-tauri/src/store/migrations/0013_archive_tables.sql`:

```sql
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
```

- [ ] **Step 4b: Run test to verify it passes**

Run: `cd src-tauri && cargo test migration_0013_adds_archive_tables`
Expected: PASS

- [ ] **Step 5: Wire the migration into `migrate()` and bump the version stamps**

In `src-tauri/src/store/mod.rs`, in `migrate()` (after the existing `if current < 12 { ... }` block, currently ending around line 192), add:

```rust
        if current < 13 {
            tracing::info!("migrating v12 -> v13 (transcript_lines + file_snapshots archive tables)");
            conn.execute_batch(include_str!("migrations/0013_archive_tables.sql"))
                .context("apply migration 0013")?;
        }
```

Change the final stamp at the end of `migrate()` (currently `conn.execute("INSERT OR REPLACE INTO schema_version (version) VALUES (?1)", [12_i64],)?;`) to `[13_i64]`.

In `create_fresh_db`, change `conn.execute("INSERT OR REPLACE INTO schema_version (version) VALUES (?1)", [12_i64],)` to `[13_i64]`, and update its doc comment ("...stamp schema_version=12...") to say 13.

- [ ] **Step 6: Update the two existing version-stamp tests**

In `fresh_database_is_stamped_at_version_11` (the test name is already stale relative to its assertion — do not rename it, just update the value), change:

```rust
        assert_eq!(version, 12, "create_fresh_db and migrate() must both stamp 12");
```
to:
```rust
        assert_eq!(version, 13, "create_fresh_db and migrate() must both stamp 13");
```

In `migrates_to_v12_with_account_intervals_table`, change:
```rust
        assert_eq!(version, 12);
```
to:
```rust
        assert_eq!(version, 13);
```

- [ ] **Step 7: Run the full store test suite**

Run: `cd src-tauri && cargo test --lib store::`
Expected: PASS (all existing + new tests)

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/store/schema.sql src-tauri/src/store/migrations/0013_archive_tables.sql src-tauri/src/store/mod.rs
git commit -m "feat: add transcript_lines + file_snapshots archive tables (schema v13)"
```

---

### Task 2: `Db` methods for the archive tables

**Files:**
- Modify: `src-tauri/src/store/queries.rs` (add types near `StoredCompaction`, ~line 57; add methods to `impl Db` near `ingest_atomic`, ~line 674; add tests to the existing `#[cfg(test)] mod tests` block)

**Interfaces:**
- Consumes: `transcript_lines`/`file_snapshots` tables from Task 1.
- Produces: `StoredTranscriptLine { project_slug: String, session_id: String, jsonl_path: String, line_no: i64, raw_line: String }`, `StoredFileSnapshot { source_path: String, kind: String, content: String, content_hash: String }`, `Db::insert_transcript_lines(&self, lines: &[StoredTranscriptLine]) -> Result<usize>`, `Db::transcript_lines_for_path(&self, jsonl_path: &str) -> Result<Vec<StoredTranscriptLine>>`, `Db::insert_file_snapshot(&self, snap: &StoredFileSnapshot) -> Result<bool>`, `Db::file_snapshots_for_path(&self, source_path: &str) -> Result<Vec<StoredFileSnapshot>>` — all consumed by Task 3 (transcript) and Task 5 (file snapshot).

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/store/queries.rs` (near the other prune tests):

```rust
    #[test]
    fn insert_transcript_lines_is_idempotent_and_replaces_stale_content() {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        let line = StoredTranscriptLine {
            project_slug: "proj".into(),
            session_id: "sess".into(),
            jsonl_path: "proj/sess.jsonl".into(),
            line_no: 0,
            raw_line: r#"{"a":1}"#.into(),
        };
        let n1 = db.insert_transcript_lines(std::slice::from_ref(&line)).unwrap();
        assert_eq!(n1, 1);
        let n2 = db.insert_transcript_lines(std::slice::from_ref(&line)).unwrap();
        assert_eq!(n2, 1, "REPLACE still reports a row written, but count must not grow");

        let rows = db.transcript_lines_for_path("proj/sess.jsonl").unwrap();
        assert_eq!(rows.len(), 1, "same (jsonl_path, line_no) must not duplicate");

        let rewritten = StoredTranscriptLine { raw_line: r#"{"a":2}"#.into(), ..line };
        db.insert_transcript_lines(std::slice::from_ref(&rewritten)).unwrap();
        let rows = db.transcript_lines_for_path("proj/sess.jsonl").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].raw_line, r#"{"a":2}"#, "replace must win over the stale row");
    }

    #[test]
    fn insert_file_snapshot_dedupes_identical_content() {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        let snap = StoredFileSnapshot {
            source_path: "/home/.claude/settings.json".into(),
            kind: "settings".into(),
            content: "{\"a\":1}".into(),
            content_hash: "hash1".into(),
        };
        assert!(db.insert_file_snapshot(&snap).unwrap(), "first write is new");
        assert!(!db.insert_file_snapshot(&snap).unwrap(), "identical content is not re-stored");

        let changed = StoredFileSnapshot {
            content_hash: "hash2".into(),
            content: "{\"a\":2}".into(),
            ..snap.clone()
        };
        assert!(db.insert_file_snapshot(&changed).unwrap(), "a real content change gets its own row");

        let rows = db.file_snapshots_for_path("/home/.claude/settings.json").unwrap();
        assert_eq!(rows.len(), 2, "two distinct content versions must both be kept");
    }

    #[test]
    fn archive_tables_are_never_pruned() {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        db.insert_transcript_lines(&[StoredTranscriptLine {
            project_slug: "p".into(),
            session_id: "s".into(),
            jsonl_path: "p/s.jsonl".into(),
            line_no: 0,
            raw_line: "{}".into(),
        }])
        .unwrap();
        db.insert_file_snapshot(&StoredFileSnapshot {
            source_path: "/x".into(),
            kind: "misc".into(),
            content: "x".into(),
            content_hash: "h".into(),
        })
        .unwrap();

        let far_future = Utc::now() + chrono::Duration::days(3650);
        db.prune_events_older_than(far_future).unwrap();
        db.prune_snapshots(0).unwrap();

        assert_eq!(db.transcript_lines_for_path("p/s.jsonl").unwrap().len(), 1);
        assert_eq!(db.file_snapshots_for_path("/x").unwrap().len(), 1);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib insert_transcript_lines_is_idempotent insert_file_snapshot_dedupes archive_tables_are_never_pruned`
Expected: FAIL — `StoredTranscriptLine`/`StoredFileSnapshot`/the four methods don't exist yet.

- [ ] **Step 3: Add the types**

In `src-tauri/src/store/queries.rs`, immediately after the `StoredCompaction` struct definition, add:

```rust
/// One raw line ever observed in a transcript JSONL file, archived verbatim
/// (trimmed of surrounding whitespace — the same text session_events'
/// parser already works from) regardless of whether the line carried usage
/// data. `line_no` is the byte offset the line started at — the same value
/// session_events.source_line and jsonl_cursors track — not a sequential
/// count. Never pruned: this table's existence is what lets ~/.claude be
/// deleted without losing session history.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredTranscriptLine {
    pub project_slug: String,
    pub session_id: String,
    pub jsonl_path: String,
    pub line_no: i64,
    pub raw_line: String,
}

/// One content version of a small non-transcript file worth archiving
/// (settings.json, CLAUDE.md, a repo's .remember/*.md, ...). `content_hash`
/// dedups repeated writes of identical content; a genuine content change
/// gets its own row rather than overwriting the last one, so the history of
/// what changed and when survives too. Never pruned, same as
/// transcript_lines.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredFileSnapshot {
    pub source_path: String,
    pub kind: String,
    pub content: String,
    pub content_hash: String,
}
```

- [ ] **Step 4: Add the `Db` methods**

In `src-tauri/src/store/queries.rs`, immediately after the `ingest_atomic` method, add:

```rust
    /// Archives every raw transcript line passed in, keyed by (jsonl_path,
    /// line_no). `INSERT OR REPLACE` rather than `OR IGNORE`: on the rare
    /// truncation-then-rewrite case (see jsonl_parser::walker), a byte
    /// offset can be reused for genuinely different content, and the
    /// archive must reflect what's actually there now, not the first thing
    /// ever seen at that offset. For the ordinary append-only case this is
    /// indistinguishable from IGNORE — the values are identical, so REPLACE
    /// just rewrites the same row.
    pub fn insert_transcript_lines(&self, lines: &[StoredTranscriptLine]) -> Result<usize> {
        if lines.is_empty() {
            return Ok(0);
        }
        let now = Utc::now().timestamp();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let inserted = {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO transcript_lines
                 (project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            let mut n = 0;
            for l in lines {
                n += stmt.execute(params![
                    l.project_slug,
                    l.session_id,
                    l.jsonl_path,
                    l.line_no,
                    l.raw_line,
                    now,
                ])?;
            }
            n
        };
        tx.commit()?;
        Ok(inserted)
    }

    /// Ordered by line_no (== byte offset) — a transcript's archived lines
    /// back in original file order.
    pub fn transcript_lines_for_path(&self, jsonl_path: &str) -> Result<Vec<StoredTranscriptLine>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT project_slug, session_id, jsonl_path, line_no, raw_line
             FROM transcript_lines WHERE jsonl_path = ?1 ORDER BY line_no",
        )?;
        let rows = stmt.query_map(params![jsonl_path], |r| {
            Ok(StoredTranscriptLine {
                project_slug: r.get(0)?,
                session_id: r.get(1)?,
                jsonl_path: r.get(2)?,
                line_no: r.get(3)?,
                raw_line: r.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Inserts one file snapshot if its content_hash differs from every
    /// prior snapshot at the same path. Returns true if a new row was
    /// written, false if this exact content was already archived.
    pub fn insert_file_snapshot(&self, snap: &StoredFileSnapshot) -> Result<bool> {
        let now = Utc::now().timestamp();
        let changed = self.conn().execute(
            "INSERT OR IGNORE INTO file_snapshots
             (source_path, kind, content, content_hash, captured_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![snap.source_path, snap.kind, snap.content, snap.content_hash, now],
        )?;
        Ok(changed > 0)
    }

    /// Ordered oldest-first.
    pub fn file_snapshots_for_path(&self, source_path: &str) -> Result<Vec<StoredFileSnapshot>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT source_path, kind, content, content_hash
             FROM file_snapshots WHERE source_path = ?1 ORDER BY captured_at, id",
        )?;
        let rows = stmt.query_map(params![source_path], |r| {
            Ok(StoredFileSnapshot {
                source_path: r.get(0)?,
                kind: r.get(1)?,
                content: r.get(2)?,
                content_hash: r.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib insert_transcript_lines_is_idempotent insert_file_snapshot_dedupes archive_tables_are_never_pruned`
Expected: PASS

- [ ] **Step 6: Run the full store test suite**

Run: `cd src-tauri && cargo test --lib store::`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/store/queries.rs
git commit -m "feat: add Db methods for transcript_lines and file_snapshots"
```

---

### Task 3: Archive raw transcript lines from the JSONL walker

**Files:**
- Modify: `src-tauri/src/jsonl_parser/walker.rs:1` (imports), `:84-208` (`ingest_file`)
- Modify: `src-tauri/tests/jsonl_walker.rs` (add tests)

**Interfaces:**
- Consumes: `Db::insert_transcript_lines`, `StoredTranscriptLine` (Task 2).
- Produces: no new public interface — `ingest_file`'s signature is unchanged; this is an internal behavior addition.

- [ ] **Step 1: Write the failing tests**

Add to `src-tauri/tests/jsonl_walker.rs` (after `truncation_resets_cursor_and_dedupes`):

```rust
#[test]
fn archives_raw_lines_alongside_events() {
    let (_d, db, p, projects) = setup();
    let f = projects.join("demo").join("session.jsonl");
    fs::copy("tests/fixtures/jsonl/current_schema.jsonl", &f).unwrap();
    walker::ingest_file(&db, &p, &f, &projects).unwrap();

    let rel = f.strip_prefix(&projects).unwrap().to_string_lossy().into_owned();
    let lines = db.transcript_lines_for_path(&rel).unwrap();
    let expected: Vec<&str> = include_str!("fixtures/jsonl/current_schema.jsonl").lines().collect();
    assert_eq!(
        lines.len(),
        expected.len(),
        "every raw line is archived, not just usage-bearing ones"
    );
    for (row, original) in lines.iter().zip(expected.iter()) {
        assert_eq!(row.raw_line, original.trim());
    }
}

#[test]
fn archive_reflects_new_content_after_truncation() {
    let (_d, db, p, projects) = setup();
    let f = projects.join("demo").join("session.jsonl");
    fs::copy("tests/fixtures/jsonl/current_schema.jsonl", &f).unwrap();
    walker::ingest_file(&db, &p, &f, &projects).unwrap();

    fs::write(&f, "{\"different\":true}\n").unwrap();
    walker::ingest_file(&db, &p, &f, &projects).unwrap();

    let rel = f.strip_prefix(&projects).unwrap().to_string_lossy().into_owned();
    let lines = db.transcript_lines_for_path(&rel).unwrap();
    assert_eq!(lines.len(), 1, "offset 0 has exactly one row after truncation, not two");
    assert_eq!(
        lines[0].raw_line, "{\"different\":true}",
        "archive must reflect current content, not the stale first-seen bytes"
    );
}
```

Add `StoredTranscriptLine` to the existing import at the top of the file: change
```rust
use claude_switchboard_lib::store::{Db, StoredAccount};
```
to
```rust
use claude_switchboard_lib::store::{Db, StoredAccount, StoredTranscriptLine};
```
(`StoredTranscriptLine` isn't referenced directly in the test bodies above via that import — `transcript_lines_for_path`'s return type is inferred — so if `cargo build` reports it unused, skip this import change and rely on the inferred type instead.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --test jsonl_walker archives_raw_lines archive_reflects_new_content`
Expected: FAIL — `transcript_lines_for_path` returns 0 rows (nothing archives yet).

- [ ] **Step 3: Wire archiving into `ingest_file`**

In `src-tauri/src/jsonl_parser/walker.rs`, change the import line:
```rust
use crate::store::{Db, StoredCompaction, StoredSessionEvent};
```
to:
```rust
use crate::store::{Db, StoredCompaction, StoredSessionEvent, StoredTranscriptLine};
```

Immediately before `let mut reader = std::io::BufReader::new(f);`, add:
```rust
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
```

Change:
```rust
    let mut reader = std::io::BufReader::new(f);
    let mut buf = Vec::new();
    let mut stored = Vec::<StoredSessionEvent>::new();
    let mut compactions = Vec::<StoredCompaction>::new();
    let mut consumed: i64 = offset;
```
to:
```rust
    let mut reader = std::io::BufReader::new(f);
    let mut buf = Vec::new();
    let mut stored = Vec::<StoredSessionEvent>::new();
    let mut compactions = Vec::<StoredCompaction>::new();
    let mut archive_lines = Vec::<StoredTranscriptLine>::new();
    let mut consumed: i64 = offset;
```

Immediately after:
```rust
        if text.is_empty() {
            continue;
        }
```
add:
```rust
        archive_lines.push(StoredTranscriptLine {
            project_slug: project.clone(),
            session_id: session_id.clone(),
            jsonl_path: source_file_path.clone(),
            line_no: line_start,
            raw_line: text.to_string(),
        });
```

Change the final two lines of the function:
```rust
    let inserted = db.ingest_atomic(&key, &stored, &compactions, mtime_ns, consumed)?;
    Ok(inserted)
}
```
to:
```rust
    // Archived before the cursor advances (ingest_atomic, below, is what
    // advances it): if the process crashes between the two calls, the
    // cursor is still at the old offset, so the next read re-derives and
    // re-inserts the same archive_lines (idempotent via REPLACE) rather
    // than silently skipping them. Errors here are logged, not propagated —
    // the archive is a separate concern from analytics ingestion and must
    // never block it.
    if let Err(e) = db.insert_transcript_lines(&archive_lines) {
        tracing::warn!("archive: failed to insert transcript lines for {}: {}", key, e);
    }

    let inserted = db.ingest_atomic(&key, &stored, &compactions, mtime_ns, consumed)?;
    Ok(inserted)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --test jsonl_walker`
Expected: PASS (all tests in this file, old and new)

- [ ] **Step 5: Run the full test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/jsonl_parser/walker.rs src-tauri/tests/jsonl_walker.rs
git commit -m "feat: archive raw transcript lines during JSONL ingestion"
```

---

### Task 4: Repo discovery — `archive_watcher::discover_project_roots`

**Files:**
- Create: `src-tauri/src/archive_watcher.rs`
- Modify: `src-tauri/src/lib.rs:1` (module declaration)

**Interfaces:**
- Consumes: `sessions::scan::discover_session_files(root: &Path) -> Vec<PathBuf>`, `sessions::recap::parse_session(path: &Path) -> Option<SessionSummary>` (existing, `SessionSummary.cwd: String`).
- Produces: `pub fn discover_project_roots(claude_projects_root: &Path) -> Vec<PathBuf>` — consumed by Task 6.

- [ ] **Step 1: Write the failing tests against a stub**

Create `src-tauri/src/archive_watcher.rs`:

```rust
use crate::sessions::{recap, scan};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Distinct, still-existing repo roots derived from every session
/// transcript's own `cwd` field — the same resolution the Repo tab already
/// relies on. Deliberately not derived by de-slugifying
/// `~/.claude/projects/<slug>` directory names: that's lossy wherever a real
/// path component contains a literal `-`, which `cwd` never is (it's read
/// straight from the JSONL, not reconstructed).
pub fn discover_project_roots(_claude_projects_root: &Path) -> Vec<PathBuf> {
    Vec::new() // stub — real implementation in Step 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_session(project_dir: &Path, file_name: &str, cwd: &str) {
        std::fs::create_dir_all(project_dir).unwrap();
        let line = format!(
            r#"{{"type":"user","timestamp":"2026-01-01T00:00:00Z","cwd":"{cwd}","message":{{"role":"user","content":"hi"}}}}"#
        );
        std::fs::write(project_dir.join(file_name), line + "\n").unwrap();
    }

    #[test]
    fn discovers_distinct_existing_repo_roots() {
        let root = tempdir().unwrap();
        let repo_a = tempdir().unwrap();
        let repo_b = tempdir().unwrap();

        let proj_a = root.path().join("-slug-a");
        write_session(&proj_a, "sess-1.jsonl", repo_a.path().to_str().unwrap());
        write_session(&proj_a, "sess-2.jsonl", repo_a.path().to_str().unwrap());

        let proj_b = root.path().join("-slug-b");
        write_session(&proj_b, "sess-1.jsonl", repo_b.path().to_str().unwrap());

        let found = discover_project_roots(root.path());
        assert_eq!(found.len(), 2, "two distinct repos, deduplicated");
        assert!(found.contains(&repo_a.path().to_path_buf()));
        assert!(found.contains(&repo_b.path().to_path_buf()));
    }

    #[test]
    fn skips_repos_that_no_longer_exist_on_disk() {
        let root = tempdir().unwrap();
        let proj = root.path().join("-slug-gone");
        write_session(&proj, "sess-1.jsonl", "/this/path/does/not/exist/anywhere");
        let found = discover_project_roots(root.path());
        assert!(found.is_empty(), "a deleted/moved repo must not be watched");
    }
}
```

Add `mod archive_watcher;` to `src-tauri/src/lib.rs`, after `mod app_state;` (line 1) and before `pub mod auth;` (line 2) — alphabetical among the existing module declarations.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib archive_watcher::`
Expected: FAIL — `discovers_distinct_existing_repo_roots` fails because the stub always returns an empty `Vec` (`skips_repos_that_no_longer_exist_on_disk` will pass against the stub too, since an empty result also satisfies "must not be watched" — that's expected and fine; the meaningful red signal is the first test).

- [ ] **Step 3: Implement**

Replace the stub body:
```rust
pub fn discover_project_roots(_claude_projects_root: &Path) -> Vec<PathBuf> {
    Vec::new() // stub — real implementation in Step 3
}
```
with:
```rust
pub fn discover_project_roots(claude_projects_root: &Path) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    for f in scan::discover_session_files(claude_projects_root) {
        let Some(summary) = recap::parse_session(&f) else {
            continue;
        };
        if summary.cwd.is_empty() || !seen.insert(summary.cwd.clone()) {
            continue;
        }
        let path = PathBuf::from(&summary.cwd);
        if path.is_dir() {
            roots.push(path);
        }
    }
    roots
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib archive_watcher::`
Expected: PASS

- [ ] **Step 5: Run the full test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/archive_watcher.rs src-tauri/src/lib.rs
git commit -m "feat: discover repo roots for the archive watcher via session cwd"
```

---

### Task 5: Watch targets, hashing, and the live watcher

**Files:**
- Modify: `src-tauri/src/archive_watcher.rs` (append to the file created in Task 4)

**Interfaces:**
- Consumes: `Db::insert_file_snapshot`, `StoredFileSnapshot` (Task 2), `discover_project_roots` (Task 4).
- Produces: `pub enum WatchScope { File { path: PathBuf, kind: &'static str }, MarkdownDir { dir: PathBuf, kind: &'static str } }`, `pub fn fixed_scopes(home: &Path) -> Vec<WatchScope>`, `pub fn repo_scopes(repo_root: &Path) -> Vec<WatchScope>`, `pub fn backfill(db: &Db, scopes: &[WatchScope])`, `pub fn home_dir() -> Option<PathBuf>`, `pub struct ArchiveWatcherHandle`, `pub fn start(db: Arc<Db>, scopes: Vec<WatchScope>) -> Result<ArchiveWatcherHandle>` — `home_dir`, `fixed_scopes`, `repo_scopes`, `backfill`, and `start` are consumed by Task 6.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/archive_watcher.rs` (after the existing two tests):

```rust
    #[test]
    fn fixed_scopes_covers_expected_claude_paths() {
        let home = tempdir().unwrap();
        let scopes = fixed_scopes(home.path());
        let claude = home.path().join(".claude");
        let files: Vec<_> = scopes
            .iter()
            .filter_map(|s| match s {
                WatchScope::File { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(files.contains(&claude.join("settings.json")));
        assert!(files.contains(&claude.join("settings.local.json")));
        assert!(files.contains(&claude.join("CLAUDE.md")));
        assert!(files.contains(&claude.join("history.jsonl")));
        assert!(files.contains(&claude.join("statusline-usage.json")));
        assert!(files.contains(&claude.join("mcp-needs-auth-cache.json")));
        assert!(
            scopes.iter().any(
                |s| matches!(s, WatchScope::MarkdownDir { dir, .. } if dir == &claude.join("plans"))
            ),
            "plans/ is scanned as a directory, not named files"
        );
    }

    #[test]
    fn repo_scopes_covers_claude_md_and_remember_dir() {
        let repo = tempdir().unwrap();
        let scopes = repo_scopes(repo.path());
        assert!(scopes.iter().any(
            |s| matches!(s, WatchScope::File { path, .. } if path == &repo.path().join("CLAUDE.md"))
        ));
        assert!(scopes.iter().any(
            |s| matches!(s, WatchScope::MarkdownDir { dir, .. } if dir == &repo.path().join(".remember"))
        ));
    }

    #[test]
    fn backfill_snapshots_fixed_files_and_dynamic_markdown_dir_contents() {
        let home = tempdir().unwrap();
        let claude = home.path().join(".claude");
        std::fs::create_dir_all(claude.join("plans")).unwrap();
        std::fs::write(claude.join("settings.json"), "{}").unwrap();
        std::fs::write(claude.join("plans").join("one.md"), "# plan one").unwrap();
        std::fs::write(claude.join("plans").join("two.md"), "# plan two").unwrap();

        let db_dir = tempdir().unwrap();
        let db = Db::open(db_dir.path()).unwrap();

        backfill(&db, &fixed_scopes(home.path()));

        let settings_path = claude.join("settings.json").to_string_lossy().into_owned();
        assert_eq!(db.file_snapshots_for_path(&settings_path).unwrap().len(), 1);

        let plan_one = claude.join("plans").join("one.md").to_string_lossy().into_owned();
        let plan_two = claude.join("plans").join("two.md").to_string_lossy().into_owned();
        assert_eq!(db.file_snapshots_for_path(&plan_one).unwrap().len(), 1);
        assert_eq!(db.file_snapshots_for_path(&plan_two).unwrap().len(), 1);
    }

    #[test]
    fn snapshot_file_skips_oversized_and_binary_content() {
        let dir = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let db = Db::open(db_dir.path()).unwrap();

        let huge = dir.path().join("huge.md");
        std::fs::write(&huge, vec![b'a'; (MAX_SNAPSHOT_BYTES + 1) as usize]).unwrap();
        snapshot_file(&db, &huge, "misc");
        assert!(db.file_snapshots_for_path(&huge.to_string_lossy()).unwrap().is_empty());

        let binary = dir.path().join("binary.md");
        std::fs::write(&binary, [0xFF, 0xFE, 0x00, 0xD8]).unwrap();
        snapshot_file(&db, &binary, "misc");
        assert!(db.file_snapshots_for_path(&binary.to_string_lossy()).unwrap().is_empty());

        let normal = dir.path().join("normal.md");
        std::fs::write(&normal, "hello").unwrap();
        snapshot_file(&db, &normal, "misc");
        assert_eq!(db.file_snapshots_for_path(&normal.to_string_lossy()).unwrap().len(), 1);
    }

    #[test]
    fn matching_kind_covers_new_files_in_a_markdown_dir() {
        let scopes = vec![
            WatchScope::File {
                path: PathBuf::from("/home/.claude/settings.json"),
                kind: "settings",
            },
            WatchScope::MarkdownDir { dir: PathBuf::from("/repo/.remember"), kind: "memory" },
        ];
        assert_eq!(
            matching_kind(&scopes, Path::new("/repo/.remember/today-2026-08-22.md")),
            Some("memory"),
            "a file created after scopes were built must still match by directory + extension"
        );
        assert_eq!(
            matching_kind(&scopes, Path::new("/home/.claude/settings.json")),
            Some("settings")
        );
        assert_eq!(
            matching_kind(&scopes, Path::new("/repo/.remember/logs/x.md")),
            None,
            "nested paths are not the watched directory itself"
        );
        assert_eq!(
            matching_kind(&scopes, Path::new("/repo/.remember/not-markdown.txt")),
            None
        );
        assert_eq!(matching_kind(&scopes, Path::new("/unrelated/file.md")), None);
    }
```

Add these imports at the top of the test module (extend the existing `use super::*; use tempfile::tempdir;` in the `#[cfg(test)] mod tests` block — no change needed there since `use super::*;` already pulls in everything below), and add to the top of the *file* (outside the test module, alongside the existing `use` lines from Task 4):

```rust
use crate::store::{Db, StoredFileSnapshot};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebouncedEvent};
use tokio::sync::mpsc;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib archive_watcher::`
Expected: FAIL to compile — `WatchScope`, `fixed_scopes`, `repo_scopes`, `backfill`, `snapshot_file`, `matching_kind`, `MAX_SNAPSHOT_BYTES` don't exist yet.

- [ ] **Step 3: Implement**

Append to `src-tauri/src/archive_watcher.rs` (before the `#[cfg(test)]` module):

```rust
const MAX_SNAPSHOT_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Clone)]
pub enum WatchScope {
    /// A single fixed file (settings.json, CLAUDE.md, history.jsonl, ...).
    File { path: PathBuf, kind: &'static str },
    /// Any *.md file directly inside this directory (plans/, .remember/) —
    /// covers files created after startup (e.g. tomorrow's today-*.md),
    /// which a fixed file list would miss.
    MarkdownDir { dir: PathBuf, kind: &'static str },
}

/// Fixed targets under ~/.claude, independent of any repo.
pub fn fixed_scopes(home: &Path) -> Vec<WatchScope> {
    let claude = home.join(".claude");
    vec![
        WatchScope::File { path: claude.join("settings.json"), kind: "settings" },
        WatchScope::File { path: claude.join("settings.local.json"), kind: "settings" },
        WatchScope::File { path: claude.join("CLAUDE.md"), kind: "claude_md" },
        WatchScope::File { path: claude.join("history.jsonl"), kind: "misc" },
        WatchScope::File { path: claude.join("statusline-usage.json"), kind: "misc" },
        WatchScope::File { path: claude.join("mcp-needs-auth-cache.json"), kind: "misc" },
        WatchScope::MarkdownDir { dir: claude.join("plans"), kind: "plan" },
    ]
}

/// Per-repo targets: the repo's own CLAUDE.md plus every *.md file directly
/// under its .remember/.
pub fn repo_scopes(repo_root: &Path) -> Vec<WatchScope> {
    vec![
        WatchScope::File { path: repo_root.join("CLAUDE.md"), kind: "claude_md" },
        WatchScope::MarkdownDir { dir: repo_root.join(".remember"), kind: "memory" },
    ]
}

/// Every concrete file a scope currently covers. For a MarkdownDir this
/// expands to whatever *.md files exist right now; `start`'s live watcher
/// separately covers files that appear later via `matching_kind`.
fn expand(scope: &WatchScope) -> Vec<(PathBuf, &'static str)> {
    match scope {
        WatchScope::File { path, kind } => vec![(path.clone(), *kind)],
        WatchScope::MarkdownDir { dir, kind } => {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return Vec::new();
            };
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
                .map(|p| (p, *kind))
                .collect()
        }
    }
}

/// Reads and snapshots every file every scope currently covers. Cheap: both
/// `transcript_lines` and `file_snapshots` dedupe unchanged content, so
/// running this on every launch (not just once) costs one hash comparison
/// per already-seen file.
pub fn backfill(db: &Db, scopes: &[WatchScope]) {
    for scope in scopes {
        for (path, kind) in expand(scope) {
            snapshot_file(db, &path, kind);
        }
    }
}

/// Reads `path`, and if it's a file, within the size ceiling, and valid
/// UTF-8, snapshots it via `Db::insert_file_snapshot`. Anything that fails a
/// guard is logged and skipped, never propagated — one bad file must never
/// block the rest.
fn snapshot_file(db: &Db, path: &Path, kind: &'static str) {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return,
    };
    if !meta.is_file() {
        return;
    }
    if meta.len() > MAX_SNAPSHOT_BYTES {
        tracing::warn!(
            "archive: skipping oversized file (>{}MB): {}",
            MAX_SNAPSHOT_BYTES / (1024 * 1024),
            path.display()
        );
        return;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("archive: skipping unreadable/non-UTF-8 file {}: {}", path.display(), e);
            return;
        }
    };
    let content_hash = format!("{:x}", Sha256::digest(content.as_bytes()));
    let snap = StoredFileSnapshot {
        source_path: path.to_string_lossy().into_owned(),
        kind: kind.to_string(),
        content,
        content_hash,
    };
    if let Err(e) = db.insert_file_snapshot(&snap) {
        tracing::warn!("archive: failed to store snapshot for {}: {}", path.display(), e);
    }
}

/// `None` if the OS home directory can't be resolved (matches the existing
/// `jsonl_parser::walker::claude_projects_root` fallback behavior).
pub fn home_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|u| u.home_dir().to_path_buf())
}

fn matching_kind(scopes: &[WatchScope], path: &Path) -> Option<&'static str> {
    for scope in scopes {
        match scope {
            WatchScope::File { path: p, kind } if p == path => return Some(kind),
            WatchScope::MarkdownDir { dir, kind }
                if path.parent() == Some(dir.as_path())
                    && path.extension().and_then(|e| e.to_str()) == Some("md") =>
            {
                return Some(kind);
            }
            _ => {}
        }
    }
    None
}

pub struct ArchiveWatcherHandle {
    _debouncer: notify_debouncer_full::Debouncer<
        notify::RecommendedWatcher,
        notify_debouncer_full::RecommendedCache,
    >,
}

/// Watches every scope's parent directory (or, for a MarkdownDir, the
/// directory itself) non-recursively, and snapshots on any debounced event
/// whose path matches `matching_kind`. A directory that doesn't exist yet
/// (e.g. no settings.local.json ever created, so its parent may still exist
/// but the file itself won't trigger until created — this is fine, `notify`
/// watches the directory) is simply not registered if the directory itself
/// is missing.
pub fn start(db: Arc<Db>, scopes: Vec<WatchScope>) -> Result<ArchiveWatcherHandle> {
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<Vec<DebouncedEvent>>();
    let mut debouncer = new_debouncer(Duration::from_millis(500), None, move |res| {
        if let Ok(events) = res {
            let _ = notify_tx.send(events);
        }
    })?;

    let mut watch_dirs: HashSet<PathBuf> = HashSet::new();
    for scope in &scopes {
        let dir = match scope {
            WatchScope::File { path, .. } => path.parent().map(|p| p.to_path_buf()),
            WatchScope::MarkdownDir { dir, .. } => Some(dir.clone()),
        };
        if let Some(dir) = dir {
            watch_dirs.insert(dir);
        }
    }
    for dir in &watch_dirs {
        if dir.is_dir() {
            let _ = debouncer.watch(dir, RecursiveMode::NonRecursive);
        }
    }

    let db_clone = db.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(events) = notify_rx.recv().await {
            let mut touched = HashSet::<PathBuf>::new();
            for e in &events {
                touched.extend(e.paths.iter().cloned());
            }
            for p in touched {
                if let Some(kind) = matching_kind(&scopes, &p) {
                    snapshot_file(&db_clone, &p, kind);
                }
            }
        }
    });

    Ok(ArchiveWatcherHandle { _debouncer: debouncer })
}
```

Also add `use anyhow::Result;` to the top-of-file imports (alongside the ones added in Step 1) if not already present from Task 4.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib archive_watcher::`
Expected: PASS

- [ ] **Step 5: Run the full test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/archive_watcher.rs
git commit -m "feat: add archive watch targets, hashing, and live file watcher"
```

---

### Task 6: Wire the archive watcher into app startup

**Files:**
- Modify: `src-tauri/src/lib.rs:822-824` (insert new block between the existing JSONL watcher block and the closure's final `Ok(())`)

**Interfaces:**
- Consumes: `archive_watcher::{home_dir, fixed_scopes, repo_scopes, discover_project_roots, backfill, start}` (Tasks 4–5), `jsonl_parser::walker::claude_projects_root()` (existing), `state: Arc<AppState>` and `handle` (existing locals in the setup closure).

- [ ] **Step 1: Add the startup block**

In `src-tauri/src/lib.rs`, between the closing `}` of the existing `if let Some(root) = jsonl_parser::walker::claude_projects_root() { ... }` block (line 822) and the closure's final `Ok(())` (line 824), insert:

```rust
            // Repo discovery depends on the same projects root as the JSONL
            // backfill above, so this runs after it — but independently: a
            // failure here must never affect session ingestion. Backfill and
            // watcher-start are bundled into one task (unlike the JSONL
            // watcher, which starts immediately) because start() needs the
            // scope list, which depends on discovery finishing first.
            if let Some(home) = archive_watcher::home_dir() {
                let archive_state = state.clone();
                tauri::async_runtime::spawn(async move {
                    let mut scopes = archive_watcher::fixed_scopes(&home);
                    if let Some(root) = jsonl_parser::walker::claude_projects_root() {
                        for repo in archive_watcher::discover_project_roots(&root) {
                            scopes.extend(archive_watcher::repo_scopes(&repo));
                        }
                    }
                    archive_watcher::backfill(&archive_state.db, &scopes);
                    match archive_watcher::start(archive_state.db.clone(), scopes) {
                        Ok(watcher_handle) => {
                            Box::leak(Box::new(watcher_handle));
                        }
                        Err(e) => {
                            tracing::error!("archive watcher failed to start: {e}");
                        }
                    }
                });
            }
```

- [ ] **Step 2: Build**

Run: `cd src-tauri && cargo build`
Expected: compiles cleanly.

- [ ] **Step 3: Run the full test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS (every existing and new test)

- [ ] **Step 4: Manual verification**

This wiring step has no isolated unit — it's Tauri app bootstrap glue, consistent with how the existing JSONL watcher's own startup wiring has no direct test either (all its logic is tested at the `walker`/`watcher` module level, same as this feature's Tasks 1–5). Verify by running the app once:

```bash
cd src-tauri && cargo build && ./target/debug/claude-switchboard &
sleep 5
sqlite3 "$(find ~/Library/Application\ Support -iname data.db -path '*ClaudeSwitchboard*' 2>/dev/null | head -1)" \
  "SELECT source_path, kind FROM file_snapshots LIMIT 10;"
sqlite3 "$(find ~/Library/Application\ Support -iname data.db -path '*ClaudeSwitchboard*' 2>/dev/null | head -1)" \
  "SELECT COUNT(*) FROM transcript_lines;"
kill %1
```

Confirm `file_snapshots` has rows for `settings.json` and at least one repo's `CLAUDE.md`/`.remember/*.md` if any exist locally, and `transcript_lines` has a non-zero count matching roughly the volume of local session history. Then touch `~/.claude/settings.json` (e.g. `touch ~/.claude/settings.json`) while the app is running and re-check for a new snapshot row to confirm the live watcher fires.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat: start the durable archive watcher and backfill on launch"
```
