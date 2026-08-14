# Per-account session attribution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Attribute local Claude Code session data (Cost, Models, Trends, Repo, Heatmap, Cache tabs) to whichever managed account was active when it happened, and surface that attribution per-row (or the closest equivalent each tab's data shape allows) instead of silently mixing every account's activity together.

**Architecture:** A new `account_intervals` SQLite table records `(account_uuid, started_at, ended_at)` spans, fed by two hook points that already detect "the active account changed" (the in-app `swap_to_account` command, and the poll loop's existing ≤60s active-account reconciliation). Every session-data read query gains a `LEFT JOIN` against this table by timestamp, threading `account_uuid: Option<String>` through the existing `StoredSessionEvent`/`SessionEvent` type and into new or extended report commands. Six report tabs each get their own account-aware rendering, reusing a shared `AccountBadge` component and color-token set.

**Tech Stack:** Rust (Tauri backend, `rusqlite`, `tauri-specta` for generated TS bindings), React 19 + TypeScript (frontend), Vitest (frontend tests), `cargo test` (backend tests).

**Spec:** `docs/superpowers/specs/2026-08-14-per-account-session-attribution-design.md`

## Global Constraints

- Sessions logged before this ships have no recorded account interval and will always show `account_uuid: null` → rendered as "Unknown" — this is expected, not a bug to fix later.
- A swap made outside Switchboard (manual `claude login`, another tool) is only detected on the poll loop's next reconciliation tick, which runs at least every 60 seconds — attribution near a swap boundary can be off by up to that window.
- No global account filter — every tab keeps showing all accounts' data at once, tagged in place.
- Every new UI color must be a CSS custom property in `src/styles/tokens.css` (both the light `:root` block and the dark-mode block) — no hard-coded hex/oklch values inline in components, per this project's "one tight token set" design principle.
- `cargo test` must pass from `src-tauri/` and the relevant Vitest suite must pass from the repo root after every task.

---

### Task 1: `account_intervals` migration

**Files:**
- Create: `src-tauri/src/store/migrations/0012_account_intervals.sql`
- Modify: `src-tauri/src/store/schema.sql`
- Modify: `src-tauri/src/store/mod.rs:104-193` (`create_fresh_db`, `migrate`)
- Test: `src-tauri/src/store/mod.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `account_intervals` table (`id INTEGER PK`, `account_uuid TEXT NOT NULL`, `started_at INTEGER NOT NULL`, `ended_at INTEGER`), schema version 12.

- [ ] **Step 1: Write the migration file**

```sql
-- src-tauri/src/store/migrations/0012_account_intervals.sql
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
```

- [ ] **Step 2: Add the same table to `schema.sql`**

In `src-tauri/src/store/schema.sql`, immediately after the existing `CREATE TABLE IF NOT EXISTS statusline_install (...)` block (the last table in the file), append:

```sql

CREATE TABLE IF NOT EXISTS account_intervals (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    account_uuid TEXT NOT NULL,
    started_at   INTEGER NOT NULL,
    ended_at     INTEGER,
    FOREIGN KEY (account_uuid) REFERENCES accounts(id)
);
CREATE INDEX IF NOT EXISTS idx_account_intervals_span
    ON account_intervals(started_at, ended_at);
```

- [ ] **Step 3: Bump the schema version in `mod.rs`**

In `src-tauri/src/store/mod.rs`, change `create_fresh_db` (currently stamping `11_i64`):

```rust
    /// Create a brand-new SQLite database with the current schema and stamp
    /// schema_version=12 so that migrate() skips steps meant for older upgrades.
    fn create_fresh_db(db_path: &Path) -> Result<Connection> {
        let conn = Connection::open(db_path).context("open sqlite")?;
        conn.execute_batch(include_str!("schema.sql")).context("apply schema")?;
        conn.execute(
            "INSERT OR REPLACE INTO schema_version (version) VALUES (?1)",
            [12_i64],
        )
        .context("stamp schema version")?;
        Ok(conn)
    }
```

Then in `migrate()`, add a new block right before the final version stamp (after the existing `if current < 11 { ... }` block):

```rust
        if current < 12 {
            tracing::info!("migrating v11 -> v12 (account_intervals for session attribution)");
            conn.execute_batch(include_str!("migrations/0012_account_intervals.sql"))
                .context("apply migration 0012")?;
        }

        conn.execute(
            "INSERT OR REPLACE INTO schema_version (version) VALUES (?1)",
            [12_i64],
        )?;
        Ok(())
    }
```

(This replaces the old `[11_i64]` final stamp — there is only one such block at the end of `migrate()`.)

- [ ] **Step 4: Write the test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/store/mod.rs` (same module that already has `migration_0005_adds_warmup_columns_and_consent_setting`):

```rust
    #[test]
    fn migrates_to_v12_with_account_intervals_table() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path()).expect("open");
        let conn = db.conn();

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 12);

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
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd src-tauri && cargo test migrates_to_v12_with_account_intervals_table -- --nocapture`
Expected: PASS. (This step comes after Steps 1-3 rather than before, unlike this plan's other tasks — a schema migration has no meaningful "red" state to run separately: the migration file, the version bump, and the fresh-DB schema all have to exist together before `Db::open` even produces a database the test can query. If you want to see it fail first, temporarily comment out the `if current < 12 { ... }` block added in Step 3, run the test, expect `no such table: account_intervals`, then restore the block.)

- [ ] **Step 6: Run the full backend test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS (no regressions in existing migration tests)

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/store/migrations/0012_account_intervals.sql src-tauri/src/store/schema.sql src-tauri/src/store/mod.rs
git commit -m "feat(store): add account_intervals table (schema v12)"
```

---

### Task 2: `record_account_transition` Db method

**Files:**
- Modify: `src-tauri/src/store/queries.rs`
- Test: `src-tauri/src/store/queries.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `account_intervals` table (Task 1).
- Produces: `Db::record_account_transition(&self, new_active: Option<&str>, at: DateTime<Utc>) -> Result<()>` — closes the currently-open interval (if its account differs from `new_active`) and opens a new one for `new_active` if `Some` and different. No-ops (no writes) if `new_active` already matches the open interval.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/store/queries.rs` (uses the existing `fresh_db()` helper already in that module):

```rust
    #[test]
    fn record_account_transition_opens_first_interval() {
        let (_dir, db) = fresh_db();
        let t0 = Utc::now();
        db.record_account_transition(Some("acc1"), t0).unwrap();

        let conn = db.conn();
        let (account_uuid, ended_at): (String, Option<i64>) = conn
            .query_row(
                "SELECT account_uuid, ended_at FROM account_intervals",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(account_uuid, "acc1");
        assert_eq!(ended_at, None);
    }

    #[test]
    fn record_account_transition_closes_then_opens_on_change() {
        let (_dir, db) = fresh_db();
        db.upsert_account(&StoredAccount { id: "acc2".into(), email: "b@example.com".into(), display_name: None }).unwrap();
        let t0 = Utc::now();
        let t1 = t0 + chrono::Duration::minutes(5);

        db.record_account_transition(Some("acc1"), t0).unwrap();
        db.record_account_transition(Some("acc2"), t1).unwrap();

        let conn = db.conn();
        let mut stmt = conn
            .prepare("SELECT account_uuid, started_at, ended_at FROM account_intervals ORDER BY started_at")
            .unwrap();
        let rows: Vec<(String, i64, Option<i64>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], ("acc1".into(), t0.timestamp(), Some(t1.timestamp())));
        assert_eq!(rows[1].0, "acc2");
        assert_eq!(rows[1].2, None);
    }

    #[test]
    fn record_account_transition_is_a_noop_when_account_unchanged() {
        let (_dir, db) = fresh_db();
        let t0 = Utc::now();
        let t1 = t0 + chrono::Duration::minutes(1);

        db.record_account_transition(Some("acc1"), t0).unwrap();
        db.record_account_transition(Some("acc1"), t1).unwrap();

        let conn = db.conn();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM account_intervals", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "re-observing the same active account must not create a second interval");
    }

    #[test]
    fn record_account_transition_closes_interval_on_none() {
        let (_dir, db) = fresh_db();
        let t0 = Utc::now();
        let t1 = t0 + chrono::Duration::minutes(1);

        db.record_account_transition(Some("acc1"), t0).unwrap();
        db.record_account_transition(None, t1).unwrap();

        let conn = db.conn();
        let open_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM account_intervals WHERE ended_at IS NULL", [], |r| r.get(0))
            .unwrap();
        assert_eq!(open_count, 0, "no managed account live must leave no open interval");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test record_account_transition -- --nocapture`
Expected: FAIL with `no method named 'record_account_transition' found`

- [ ] **Step 3: Implement `record_account_transition`**

Add to the `impl Db` block in `src-tauri/src/store/queries.rs`, near `record_window_peak` (uses the same `let mut conn = self.conn(); let tx = conn.transaction()?; ... tx.commit()?;` pattern as `insert_events`/`ingest_atomic` elsewhere in this file):

```rust
    /// Closes the currently-open account interval (if its account differs
    /// from `new_active`) and opens a new one for `new_active` (if `Some`
    /// and different from what was already open). No-op if `new_active`
    /// already matches the open interval — both `swap_to_account` and the
    /// poll loop's active-account reconciliation call this, and the poll
    /// loop's later observation of an already-recorded in-app swap must not
    /// create a spurious zero-length interval.
    pub fn record_account_transition(
        &self,
        new_active: Option<&str>,
        at: DateTime<Utc>,
    ) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let open: Option<String> = tx
            .query_row(
                "SELECT account_uuid FROM account_intervals WHERE ended_at IS NULL",
                [],
                |r| r.get(0),
            )
            .optional()?;

        if open.as_deref() == new_active {
            return Ok(());
        }

        if open.is_some() {
            tx.execute(
                "UPDATE account_intervals SET ended_at = ?1 WHERE ended_at IS NULL",
                params![at.timestamp()],
            )?;
        }
        if let Some(uuid) = new_active {
            tx.execute(
                "INSERT INTO account_intervals (account_uuid, started_at, ended_at) VALUES (?1, ?2, NULL)",
                params![uuid, at.timestamp()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd src-tauri && cargo test record_account_transition -- --nocapture`
Expected: PASS (all 4 tests)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/store/queries.rs
git commit -m "feat(store): add record_account_transition"
```

---

### Task 3: Wire transition recording into swap and poll-loop reconciliation

**Files:**
- Modify: `src-tauri/src/commands.rs` (`swap_to_account`, ~line 1053-1068)
- Modify: `src-tauri/src/poll_loop.rs` (`poll_all`, ~line 420-441)

**Interfaces:**
- Consumes: `Db::record_account_transition` (Task 2).

- [ ] **Step 1: Hook `swap_to_account`**

In `src-tauri/src/commands.rs`, find the block right after `swap_to_account` sets `*state.active_slot.write() = Some(slot);` and `*state.active_since.write() = Some(Utc::now());` (the comment above `active_since` explains why it's set there). Immediately after the `active_since` line, add:

```rust
    *state.active_since.write() = Some(Utc::now());

    // Record the swap in account_intervals so local session data (which
    // carries no account identity of its own) can later be attributed to
    // whichever account was live when it happened.
    if let Ok(Some(target)) = state.accounts.get(slot) {
        if let Err(e) = state.db.record_account_transition(Some(&target.account_uuid), Utc::now()) {
            tracing::warn!("failed to record account interval for slot {slot}: {e:#}");
        }
    }
```

- [ ] **Step 2: Hook `poll_loop.rs`'s reconciliation**

In `src-tauri/src/poll_loop.rs`, inside `poll_all`, find:

```rust
    if prev_active_slot != active_slot {
        *state.active_since.write() = Some(Utc::now());
    }
```

Replace with:

```rust
    if prev_active_slot != active_slot {
        *state.active_since.write() = Some(Utc::now());

        let new_account_uuid = active_slot.and_then(|slot| {
            accounts.iter().find(|a| a.slot == slot).map(|a| a.account_uuid.as_str())
        });
        if let Err(e) = state.db.record_account_transition(new_account_uuid, Utc::now()) {
            tracing::warn!("failed to record account interval: {e:#}");
        }
    }
```

- [ ] **Step 3: Build and run the backend test suite**

Run: `cd src-tauri && cargo build && cargo test`
Expected: builds cleanly, all tests PASS (this task has no new unit tests of its own — behavior is covered by Task 2's tests of the underlying method; this wiring is exercised end-to-end manually in the final manual-verification pass at the end of this plan).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/poll_loop.rs
git commit -m "feat: record account transitions on swap and poll-loop reconciliation"
```

---

### Task 4: `StoredSessionEvent.account_uuid` + join in `events_between`

**Files:**
- Modify: `src-tauri/src/store/queries.rs` (`StoredSessionEvent`, `events_between`, and every test-module struct literal that constructs `StoredSessionEvent`)
- Modify: `src-tauri/src/commands.rs` (`test_event` helper in its test module)
- Modify: `src-tauri/src/jsonl_parser/walker.rs` (~line 178, production ingestion construction)
- Modify: `src-tauri/src/live_sessions.rs` (`seed`/`seed_at_line` test helpers, ~lines 407, 439)
- Test: `src-tauri/src/store/queries.rs`

**Interfaces:**
- Consumes: `account_intervals` table (Task 1).
- Produces: `StoredSessionEvent.account_uuid: Option<String>`; `Db::events_between` populates it via a `LEFT JOIN account_intervals`.

- [ ] **Step 1: Add the field to `StoredSessionEvent`**

In `src-tauri/src/store/queries.rs`, change:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct StoredSessionEvent {
    #[specta(type = String)]
    pub ts: DateTime<Utc>,
    pub project: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_5m_tokens: u64,
    pub cache_creation_1h_tokens: u64,
    pub cost_usd: f64,
    pub source_file: String,
    pub source_line: i64,
    /// Stable per-API-call key used for dedup. Format: "{requestId}:{message.id}"
    /// when both are present in the JSONL line, else "{source_file}:{source_line}"
    /// as a structural fallback for older / pre-requestId schemas.
    pub event_id: String,
}
```

to:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct StoredSessionEvent {
    #[specta(type = String)]
    pub ts: DateTime<Utc>,
    pub project: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_5m_tokens: u64,
    pub cache_creation_1h_tokens: u64,
    pub cost_usd: f64,
    pub source_file: String,
    pub source_line: i64,
    /// Stable per-API-call key used for dedup. Format: "{requestId}:{message.id}"
    /// when both are present in the JSONL line, else "{source_file}:{source_line}"
    /// as a structural fallback for older / pre-requestId schemas.
    pub event_id: String,
    /// The managed account whose interval this event's `ts` falls inside,
    /// resolved by `events_between`'s join against `account_intervals`.
    /// `None` means either the event predates account_intervals tracking, or
    /// it landed in a gap where no managed account was live. Not a real
    /// column on `session_events` — only ever populated by query methods
    /// that explicitly join for it; constructing a `StoredSessionEvent` for
    /// insertion (e.g. in the JSONL walker) should always set this to `None`.
    pub account_uuid: Option<String>,
}
```

- [ ] **Step 2: Update `events_between` to join and select it**

In `src-tauri/src/store/queries.rs`, change `events_between`:

```rust
    pub fn events_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StoredSessionEvent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT e.ts, e.project, e.model, e.input_tokens, e.output_tokens, e.cache_read_tokens,
                    e.cache_creation_5m_tokens, e.cache_creation_1h_tokens, e.cost_usd,
                    e.source_file, e.source_line, e.event_id, ai.account_uuid
             FROM session_events e
             LEFT JOIN account_intervals ai
               ON e.ts >= ai.started_at AND (ai.ended_at IS NULL OR e.ts < ai.ended_at)
             WHERE e.ts BETWEEN ?1 AND ?2 ORDER BY e.ts DESC",
        )?;
        let rows = stmt.query_map(params![from.timestamp(), to.timestamp()], |r| {
            Ok(StoredSessionEvent {
                ts: DateTime::from_timestamp(r.get(0)?, 0).unwrap(),
                project: r.get(1)?,
                model: r.get(2)?,
                input_tokens: r.get::<_, i64>(3)? as u64,
                output_tokens: r.get::<_, i64>(4)? as u64,
                cache_read_tokens: r.get::<_, i64>(5)? as u64,
                cache_creation_5m_tokens: r.get::<_, i64>(6)? as u64,
                cache_creation_1h_tokens: r.get::<_, i64>(7)? as u64,
                cost_usd: r.get(8)?,
                source_file: r.get(9)?,
                source_line: r.get(10)?,
                event_id: r.get(11)?,
                account_uuid: r.get(12)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
```

- [ ] **Step 3: Fix every other construction site so the crate compiles**

Run: `cd src-tauri && cargo build 2>&1 | grep "missing field"`
Expected output lists every `StoredSessionEvent { ... }` literal missing `account_uuid`. For each, add `account_uuid: None,` as the last field. Known sites to check (from a repo-wide grep of `StoredSessionEvent {`):
  - `src-tauri/src/store/queries.rs` test module: `event(...)` (~line 862), the `let e = StoredSessionEvent { ... }` at ~line 942, and every `let mk = |...| StoredSessionEvent { ... }` closure (~lines 971, 992, 1101), `mk_event(...)` (~line 1120), and the four inline `db.insert_events(&[StoredSessionEvent { ... }])` call sites (~lines 1271, 1286, 1347, 1363, 1413).
  - `src-tauri/src/commands.rs` test module: `test_event(ts)` helper (~line 1513).
  - `src-tauri/src/jsonl_parser/walker.rs`: the production `stored.push(StoredSessionEvent { ... })` (~line 178) — this is real ingestion code, not a test; add `account_uuid: None,` there too, since the walker only ever writes rows (the join that populates this field happens only on read, in `events_between`).
  - `src-tauri/src/live_sessions.rs` test module: `seed(...)` (~line 407) and `seed_at_line(...)` (~line 439).

For every one of these, add exactly one line: `account_uuid: None,` (matching each site's existing trailing-comma style).

- [ ] **Step 4: Confirm the crate builds clean**

Run: `cd src-tauri && cargo build`
Expected: no errors.

- [ ] **Step 5: Write the failing join-attribution test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/store/queries.rs`:

```rust
    #[test]
    fn events_between_attributes_account_via_interval_join() {
        let (_dir, db) = fresh_db();
        db.upsert_account(&StoredAccount { id: "acc2".into(), email: "b@example.com".into(), display_name: None }).unwrap();

        let t_before = Utc::now() - chrono::Duration::hours(3);
        let t_acc1 = Utc::now() - chrono::Duration::hours(2);
        let t_acc2 = Utc::now() - chrono::Duration::hours(1);

        // acc1 active from t_acc1 to t_acc2, then acc2 active from t_acc2 onward.
        db.record_account_transition(Some("acc1"), t_acc1).unwrap();
        db.record_account_transition(Some("acc2"), t_acc2).unwrap();

        let mk = |ts: chrono::DateTime<Utc>, id: &str| StoredSessionEvent {
            ts,
            project: "p".into(),
            model: "m".into(),
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 0.0,
            source_file: "a.jsonl".into(),
            source_line: 0,
            event_id: id.into(),
            account_uuid: None,
        };
        db.insert_events(&[
            mk(t_before, "before"), // predates any interval
            mk(t_acc1 + chrono::Duration::minutes(1), "during-acc1"),
            mk(t_acc2 + chrono::Duration::minutes(1), "during-acc2"),
        ])
        .unwrap();

        let events = db
            .events_between(t_before - chrono::Duration::minutes(1), Utc::now())
            .unwrap();

        let by_id: std::collections::HashMap<&str, &Option<String>> = events
            .iter()
            .map(|e| (e.event_id.as_str(), &e.account_uuid))
            .collect();

        assert_eq!(by_id.get("before"), Some(&&None), "event before any interval must be unattributed");
        assert_eq!(by_id.get("during-acc1"), Some(&&Some("acc1".to_string())));
        assert_eq!(by_id.get("during-acc2"), Some(&&Some("acc2".to_string())));
    }
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cd src-tauri && cargo test events_between_attributes_account_via_interval_join -- --nocapture`
Expected: PASS

- [ ] **Step 7: Run the full backend test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/store/queries.rs src-tauri/src/commands.rs src-tauri/src/jsonl_parser/walker.rs src-tauri/src/live_sessions.rs
git commit -m "feat(store): attribute session events to accounts via interval join"
```

---

### Task 5: `account_uuids_by_source_file`

**Files:**
- Modify: `src-tauri/src/store/queries.rs`
- Test: `src-tauri/src/store/queries.rs`

**Interfaces:**
- Consumes: `account_intervals` table (Task 1), `record_account_transition` (Task 2).
- Produces: `Db::account_uuids_by_source_file(&self) -> Result<HashMap<String, Vec<String>>>` — distinct account UUIDs per conversation, keyed the same way as `session_totals` (subagent transcripts folded onto their parent's `source_file`).

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/src/store/queries.rs`'s test module:

```rust
    #[test]
    fn account_uuids_by_source_file_folds_subagents_onto_parent_and_dedupes() {
        let (_dir, db) = fresh_db();
        db.upsert_account(&StoredAccount { id: "acc2".into(), email: "b@example.com".into(), display_name: None }).unwrap();

        let t1 = Utc::now() - chrono::Duration::hours(2);
        let t2 = Utc::now() - chrono::Duration::hours(1);
        db.record_account_transition(Some("acc1"), t1).unwrap();
        db.record_account_transition(Some("acc2"), t2).unwrap();

        let mk = |ts: chrono::DateTime<Utc>, src: &str, id: &str| StoredSessionEvent {
            ts,
            project: "p".into(),
            model: "m".into(),
            input_tokens: 1,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 0.0,
            source_file: src.into(),
            source_line: 0,
            event_id: id.into(),
            account_uuid: None,
        };
        db.insert_events(&[
            mk(t1 + chrono::Duration::minutes(1), "proj/abc.jsonl", "e1"),
            mk(t1 + chrono::Duration::minutes(2), "proj/abc/subagents/agent-x.jsonl", "e2"),
            mk(t2 + chrono::Duration::minutes(1), "proj/abc.jsonl", "e3"), // same conversation, resumed under acc2
            mk(t2 + chrono::Duration::minutes(2), "proj/other.jsonl", "e4"),
        ])
        .unwrap();

        let map = db.account_uuids_by_source_file().unwrap();

        let mut abc = map.get("proj/abc.jsonl").cloned().unwrap_or_default();
        abc.sort();
        assert_eq!(abc, vec!["acc1".to_string(), "acc2".to_string()], "subagent folds onto parent; both accounts recorded, no duplicates");
        assert_eq!(map.get("proj/other.jsonl"), Some(&vec!["acc2".to_string()]));
        assert!(!map.contains_key("proj/abc/subagents/agent-x.jsonl"));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test account_uuids_by_source_file -- --nocapture`
Expected: FAIL with `no method named 'account_uuids_by_source_file' found`

- [ ] **Step 3: Implement the method**

Add to `impl Db` in `src-tauri/src/store/queries.rs`, near `session_totals` (mirrors its key-folding logic):

```rust
    /// Distinct account UUIDs whose interval overlaps at least one event in
    /// each conversation, keyed the same way as `session_totals` (subagent
    /// transcripts folded onto their parent's `source_file`). Used by
    /// `get_repo_breakdown` to badge each repo/project with the account(s)
    /// that worked in it — a conversation almost always maps to exactly one
    /// account, but nothing prevents more if it happened to span a swap.
    pub fn account_uuids_by_source_file(&self) -> Result<std::collections::HashMap<String, Vec<String>>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT e.source_file, ai.account_uuid
             FROM session_events e
             LEFT JOIN account_intervals ai
               ON e.ts >= ai.started_at AND (ai.ended_at IS NULL OR e.ts < ai.ended_at)
             WHERE ai.account_uuid IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;

        let mut out: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for row in rows {
            let (source_file, account_uuid) = row?;
            let key = match source_file.find("/subagents/") {
                Some(i) => format!("{}.jsonl", &source_file[..i]),
                None => source_file,
            };
            let list = out.entry(key).or_default();
            if !list.contains(&account_uuid) {
                list.push(account_uuid);
            }
        }
        Ok(out)
    }
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test account_uuids_by_source_file -- --nocapture`
Expected: PASS

- [ ] **Step 5: Run the full backend test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/store/queries.rs
git commit -m "feat(store): add account_uuids_by_source_file"
```

---

### Task 6: Frontend account color tokens, `accountDisplay.ts`, `AccountBadge`

**Files:**
- Modify: `src/styles/tokens.css`
- Create: `src/report/accountDisplay.ts`
- Create: `src/components/ui/AccountBadge.tsx`
- Test: `src/report/accountDisplay.test.ts`

**Interfaces:**
- Consumes: `AccountListEntry` (existing generated binding — `slot: number`, `email: string`, `account_uuid: string`).
- Produces: CSS tokens `--color-account-1`..`--color-account-4`; `accountColor(slot: number): string`; `colorForAccount(accountUuid: string | null, accounts: AccountListEntry[]): string`; `labelForAccount(accountUuid: string | null, accounts: AccountListEntry[]): string`; `<AccountBadge accountUuid={string | null} accounts={AccountListEntry[]} className?={string} />`.

- [ ] **Step 1: Add account color tokens**

In `src/styles/tokens.css`, in the light `:root` block, immediately after the existing `--color-model-haiku-text: var(--color-text);` line, add:

```css
  /* Account identity — hues distinct from the model/status palette (warm
   * terracotta family + teal) so a colored dot never doubles as "which
   * model" and "which account" in the same view. Cycles for a 5th+
   * account rather than growing unboundedly. */
  --color-account-1: oklch(52% 0.14 300);
  --color-account-2: oklch(52% 0.14 250);
  --color-account-3: oklch(52% 0.13 140);
  --color-account-4: oklch(58% 0.16 350);
```

In the dark-mode block, immediately after the corresponding `--color-model-haiku-text: var(--color-text);` line, add:

```css
  --color-account-1: oklch(78% 0.13 300);
  --color-account-2: oklch(74% 0.13 250);
  --color-account-3: oklch(76% 0.12 140);
  --color-account-4: oklch(78% 0.14 350);
```

- [ ] **Step 2: Write the failing test for `accountDisplay.ts`**

Create `src/report/accountDisplay.test.ts`:

```ts
import { describe, it, expect } from 'vitest';
import { accountColor, colorForAccount, labelForAccount } from './accountDisplay';
import type { AccountListEntry } from '../lib/generated/bindings';

function account(overrides: Partial<AccountListEntry> = {}): AccountListEntry {
  return {
    slot: 0,
    email: 'work@example.com',
    account_uuid: 'uuid-1',
    org_name: null,
    org_uuid: null,
    subscription_type: null,
    source: 'OAuth',
    is_active: false,
    cached_usage: null,
    last_error: null,
    ...overrides,
  };
}

describe('accountColor', () => {
  it('is deterministic per slot and cycles the palette', () => {
    expect(accountColor(0)).toBe('var(--color-account-1)');
    expect(accountColor(4)).toBe(accountColor(0));
  });
});

describe('colorForAccount', () => {
  it('resolves the color of the account matching accountUuid', () => {
    const accounts = [account({ slot: 2, account_uuid: 'uuid-a' })];
    expect(colorForAccount('uuid-a', accounts)).toBe(accountColor(2));
  });

  it('falls back to the muted color for null or unmatched uuids', () => {
    const accounts = [account({ slot: 0, account_uuid: 'uuid-a' })];
    expect(colorForAccount(null, accounts)).toBe('var(--color-text-muted)');
    expect(colorForAccount('uuid-missing', accounts)).toBe('var(--color-text-muted)');
  });
});

describe('labelForAccount', () => {
  it('shows the email local-part for a matched account', () => {
    const accounts = [account({ account_uuid: 'uuid-a', email: 'jay@work.com' })];
    expect(labelForAccount('uuid-a', accounts)).toBe('jay');
  });

  it('shows "Unknown" for null or unmatched uuids', () => {
    const accounts = [account({ account_uuid: 'uuid-a' })];
    expect(labelForAccount(null, accounts)).toBe('Unknown');
    expect(labelForAccount('uuid-missing', accounts)).toBe('Unknown');
  });
});
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `npx vitest run src/report/accountDisplay.test.ts`
Expected: FAIL — `Failed to resolve import "./accountDisplay"`

- [ ] **Step 4: Implement `accountDisplay.ts`**

Create `src/report/accountDisplay.ts`:

```ts
import type { AccountListEntry } from '../lib/generated/bindings';

const PALETTE = [
  'var(--color-account-1)',
  'var(--color-account-2)',
  'var(--color-account-3)',
  'var(--color-account-4)',
] as const;

const UNKNOWN_COLOR = 'var(--color-text-muted)';

/** Deterministic color for an account, keyed by its stable `slot` number so
 *  the color doesn't shift as other accounts are added or removed. Cycles
 *  the 4-color palette for a 5th+ account rather than growing it. */
export function accountColor(slot: number): string {
  return PALETTE[slot % PALETTE.length];
}

/** Color for a row/entry's `account_uuid` (as returned by the account-aware
 *  report commands). `null` — no attribution, either pre-feature history or
 *  a gap with no managed account live — and an unmatched uuid (the account
 *  was since removed) both render muted. */
export function colorForAccount(accountUuid: string | null, accounts: AccountListEntry[]): string {
  if (!accountUuid) return UNKNOWN_COLOR;
  const account = accounts.find((a) => a.account_uuid === accountUuid);
  return account ? accountColor(account.slot) : UNKNOWN_COLOR;
}

/** Short display label for an account badge: the email's local-part, matching
 *  how tight-space UI elsewhere favors recognizable identity over the full
 *  address. */
export function labelForAccount(accountUuid: string | null, accounts: AccountListEntry[]): string {
  if (!accountUuid) return 'Unknown';
  const account = accounts.find((a) => a.account_uuid === accountUuid);
  return account ? account.email.split('@')[0] : 'Unknown';
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `npx vitest run src/report/accountDisplay.test.ts`
Expected: PASS

- [ ] **Step 6: Implement `AccountBadge`**

Create `src/components/ui/AccountBadge.tsx`:

```tsx
import type { AccountListEntry } from '../../lib/generated/bindings';
import { colorForAccount, labelForAccount } from '../../report/accountDisplay';

interface Props {
  accountUuid: string | null;
  accounts: AccountListEntry[];
  className?: string;
}

/**
 * A colored dot + short label identifying which account a row's data
 * belongs to. Renders alongside `ModelBadge` rather than replacing it —
 * model and account are independent facets of the same row. Color is
 * per-account (dynamic, not one of `Badge`'s fixed variants), so this
 * renders its own pill rather than composing `Badge`.
 */
export function AccountBadge({ accountUuid, accounts, className = '' }: Props) {
  const color = colorForAccount(accountUuid, accounts);
  const label = labelForAccount(accountUuid, accounts);
  return (
    <span
      className={[
        'inline-flex items-center gap-[var(--space-2xs)]',
        'px-[7px] py-[2px]',
        'rounded-[var(--radius-pill)]',
        'text-[length:var(--text-micro)] font-[var(--weight-medium)]',
        'select-none',
        'bg-[var(--color-track)]',
        className,
      ].join(' ')}
    >
      <span aria-hidden className="w-[6px] h-[6px] rounded-full shrink-0" style={{ background: color }} />
      <span style={{ color }}>{label}</span>
    </span>
  );
}
```

- [ ] **Step 7: Run the full frontend test suite**

Run: `npx vitest run`
Expected: PASS (no regressions)

- [ ] **Step 8: Commit**

```bash
git add src/styles/tokens.css src/report/accountDisplay.ts src/report/accountDisplay.test.ts src/components/ui/AccountBadge.tsx
git commit -m "feat(ui): add account color tokens, accountDisplay helpers, AccountBadge"
```

---

### Task 7: Per-account model breakdown

**Files:**
- Modify: `src-tauri/src/commands.rs` (`ModelStats`, `get_model_breakdown`, `get_daily_model_breakdown`)
- Test: `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `StoredSessionEvent.account_uuid` (Task 4).
- Produces: `ModelAccountShare { account_uuid: Option<String>, input_tokens: u64, output_tokens: u64, cost_usd: f64 }`; `ModelStats.by_account: Vec<ModelAccountShare>`; shared `fn accumulate_model_stats(events: &[StoredSessionEvent]) -> Vec<ModelStats>`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/commands.rs` (reuses the existing `test_event` helper, extended in Task 4 with `account_uuid: None`):

```rust
    #[test]
    fn accumulate_model_stats_splits_each_model_by_account() {
        let mut e1 = test_event(Utc::now());
        e1.model = "claude-opus-5".into();
        e1.account_uuid = Some("acc1".into());
        e1.input_tokens = 10;
        e1.output_tokens = 5;

        let mut e2 = e1.clone();
        e2.account_uuid = Some("acc2".into());
        e2.input_tokens = 3;
        e2.output_tokens = 1;
        e2.event_id = "evt-2".into();

        let mut e3 = e1.clone();
        e3.account_uuid = None; // pre-feature / unattributed history
        e3.input_tokens = 7;
        e3.output_tokens = 0;
        e3.event_id = "evt-3".into();

        let stats = accumulate_model_stats(&[e1, e2, e3]);
        assert_eq!(stats.len(), 1);
        let opus = &stats[0];
        assert_eq!(opus.input_tokens, 20);

        let mut by_account = opus.by_account.clone();
        by_account.sort_by(|a, b| a.account_uuid.cmp(&b.account_uuid));
        assert_eq!(by_account.len(), 3);
        assert_eq!(by_account[0].account_uuid, None);
        assert_eq!(by_account[0].input_tokens, 7);
        assert_eq!(by_account[1].account_uuid, Some("acc1".to_string()));
        assert_eq!(by_account[1].input_tokens, 10);
        assert_eq!(by_account[2].account_uuid, Some("acc2".to_string()));
        assert_eq!(by_account[2].input_tokens, 3);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test accumulate_model_stats_splits_each_model_by_account -- --nocapture`
Expected: FAIL — `cannot find function 'accumulate_model_stats'`

- [ ] **Step 3: Add `ModelAccountShare` and extend `ModelStats`**

In `src-tauri/src/commands.rs`, change:

```rust
#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct ModelStats {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cost_usd: f64,
}
```

to:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, PartialEq)]
pub struct ModelAccountShare {
    pub account_uuid: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct ModelStats {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cost_usd: f64,
    /// Per-account contribution to this model's totals — lets the Models
    /// tab show which account(s) drove usage of a given model without a
    /// separate command.
    pub by_account: Vec<ModelAccountShare>,
}
```

- [ ] **Step 4: Factor the shared `accumulate_model_stats` helper and use it from both commands**

Replace the body of `get_model_breakdown` and `get_daily_model_breakdown`'s inner per-day accumulation with calls to a new shared function. In `src-tauri/src/commands.rs`, add (near `bucket_daily_trends`):

```rust
/// Groups events by model, summing tokens/cost and splitting each model's
/// totals by account. Shared by `get_model_breakdown` (whole window) and
/// `get_daily_model_breakdown` (once per day).
fn accumulate_model_stats(events: &[StoredSessionEvent]) -> Vec<ModelStats> {
    use std::collections::HashMap;

    struct Acc {
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_creation_tokens: u64,
        cost_usd: f64,
        by_account: HashMap<Option<String>, ModelAccountShare>,
    }

    let mut by_model: HashMap<String, Acc> = HashMap::new();
    for e in events {
        let entry = by_model.entry(e.model.clone()).or_insert_with(|| Acc {
            model: e.model.clone(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            cost_usd: 0.0,
            by_account: HashMap::new(),
        });
        entry.input_tokens += e.input_tokens;
        entry.output_tokens += e.output_tokens;
        entry.cache_read_tokens += e.cache_read_tokens;
        entry.cache_creation_tokens += e.cache_creation_5m_tokens + e.cache_creation_1h_tokens;
        entry.cost_usd += e.cost_usd;

        let share = entry
            .by_account
            .entry(e.account_uuid.clone())
            .or_insert_with(|| ModelAccountShare {
                account_uuid: e.account_uuid.clone(),
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: 0.0,
            });
        share.input_tokens += e.input_tokens;
        share.output_tokens += e.output_tokens;
        share.cost_usd += e.cost_usd;
    }

    by_model
        .into_values()
        .map(|a| ModelStats {
            model: a.model,
            input_tokens: a.input_tokens,
            output_tokens: a.output_tokens,
            cache_read_tokens: a.cache_read_tokens,
            cache_creation_tokens: a.cache_creation_tokens,
            cost_usd: a.cost_usd,
            by_account: a.by_account.into_values().collect(),
        })
        .collect()
}
```

Then replace `get_model_breakdown`'s body:

```rust
#[command]
#[specta::specta]
pub async fn get_model_breakdown(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<ModelStats>, String> {
    let events = get_session_history(days, state).await?;
    Ok(accumulate_model_stats(&events))
}
```

And replace `get_daily_model_breakdown`'s per-day inner loop with a call to the same helper, keeping its day-bucketing shell:

```rust
#[command]
#[specta::specta]
pub async fn get_daily_model_breakdown(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<DailyModelBucket>, String> {
    let events = get_session_history(days, state).await?;
    use std::collections::BTreeMap;
    let mut by_day: BTreeMap<String, Vec<StoredSessionEvent>> = BTreeMap::new();
    for e in events {
        let date = e.ts.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string();
        by_day.entry(date).or_default().push(e);
    }
    Ok(by_day
        .into_iter()
        .map(|(date, day_events)| {
            let mut models = accumulate_model_stats(&day_events);
            models.sort_by(|a, b| b.cost_usd.total_cmp(&a.cost_usd));
            DailyModelBucket { date, models }
        })
        .collect())
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd src-tauri && cargo test accumulate_model_stats_splits_each_model_by_account -- --nocapture`
Expected: PASS

- [ ] **Step 6: Run the full backend test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "feat: split model breakdown totals by account"
```

---

### Task 8: `get_daily_account_breakdown` command

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Test: `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `StoredSessionEvent.account_uuid` (Task 4), `bucket_daily_trends`'s day-bucketing pattern.
- Produces: `AccountStats { account_uuid: Option<String>, input_tokens: u64, output_tokens: u64, cost_usd: f64 }`; `DailyAccountBucket { date: String, accounts: Vec<AccountStats> }`; `get_daily_account_breakdown(days: u32, state) -> Result<Vec<DailyAccountBucket>, String>`.

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/src/commands.rs`'s test module:

```rust
    #[test]
    fn bucket_daily_account_breakdown_splits_each_day_by_account() {
        let day0 = Utc::now();
        let day1 = Utc::now() - Duration::days(1);

        let mut e1 = test_event(day0);
        e1.account_uuid = Some("acc1".into());
        e1.input_tokens = 10;

        let mut e2 = test_event(day0);
        e2.event_id = "evt-2".into();
        e2.account_uuid = Some("acc2".into());
        e2.input_tokens = 4;

        let mut e3 = test_event(day1);
        e3.event_id = "evt-3".into();
        e3.account_uuid = Some("acc1".into());
        e3.input_tokens = 6;

        let buckets = bucket_daily_account_breakdown(&[e1, e2, e3]);
        assert_eq!(buckets.len(), 2);

        let day0_key = day0.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string();
        let day0_bucket = buckets.iter().find(|b| b.date == day0_key).unwrap();
        let mut accounts = day0_bucket.accounts.clone();
        accounts.sort_by(|a, b| a.account_uuid.cmp(&b.account_uuid));
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].account_uuid, Some("acc1".to_string()));
        assert_eq!(accounts[0].input_tokens, 10);
        assert_eq!(accounts[1].account_uuid, Some("acc2".to_string()));
        assert_eq!(accounts[1].input_tokens, 4);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test bucket_daily_account_breakdown_splits_each_day_by_account -- --nocapture`
Expected: FAIL — `cannot find function 'bucket_daily_account_breakdown'`

- [ ] **Step 3: Implement the structs, helper, and command**

Add to `src-tauri/src/commands.rs`, near `DailyModelBucket`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, PartialEq)]
pub struct AccountStats {
    pub account_uuid: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct DailyAccountBucket {
    pub date: String,
    pub accounts: Vec<AccountStats>,
}
```

Add the pure bucketing function near `bucket_daily_trends`:

```rust
/// Groups events by local calendar day, then splits each day's tokens/cost
/// by account. Mirrors `bucket_daily_trends`'s day-bucketing but adds the
/// account dimension — backs the Trends tab's "color by account" toggle and
/// the Heatmap tab's dominant-account indicator.
fn bucket_daily_account_breakdown(events: &[StoredSessionEvent]) -> Vec<DailyAccountBucket> {
    use std::collections::{BTreeMap, HashMap};
    let mut by_day: BTreeMap<String, HashMap<Option<String>, AccountStats>> = BTreeMap::new();
    for e in events {
        let date = e.ts.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string();
        let by_account = by_day.entry(date).or_default();
        let entry = by_account
            .entry(e.account_uuid.clone())
            .or_insert_with(|| AccountStats {
                account_uuid: e.account_uuid.clone(),
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: 0.0,
            });
        entry.input_tokens += e.input_tokens;
        entry.output_tokens += e.output_tokens;
        entry.cost_usd += e.cost_usd;
    }
    by_day
        .into_iter()
        .map(|(date, accounts)| DailyAccountBucket {
            date,
            accounts: accounts.into_values().collect(),
        })
        .collect()
}

#[command]
#[specta::specta]
pub async fn get_daily_account_breakdown(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<DailyAccountBucket>, String> {
    let events = get_session_history(days, state).await?;
    Ok(bucket_daily_account_breakdown(&events))
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test bucket_daily_account_breakdown_splits_each_day_by_account -- --nocapture`
Expected: PASS

- [ ] **Step 5: Run the full backend test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "feat: add get_daily_account_breakdown command"
```

---

### Task 9: `get_cache_stats_by_account` command

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Test: `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `StoredSessionEvent.account_uuid` (Task 4), `state.pricing.cache_savings_per_mtok` (existing, used by `get_cache_stats`).
- Produces: `AccountCacheStats { account_uuid: Option<String>, total_cache_read_tokens: u64, total_cache_creation_tokens: u64, estimated_savings_usd: f64, hit_ratio: f64 }`; `get_cache_stats_by_account(days: u32, state) -> Result<Vec<AccountCacheStats>, String>`.

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/src/commands.rs`'s test module:

```rust
    #[test]
    fn accumulate_cache_stats_by_account_splits_read_and_created_tokens() {
        let mut e1 = test_event(Utc::now());
        e1.account_uuid = Some("acc1".into());
        e1.cache_read_tokens = 100;
        e1.cache_creation_5m_tokens = 20;

        let mut e2 = test_event(Utc::now());
        e2.event_id = "evt-2".into();
        e2.account_uuid = Some("acc2".into());
        e2.cache_read_tokens = 10;
        e2.cache_creation_1h_tokens = 5;

        let stats = accumulate_cache_stats_by_account(&[e1, e2]);
        assert_eq!(stats.len(), 2);

        let acc1 = stats.iter().find(|s| s.account_uuid == Some("acc1".to_string())).unwrap();
        assert_eq!(acc1.total_cache_read_tokens, 100);
        assert_eq!(acc1.total_cache_creation_tokens, 20);
        assert_eq!(acc1.hit_ratio, 100.0 / 120.0);

        let acc2 = stats.iter().find(|s| s.account_uuid == Some("acc2".to_string())).unwrap();
        assert_eq!(acc2.total_cache_read_tokens, 10);
        assert_eq!(acc2.total_cache_creation_tokens, 5);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd src-tauri && cargo test accumulate_cache_stats_by_account_splits_read_and_created_tokens -- --nocapture`
Expected: FAIL — `cannot find function 'accumulate_cache_stats_by_account'`

- [ ] **Step 3: Implement the struct, helper, and command**

Add to `src-tauri/src/commands.rs`, near `CacheStats`:

```rust
#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct AccountCacheStats {
    pub account_uuid: Option<String>,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub estimated_savings_usd: f64,
    pub hit_ratio: f64,
}
```

Add the pricing-free accumulation helper (mirrors `get_cache_stats`'s read/created counting, without the savings calculation — savings needs `pricing`, added separately in the command since it's `AppState`-scoped, matching how `get_cache_stats` clones `state.pricing` before consuming `state`):

```rust
/// Sums cache read/creation tokens per account and computes each account's
/// own hit ratio. Savings (which needs the pricing table) is filled in by
/// the caller, `get_cache_stats_by_account`, after this returns.
fn accumulate_cache_stats_by_account(events: &[StoredSessionEvent]) -> Vec<AccountCacheStats> {
    use std::collections::HashMap;
    struct Acc {
        read: u64,
        created: u64,
    }
    let mut by_account: HashMap<Option<String>, Acc> = HashMap::new();
    for e in events {
        let entry = by_account
            .entry(e.account_uuid.clone())
            .or_insert(Acc { read: 0, created: 0 });
        entry.read += e.cache_read_tokens;
        entry.created += e.cache_creation_5m_tokens + e.cache_creation_1h_tokens;
    }
    by_account
        .into_iter()
        .map(|(account_uuid, acc)| {
            let total = acc.read + acc.created;
            let hit_ratio = if total > 0 { acc.read as f64 / total as f64 } else { 0.0 };
            AccountCacheStats {
                account_uuid,
                total_cache_read_tokens: acc.read,
                total_cache_creation_tokens: acc.created,
                estimated_savings_usd: 0.0,
                hit_ratio,
            }
        })
        .collect()
}

#[command]
#[specta::specta]
pub async fn get_cache_stats_by_account(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<AccountCacheStats>, String> {
    let pricing = state.pricing.clone();
    let events = get_session_history(days, state).await?;
    let mut stats = accumulate_cache_stats_by_account(&events);

    let mut savings_by_account: std::collections::HashMap<Option<String>, f64> = std::collections::HashMap::new();
    for e in &events {
        let savings = pricing.cache_savings_per_mtok(&e.model).unwrap_or(0.0) * (e.cache_read_tokens as f64) / 1_000_000.0;
        *savings_by_account.entry(e.account_uuid.clone()).or_insert(0.0) += savings;
    }
    for s in &mut stats {
        s.estimated_savings_usd = savings_by_account.get(&s.account_uuid).copied().unwrap_or(0.0);
    }

    Ok(stats)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd src-tauri && cargo test accumulate_cache_stats_by_account_splits_read_and_created_tokens -- --nocapture`
Expected: PASS

- [ ] **Step 5: Run the full backend test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands.rs
git commit -m "feat: add get_cache_stats_by_account command"
```

---

### Task 10: Per-account repo/project attribution

**Files:**
- Modify: `src-tauri/src/sessions/recap.rs` (`SessionSummary`, `parse_session`)
- Modify: `src-tauri/src/commands.rs` (`RepoProjectStats`, `RepoStats`, `list_resumable_sessions`, `get_repo_breakdown`)
- Test: `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `Db::account_uuids_by_source_file` (Task 5).
- Produces: `SessionSummary.account_uuids: Vec<String>`; `RepoProjectStats.account_uuids: Vec<String>`; `RepoStats.account_uuids: Vec<String>`.

- [ ] **Step 1: Add `account_uuids` to `SessionSummary`**

In `src-tauri/src/sessions/recap.rs`, add a field right after `pub total_cost_usd: f64,`:

```rust
    pub total_cost_usd: f64,
    /// Accounts whose interval overlapped at least one event in this
    /// conversation. Filled in by `list_resumable_sessions` from
    /// `Db::account_uuids_by_source_file`, the same way `total_tokens`/
    /// `total_cost_usd` are filled in from `Db::session_totals` — `parse_session`
    /// reads one transcript in isolation and has no account context of its own.
    pub account_uuids: Vec<String>,
```

In the same file, find the `Some(SessionSummary { ... total_tokens: 0, total_cost_usd: 0.0, ... })` construction (~line 334-355) and add `account_uuids: Vec::new(),` immediately after `total_cost_usd: 0.0,`.

- [ ] **Step 2: Wire the lookup into `list_resumable_sessions`**

In `src-tauri/src/commands.rs`, `list_resumable_sessions` currently does:

```rust
    let totals = state.db.session_totals().unwrap_or_default();

    let mut rows: Vec<SessionSummary> = Vec::new();
    for f in &files {
        if let Some(mut s) = recap::parse_session(f) {
            if let Some((tokens, cost)) = f
                .strip_prefix(&root)
                .ok()
                .and_then(|rel| totals.get(rel.to_string_lossy().as_ref()))
            {
                s.total_tokens = *tokens;
                s.total_cost_usd = *cost;
            }
            rows.push(s);
            if rows.len() >= MAX_SESSIONS {
                break;
            }
        }
    }
```

Change to:

```rust
    let totals = state.db.session_totals().unwrap_or_default();
    let account_uuids = state.db.account_uuids_by_source_file().unwrap_or_default();

    let mut rows: Vec<SessionSummary> = Vec::new();
    for f in &files {
        if let Some(mut s) = recap::parse_session(f) {
            if let Ok(rel) = f.strip_prefix(&root) {
                let rel_str = rel.to_string_lossy();
                if let Some((tokens, cost)) = totals.get(rel_str.as_ref()) {
                    s.total_tokens = *tokens;
                    s.total_cost_usd = *cost;
                }
                if let Some(uuids) = account_uuids.get(rel_str.as_ref()) {
                    s.account_uuids = uuids.clone();
                }
            }
            rows.push(s);
            if rows.len() >= MAX_SESSIONS {
                break;
            }
        }
    }
```

- [ ] **Step 3: Write the failing test for `get_repo_breakdown`'s account propagation**

Add to `src-tauri/src/commands.rs`'s test module. Since `get_repo_breakdown` groups `Vec<SessionSummary>` by `resolve_repo_name(&s.cwd)`, and the union logic lives inline in `get_repo_breakdown` itself, extract that grouping into a pure, directly-testable helper first (Step 4), then test it here. `resolve_repo_name` walks the filesystem looking for a real `.git` directory, which a unit test with fake paths can't control precisely, so this test uses the same `cwd` twice (as if the conversation were resumed under a second account) to exercise the account-union logic without depending on `resolve_repo_name`'s fallback behavior:

```rust
    #[test]
    fn group_repo_stats_unions_account_uuids_across_a_repos_projects() {
        let mk = |cwd: &str, accounts: &[&str]| crate::sessions::SessionSummary {
            session_id: "s".into(),
            cwd: cwd.into(),
            project_name: cwd.into(),
            git_branch: None,
            title: "t".into(),
            recap: None,
            asked: "a".into(),
            left_off: None,
            touched_files: vec![],
            touched_overflow: 0,
            model: None,
            peak_context_tokens: None,
            turns: 1,
            started_at: "s".into(),
            ended_at: "e".into(),
            total_tokens: 10,
            total_cost_usd: 1.0,
            account_uuids: accounts.iter().map(|s| s.to_string()).collect(),
            permission_mode: None,
            cwd_exists: true,
        };
        // Same cwd used twice (as if resumed under a second account) — the
        // repo/project entry must union rather than overwrite.
        let sessions = vec![mk("/tmp/no-git-here", &["acc1"]), mk("/tmp/no-git-here", &["acc2"])];

        let repos = group_repo_stats(&sessions);
        assert_eq!(repos.len(), 1);
        let mut repo_accounts = repos[0].account_uuids.clone();
        repo_accounts.sort();
        assert_eq!(repo_accounts, vec!["acc1".to_string(), "acc2".to_string()]);
        assert_eq!(repos[0].projects.len(), 1);
        let mut proj_accounts = repos[0].projects[0].account_uuids.clone();
        proj_accounts.sort();
        assert_eq!(proj_accounts, vec!["acc1".to_string(), "acc2".to_string()]);
    }
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cd src-tauri && cargo test group_repo_stats_unions_account_uuids -- --nocapture`
Expected: FAIL — `cannot find function 'group_repo_stats'`

- [ ] **Step 5: Extract `group_repo_stats` and add `account_uuids` to `RepoStats`/`RepoProjectStats`**

In `src-tauri/src/commands.rs`, extend the structs:

```rust
#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct RepoProjectStats {
    pub project: String,
    pub cwd: String,
    pub session_count: u64,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    pub account_uuids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct RepoStats {
    pub repo: String,
    pub session_count: u64,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    pub projects: Vec<RepoProjectStats>,
    pub account_uuids: Vec<String>,
}
```

Extract the grouping logic out of `get_repo_breakdown` into a pure, testable function, and have the command call it:

```rust
fn group_repo_stats(sessions: &[SessionSummary]) -> Vec<RepoStats> {
    use std::collections::HashMap;

    let mut by_repo: HashMap<String, RepoStats> = HashMap::new();
    let mut by_project: HashMap<(String, String), RepoProjectStats> = HashMap::new();

    for s in sessions {
        let repo = resolve_repo_name(&s.cwd);

        let repo_entry = by_repo.entry(repo.clone()).or_insert_with(|| RepoStats {
            repo: repo.clone(),
            session_count: 0,
            total_tokens: 0,
            total_cost_usd: 0.0,
            projects: Vec::new(),
            account_uuids: Vec::new(),
        });
        repo_entry.session_count += 1;
        repo_entry.total_tokens += s.total_tokens;
        repo_entry.total_cost_usd += s.total_cost_usd;
        for uuid in &s.account_uuids {
            if !repo_entry.account_uuids.contains(uuid) {
                repo_entry.account_uuids.push(uuid.clone());
            }
        }

        let proj_entry = by_project
            .entry((repo, s.cwd.clone()))
            .or_insert_with(|| RepoProjectStats {
                project: s.project_name.clone(),
                cwd: s.cwd.clone(),
                session_count: 0,
                total_tokens: 0,
                total_cost_usd: 0.0,
                account_uuids: Vec::new(),
            });
        proj_entry.session_count += 1;
        proj_entry.total_tokens += s.total_tokens;
        proj_entry.total_cost_usd += s.total_cost_usd;
        for uuid in &s.account_uuids {
            if !proj_entry.account_uuids.contains(uuid) {
                proj_entry.account_uuids.push(uuid.clone());
            }
        }
    }

    for ((repo, _cwd), proj) in by_project {
        if let Some(entry) = by_repo.get_mut(&repo) {
            entry.projects.push(proj);
        }
    }

    let mut out: Vec<RepoStats> = by_repo.into_values().collect();
    for repo in &mut out {
        repo.projects.sort_by(|a, b| b.total_cost_usd.total_cmp(&a.total_cost_usd));
    }
    out.sort_by(|a, b| b.total_cost_usd.total_cmp(&a.total_cost_usd));
    out
}

#[command]
#[specta::specta]
pub async fn get_repo_breakdown(state: State<'_, Arc<AppState>>) -> Result<Vec<RepoStats>, String> {
    let sessions = list_resumable_sessions(state).await?;
    Ok(group_repo_stats(&sessions))
}
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cd src-tauri && cargo test group_repo_stats_unions_account_uuids -- --nocapture`
Expected: PASS

- [ ] **Step 7: Run the full backend test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/sessions/recap.rs src-tauri/src/commands.rs
git commit -m "feat: attribute repo/project breakdown rows to accounts"
```

---

### Task 11: Register new commands and regenerate TypeScript bindings

**Files:**
- Modify: `src-tauri/src/lib.rs` (both `collect_commands!` blocks)
- Modify (generated, do not hand-edit): `src/lib/generated/bindings.ts`
- Modify: `src/lib/ipc.ts`
- Modify: `src/lib/types.ts`

**Interfaces:**
- Consumes: `get_daily_account_breakdown` (Task 8), `get_cache_stats_by_account` (Task 9).
- Produces: `ipc.getDailyAccountBreakdown(days: number)` / `ipc.getCacheStatsByAccount(days: number)`; `bindings.ts` updated with `AccountStats`, `DailyAccountBucket`, `AccountCacheStats`, `ModelAccountShare`, and the extended `ModelStats`/`RepoStats`/`RepoProjectStats`/`StoredSessionEvent` shapes from Tasks 4, 7, 10; `lib/types.ts` re-exporting all of the above plus `RepoStats`/`RepoProjectStats` so tab tests can import fixture types the same way existing tab tests do.

- [ ] **Step 1: Add the two new commands to both `collect_commands!` blocks**

In `src-tauri/src/lib.rs`, in the `#[cfg(not(debug_assertions))]` block, add two lines after `commands::get_cache_stats,`:

```rust
            commands::get_cache_stats,
            commands::get_daily_account_breakdown,
            commands::get_cache_stats_by_account,
```

Do the same in the `#[cfg(debug_assertions)]` block, at the same position relative to `commands::get_cache_stats,`.

- [ ] **Step 2: Regenerate `bindings.ts`**

Run: `cd src-tauri && cargo run` (a debug build; the specta export happens as a side effect of `run()` before the DB opens — Ctrl-C once the app window appears, the file is already written by then).

Expected: `src/lib/generated/bindings.ts` (repo root, i.e. `../src/lib/generated/bindings.ts` relative to `src-tauri/`) is rewritten containing `AccountStats`, `DailyAccountBucket`, `AccountCacheStats`, `ModelAccountShare`, and the updated `ModelStats` (`by_account` field), `RepoStats`/`RepoProjectStats` (`account_uuids` field), `StoredSessionEvent` (`account_uuid` field), plus `getDailyAccountBreakdown`/`getCacheStatsByAccount` entries in the `commands` object.

- [ ] **Step 3: Add the two new calls to the `ipc` wrapper**

In `src/lib/ipc.ts`, add two lines after `getCacheStats: (days: number) => commands.getCacheStats(days).then(unwrap),`:

```ts
  getCacheStats: (days: number) => commands.getCacheStats(days).then(unwrap),
  getDailyAccountBreakdown: (days: number) => commands.getDailyAccountBreakdown(days).then(unwrap),
  getCacheStatsByAccount: (days: number) => commands.getCacheStatsByAccount(days).then(unwrap),
```

- [ ] **Step 4: Extend the `lib/types.ts` re-export barrel**

`src/lib/types.ts` re-exports a fixed allowlist of names from `generated/bindings.ts` (this is the barrel every existing tab test imports fixture types from, e.g. `ModelsTab.test.tsx`'s `import type { ModelStats, CacheStats } from '../lib/types';`). The new types this plan adds need to be in that same allowlist so later tasks' tests can import them the same way. In `src/lib/types.ts`, change:

```ts
export type {
  AuthSource,
  BurnRateProjection,
  CacheStats,
  CachedUsage,
  DailyBucket,
  DailyModelBucket,
  ExtraBurnRate,
  ExtraUsage,
  LiveSessionInfo,
  ModelStats,
  PricingEntry,
  PricingTier,
  ProjectStats,
  Settings,
  UsageSnapshot,
  Utilization,
} from './generated/bindings';
```

to:

```ts
export type {
  AccountCacheStats,
  AccountStats,
  AuthSource,
  BurnRateProjection,
  CacheStats,
  CachedUsage,
  DailyAccountBucket,
  DailyBucket,
  DailyModelBucket,
  ExtraBurnRate,
  ExtraUsage,
  LiveSessionInfo,
  ModelAccountShare,
  ModelStats,
  PricingEntry,
  PricingTier,
  ProjectStats,
  RepoProjectStats,
  RepoStats,
  Settings,
  UsageSnapshot,
  Utilization,
} from './generated/bindings';
```

- [ ] **Step 5: Confirm both stacks build**

Run: `cd src-tauri && cargo build && cargo test`
Run: `npx tsc --noEmit` (from repo root)
Expected: both succeed with no type errors.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/lib.rs src/lib/generated/bindings.ts src/lib/ipc.ts src/lib/types.ts
git commit -m "feat: register and expose get_daily_account_breakdown, get_cache_stats_by_account"
```

---

### Task 12: Cost tab — per-row account badges

**Files:**
- Modify: `src/report/SessionsTab.tsx`
- Test: `src/report/SessionsTab.test.ts`

**Interfaces:**
- Consumes: `SessionEvent.account_uuid` (Task 4, via regenerated bindings — Task 11), `AccountBadge` (Task 6), `useAppStore((s) => s.accounts)` (existing store slice).
- Produces: `AggregatedSession.account_uuids: (string | null)[]`.

- [ ] **Step 1: Write the failing test**

Find the existing `aggregateSessions` test file (`src/report/SessionsTab.test.ts`) and add a case spanning two accounts in the same day, following that file's existing fixture-building conventions (a `SessionEvent`-shaped object per turn):

```ts
it('collects every distinct account_uuid seen in a row, including null for unattributed turns', () => {
  const events: SessionEvent[] = [
    {
      ts: '2026-08-01T10:00:00Z',
      project: 'p',
      model: 'claude-sonnet-4-6',
      input_tokens: 10,
      output_tokens: 5,
      cache_read_tokens: 0,
      cache_creation_5m_tokens: 0,
      cache_creation_1h_tokens: 0,
      cost_usd: 0.01,
      source_file: 'proj/abc.jsonl',
      source_line: 0,
      event_id: 'e1',
      account_uuid: 'acc1',
    },
    {
      ts: '2026-08-01T11:00:00Z',
      project: 'p',
      model: 'claude-sonnet-4-6',
      input_tokens: 3,
      output_tokens: 1,
      cache_read_tokens: 0,
      cache_creation_5m_tokens: 0,
      cache_creation_1h_tokens: 0,
      cost_usd: 0.002,
      source_file: 'proj/abc.jsonl',
      source_line: 1,
      event_id: 'e2',
      account_uuid: null,
    },
  ];

  const rows = aggregateSessions(events, null);
  expect(rows).toHaveLength(1);
  const uuids = [...rows[0].account_uuids].sort();
  expect(uuids).toEqual([null, 'acc1'].sort());
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npx vitest run src/report/SessionsTab.test.ts`
Expected: FAIL — `rows[0].account_uuids` is `undefined`

- [ ] **Step 3: Extend `ParentAgg` and `AggregatedSession` in `SessionsTab.tsx`**

Add `accountUuids: Set<string | null>;` to `ParentAgg`, initialize it as `accountUuids: new Set(),` in the `if (!p) { ... }` block, and add `p.accountUuids.add(e.account_uuid);` right after the existing `p.modelTokens.set(...)` line in the accumulation loop. Add `account_uuids: (string | null)[];` to the `AggregatedSession` interface, and in the final `result.push({...})` block add `account_uuids: Array.from(p.accountUuids),`.

- [ ] **Step 4: Run the test to verify it passes**

Run: `npx vitest run src/report/SessionsTab.test.ts`
Expected: PASS

- [ ] **Step 5: Render `AccountBadge`s on each row**

In the `SessionsTab()` component, add the accounts selector near the top:

```tsx
  const accounts = useAppStore((s) => s.accounts);
```

In the row markup, immediately after `<ModelBadge model={session.dominant_model} />` (inside the row header's model/compaction-marker group), add:

```tsx
                    {session.account_uuids.map((uuid) => (
                      <AccountBadge key={uuid ?? 'unknown'} accountUuid={uuid} accounts={accounts} />
                    ))}
```

And add the import at the top of the file:

```tsx
import { AccountBadge } from '../components/ui/AccountBadge';
```

- [ ] **Step 6: Run the full frontend test suite**

Run: `npx vitest run`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src/report/SessionsTab.tsx src/report/SessionsTab.test.ts
git commit -m "feat(cost-tab): show which account each session row belongs to"
```

---

### Task 13: Repo tab — account badges per repo/project

**Files:**
- Modify: `src/report/RepoTab.tsx`
- Test: `src/report/RepoTab.test.tsx` (create if it doesn't already exist, following the mocking convention from `ModelsTab.test.tsx`)

**Interfaces:**
- Consumes: `RepoStats.account_uuids` / `RepoProjectStats.account_uuids` (Task 10, via Task 11's regenerated bindings), `AccountBadge` (Task 6).

- [ ] **Step 1: Write the failing test**

Create `src/report/RepoTab.test.tsx` (mirrors `ModelsTab.test.tsx`'s `vi.hoisted`/`vi.mock` pattern exactly, substituting `getRepoBreakdown`):

```tsx
import { render, screen } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { RepoStats } from '../lib/types';

const ipcMock = vi.hoisted(() => ({ getRepoBreakdown: vi.fn() }));
vi.mock('../lib/ipc', () => ({ ipc: ipcMock }));

vi.mock('../lib/store', async () => {
  const actual = await vi.importActual<typeof import('../lib/store')>('../lib/store');
  const state = {
    sessionDataVersion: 0,
    accounts: [
      { slot: 0, email: 'work@x.com', account_uuid: 'acc1', org_name: null, org_uuid: null, subscription_type: null, source: 'OAuth', is_active: true, cached_usage: null, last_error: null },
      { slot: 1, email: 'personal@x.com', account_uuid: 'acc2', org_name: null, org_uuid: null, subscription_type: null, source: 'OAuth', is_active: false, cached_usage: null, last_error: null },
    ],
  };
  const useAppStore: any = (sel: any) => sel(state);
  useAppStore.getState = () => state;
  return { ...actual, useAppStore };
});

import { RepoTab } from './RepoTab';

describe('RepoTab — account attribution', () => {
  beforeEach(() => {
    ipcMock.getRepoBreakdown.mockClear();
  });

  it('shows a badge for each account that touched a repo', async () => {
    const repos: RepoStats[] = [
      {
        repo: 'switchboard',
        session_count: 2,
        total_tokens: 100,
        total_cost_usd: 1.0,
        projects: [{ project: 'switchboard', cwd: '/repo', session_count: 2, total_tokens: 100, total_cost_usd: 1.0, account_uuids: ['acc1', 'acc2'] }],
        account_uuids: ['acc1', 'acc2'],
      },
    ];
    ipcMock.getRepoBreakdown.mockResolvedValue(repos);

    render(<RepoTab />);

    await screen.findByText('switchboard');
    expect(screen.getByText('work')).toBeInTheDocument();
    expect(screen.getByText('personal')).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npx vitest run src/report/RepoTab.test.tsx`
Expected: FAIL — no "work"/"personal" text found

- [ ] **Step 3: Render `AccountBadge`s on each repo card**

In `src/report/RepoTab.tsx`, add the accounts selector and import:

```tsx
import { AccountBadge } from '../components/ui/AccountBadge';
```

```tsx
  const accounts = useAppStore((s) => s.accounts);
```

In the repo header row, immediately after the existing `<span className="shrink-0 text-[length:var(--text-micro)] ...">{repo.session_count} session...</span>` block, add:

```tsx
                  <div className="flex shrink-0 gap-[var(--space-2xs)]">
                    {repo.account_uuids.map((uuid) => (
                      <AccountBadge key={uuid} accountUuid={uuid} accounts={accounts} />
                    ))}
                  </div>
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npx vitest run src/report/RepoTab.test.tsx`
Expected: PASS

- [ ] **Step 5: Run the full frontend test suite**

Run: `npx vitest run`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/report/RepoTab.tsx src/report/RepoTab.test.tsx
git commit -m "feat(repo-tab): badge each repo/project with the accounts that worked in it"
```

---

### Task 14: Models tab — per-account split bar under each model row

**Files:**
- Modify: `src/report/ModelsTab.tsx`
- Test: `src/report/ModelsTab.test.tsx`

**Interfaces:**
- Consumes: `ModelStats.by_account` (Task 7, via Task 11's regenerated bindings), `colorForAccount`/`labelForAccount` (Task 6).

- [ ] **Step 1: Write the failing test**

Add to `src/report/ModelsTab.test.tsx` (same `ipcMock`/store-stub pattern already in the file; extend the stubbed store state to include `accounts`):

```tsx
it('shows a per-account split bar under a model that multiple accounts contributed to', async () => {
  const models: ModelStats[] = [
    {
      model: 'claude-sonnet-4-6',
      input_tokens: 100,
      output_tokens: 20,
      cache_read_tokens: 0,
      cache_creation_tokens: 0,
      cost_usd: 1.0,
      by_account: [
        { account_uuid: 'acc1', input_tokens: 80, output_tokens: 15, cost_usd: 0.8 },
        { account_uuid: 'acc2', input_tokens: 20, output_tokens: 5, cost_usd: 0.2 },
      ],
    },
  ];
  ipcMock.getModelBreakdown.mockResolvedValue(models);

  render(<ModelsTab />);

  await screen.findByText('sonnet 4.6');
  expect(screen.getByText('work')).toBeInTheDocument();
  expect(screen.getByText('personal')).toBeInTheDocument();
});
```

Also update the `vi.mock('../lib/store', ...)` stub at the top of the file to include an `accounts` array in its `state` literal (two entries: `{ slot: 0, email: 'work@x.com', account_uuid: 'acc1', ... }`, `{ slot: 1, email: 'personal@x.com', account_uuid: 'acc2', ... }`, all other `AccountListEntry` fields `null`/`false`/`'OAuth'` as in Task 13's test), and add `by_account: []` to every pre-existing `ModelStats` fixture literal in the file so those tests still type-check.

- [ ] **Step 2: Run the test to verify it fails**

Run: `npx vitest run src/report/ModelsTab.test.tsx`
Expected: FAIL — no "work"/"personal" text found

- [ ] **Step 3: Render the per-account split bar**

In `src/report/ModelsTab.tsx`, add the accounts selector and import:

```tsx
import { AccountBadge } from '../components/ui/AccountBadge';
import { colorForAccount } from './accountDisplay';
```

```tsx
  const accounts = useAppStore((s) => s.accounts);
```

Inside the model list `<Card>` for each `seg`, after the existing cost/percent row, add a thin split bar sized by each account's share of that model's tokens:

```tsx
              {seg.by_account.length > 1 && (
                <div className="flex items-center gap-[var(--space-2xs)] mt-[var(--space-2xs)] pl-[calc(24px+var(--space-sm))]">
                  <div className="flex-1 h-[4px] rounded-[var(--radius-pill)] overflow-hidden flex">
                    {seg.by_account.map((share) => {
                      const shareTotal = share.input_tokens + share.output_tokens;
                      const widthPct = seg.total > 0 ? (shareTotal / seg.total) * 100 : 0;
                      return (
                        <div
                          key={share.account_uuid ?? 'unknown'}
                          style={{ width: `${widthPct}%`, background: colorForAccount(share.account_uuid, accounts) }}
                        />
                      );
                    })}
                  </div>
                  <div className="flex gap-[4px]">
                    {seg.by_account.map((share) => (
                      <AccountBadge key={share.account_uuid ?? 'unknown'} accountUuid={share.account_uuid} accounts={accounts} />
                    ))}
                  </div>
                </div>
              )}
```

(`seg.total` already exists on each `segments` entry, computed as `m.input_tokens + m.output_tokens` in the existing `segments` `useMemo`.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `npx vitest run src/report/ModelsTab.test.tsx`
Expected: PASS

- [ ] **Step 5: Run the full frontend test suite**

Run: `npx vitest run`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/report/ModelsTab.tsx src/report/ModelsTab.test.tsx
git commit -m "feat(models-tab): show per-account split under each model row"
```

---

### Task 15: Trends tab — "Color by: Model / Account" toggle

**Files:**
- Modify: `src/report/TrendsTab.tsx`
- Test: `src/report/TrendsTab.test.tsx`

**Interfaces:**
- Consumes: `ipc.getDailyAccountBreakdown` (Task 11), `AccountStats`/`DailyAccountBucket` (Task 8), `colorForAccount`/`labelForAccount` (Task 6).

- [ ] **Step 1: Write the failing test**

Add to `src/report/TrendsTab.test.tsx` (extend the file's existing `ipcMock`/store-stub the same way as Task 14 — add `getDailyAccountBreakdown: vi.fn()` to the hoisted mock object, and `accounts` to the stubbed store state):

```tsx
it('colors day bars by account when the Account toggle is selected', async () => {
  ipcMock.getDailyTrends.mockResolvedValue([
    { date: '2026-08-01', input_tokens: 100, output_tokens: 20, cost_usd: 1.0, request_count: 3 },
  ]);
  ipcMock.getDailyModelBreakdown.mockResolvedValue([]);
  ipcMock.getDailyAccountBreakdown.mockResolvedValue([
    {
      date: '2026-08-01',
      accounts: [
        { account_uuid: 'acc1', input_tokens: 80, output_tokens: 15, cost_usd: 0.8 },
        { account_uuid: 'acc2', input_tokens: 20, output_tokens: 5, cost_usd: 0.2 },
      ],
    },
  ]);

  render(<TrendsTab />);
  await screen.findByTestId('day-bar-2026-08-01');

  fireEvent.click(screen.getByRole('button', { name: 'Account' }));

  expect(screen.getByTestId('day-bar-2026-08-01-acc1')).toBeInTheDocument();
  expect(screen.getByTestId('day-bar-2026-08-01-acc2')).toBeInTheDocument();
});
```

Add `import { fireEvent } from '@testing-library/react';` to the file's existing `@testing-library/react` import if not already present.

- [ ] **Step 2: Run the test to verify it fails**

Run: `npx vitest run src/report/TrendsTab.test.tsx`
Expected: FAIL — no "Account" button / no `day-bar-2026-08-01-acc1` element

- [ ] **Step 3: Add the toggle and account-colored segments**

In `src/report/TrendsTab.tsx`:

1. Add the accounts selector, import, and a `colorBy` state:

```tsx
import { AccountBadge } from '../components/ui/AccountBadge';
import { colorForAccount, labelForAccount } from './accountDisplay';
```

```tsx
  const accounts = useAppStore((s) => s.accounts);
  const [colorBy, setColorBy] = useState<'model' | 'account'>('model');
```

2. Fetch the new data alongside the existing two calls:

```tsx
  const { data, error, loading, reload } = useTabData(
    () =>
      Promise.all([
        ipc.getDailyTrends(TRENDS_FETCH_DAYS),
        ipc.getDailyModelBreakdown(30),
        ipc.getDailyAccountBreakdown(30),
      ]).then(([trends, breakdown, accountBreakdown]) => ({ trends, breakdown, accountBreakdown })),
    [version],
  );
```

```tsx
  const accountBreakdown = data?.accountBreakdown ?? null;
  const accountBreakdownByDate = useMemo(
    () => new Map((accountBreakdown ?? []).map((b) => [b.date, b])),
    [accountBreakdown],
  );
```

3. Add the toggle UI next to the existing range selector (near the `{(['7d', '30d'] as const).map(...)}` block):

```tsx
        <div className="flex gap-[var(--space-2xs)] bg-[var(--color-track)] rounded-[var(--radius-sm)] p-[2px] w-fit">
          {(['model', 'account'] as const).map((c) => (
            <button
              key={c}
              type="button"
              onClick={() => setColorBy(c)}
              className={[
                'px-[var(--space-sm)] py-[var(--space-2xs)]',
                'text-[length:var(--text-label)] font-[var(--weight-medium)]',
                'rounded-[var(--radius-sm)]',
                'transition-[background,color] duration-[var(--duration-fast)]',
                colorBy === c
                  ? 'bg-[var(--color-bg-card)] text-[color:var(--color-text)]'
                  : 'text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text-secondary)]',
              ].join(' ')}
            >
              {c === 'model' ? 'Model' : 'Account'}
            </button>
          ))}
        </div>
```

4. In the day-bar rendering, branch the segment source and color on `colorBy`:

```tsx
            const dayModels = breakdownByDate.get(day.date)?.models ?? [];
            const dayAccounts = accountBreakdownByDate.get(day.date)?.accounts ?? [];
            const segmentValues: Record<string, number> = {};
            const segmentColors: Record<string, string> = {};
            if (colorBy === 'model') {
              for (const m of dayModels) {
                const key = modelKey(m.model);
                const v = metric === 'tokens' ? m.input_tokens + m.output_tokens : m.cost_usd;
                segmentValues[key] = (segmentValues[key] ?? 0) + v;
                segmentColors[key] = MODEL_COLORS[key as (typeof MODEL_ORDER)[number]] ?? MODEL_COLORS.default;
              }
            } else {
              for (const a of dayAccounts) {
                const key = a.account_uuid ?? 'unknown';
                const v = metric === 'tokens' ? a.input_tokens + a.output_tokens : a.cost_usd;
                segmentValues[key] = (segmentValues[key] ?? 0) + v;
                segmentColors[key] = colorForAccount(a.account_uuid, accounts);
              }
            }
            const segments = Object.keys(segmentValues).filter((k) => segmentValues[k] > 0);
```

And update the segment-rendering `div` (which currently reads `MODEL_COLORS[key]` directly) to use `segmentColors[key]` instead:

```tsx
                    {segments.map((key) => (
                      <div
                        key={key}
                        data-testid={`day-bar-${day.date}-${key}`}
                        className="w-full"
                        style={{ flexGrow: segmentValues[key], background: segmentColors[key] }}
                      />
                    ))}
```

5. Update the legend block to switch between the existing model legend and an account legend:

```tsx
      {colorBy === 'model' && legendKeys.length > 0 && (
        <div className="flex items-center gap-[var(--space-md)] px-[2px]">
          {legendKeys.map((key) => (
            <span key={key} className="flex items-center gap-[var(--space-2xs)]">
              <span aria-hidden className="w-[8px] h-[8px] rounded-[2px] shrink-0" style={{ background: MODEL_COLORS[key] }} />
              <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">{MODEL_LABELS[key]}</span>
            </span>
          ))}
        </div>
      )}
      {colorBy === 'account' && accounts.length > 0 && (
        <div className="flex items-center gap-[var(--space-md)] px-[2px]">
          {accounts.map((a) => (
            <AccountBadge key={a.account_uuid} accountUuid={a.account_uuid} accounts={accounts} />
          ))}
        </div>
      )}
```

(This replaces the previous unconditional `{legendKeys.length > 0 && (...)}` block — wrap it in the `colorBy === 'model'` check as shown.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `npx vitest run src/report/TrendsTab.test.tsx`
Expected: PASS

- [ ] **Step 5: Run the full frontend test suite**

Run: `npx vitest run`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/report/TrendsTab.tsx src/report/TrendsTab.test.tsx
git commit -m "feat(trends-tab): add color-by-account toggle for the day chart"
```

---

### Task 16: Heatmap tab — dominant-account ring + tooltip split

**Files:**
- Modify: `src/report/HeatmapTab.tsx`
- Test: `src/report/HeatmapTab.test.tsx` (create if it doesn't already exist)

**Interfaces:**
- Consumes: `ipc.getDailyAccountBreakdown` (Task 11), `colorForAccount`/`labelForAccount` (Task 6).

- [ ] **Step 1: Write the failing test**

Create `src/report/HeatmapTab.test.tsx` (same conventions as `RepoTab.test.tsx`/`ModelsTab.test.tsx`):

```tsx
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { SessionEvent, DailyAccountBucket } from '../lib/types';

const ipcMock = vi.hoisted(() => ({
  getSessionHistory: vi.fn(),
  getDailyAccountBreakdown: vi.fn(),
}));
vi.mock('../lib/ipc', () => ({ ipc: ipcMock }));

vi.mock('../lib/store', async () => {
  const actual = await vi.importActual<typeof import('../lib/store')>('../lib/store');
  const state = {
    sessionDataVersion: 0,
    accounts: [
      { slot: 0, email: 'work@x.com', account_uuid: 'acc1', org_name: null, org_uuid: null, subscription_type: null, source: 'OAuth', is_active: true, cached_usage: null, last_error: null },
    ],
  };
  const useAppStore: any = (sel: any) => sel(state);
  useAppStore.getState = () => state;
  return { ...actual, useAppStore };
});

import { HeatmapTab } from './HeatmapTab';

describe('HeatmapTab — account attribution', () => {
  beforeEach(() => {
    ipcMock.getSessionHistory.mockClear();
    ipcMock.getDailyAccountBreakdown.mockClear();
  });

  it('shows the account split for a day in its hover tooltip', async () => {
    const today = new Date().toISOString().slice(0, 10);
    const events: SessionEvent[] = [
      {
        ts: new Date().toISOString(),
        project: 'p',
        model: 'm',
        input_tokens: 10,
        output_tokens: 5,
        cache_read_tokens: 0,
        cache_creation_5m_tokens: 0,
        cache_creation_1h_tokens: 0,
        cost_usd: 0.01,
        source_file: 'a.jsonl',
        source_line: 0,
        event_id: 'e1',
        account_uuid: 'acc1',
      },
    ];
    ipcMock.getSessionHistory.mockResolvedValue(events);
    const accountBuckets: DailyAccountBucket[] = [
      { date: today, accounts: [{ account_uuid: 'acc1', input_tokens: 10, output_tokens: 5, cost_usd: 0.01 }] },
    ];
    ipcMock.getDailyAccountBreakdown.mockResolvedValue(accountBuckets);

    render(<HeatmapTab />);

    const cell = await screen.findByTestId(`heatmap-cell-${today}`);
    fireEvent.mouseEnter(cell);

    expect(screen.getByText(/work 100%/)).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npx vitest run src/report/HeatmapTab.test.tsx`
Expected: FAIL — no element with `data-testid="heatmap-cell-<today>"` (the current `<rect>` has no `data-testid`), and no "work 100%" text.

- [ ] **Step 3: Fetch account breakdown, add a `data-testid`, dominant-account ring, and tooltip split**

In `src/report/HeatmapTab.tsx`:

1. Add the accounts selector, import, and second fetch:

```tsx
import { colorForAccount, labelForAccount } from './accountDisplay';
import type { DailyAccountBucket } from '../lib/types';
```

```tsx
  const accounts = useAppStore((s) => s.accounts);
  const { data: events, error, loading, reload } = useTabData(
    () => ipc.getSessionHistory(180),
    [version],
  );
  const { data: accountBuckets } = useTabData(
    () => ipc.getDailyAccountBreakdown(180),
    [version],
  );
  const accountsByDate = useMemo(
    () => new Map((accountBuckets ?? []).map((b: DailyAccountBucket) => [b.date, b.accounts])),
    [accountBuckets],
  );
```

2. Compute each cell's dominant account alongside its existing `level`, and give the `<rect>` a `data-testid`:

```tsx
                <rect
                  data-testid={`heatmap-cell-${cell.date}`}
                  x={x}
                  y={y}
```

Add a small ring in the dominant account's color, right after the existing `<rect>`:

```tsx
                {(() => {
                  const dayAccounts = accountsByDate.get(cell.date) ?? [];
                  if (dayAccounts.length === 0) return null;
                  const dominant = [...dayAccounts].sort(
                    (a, b) => (b.input_tokens + b.output_tokens) - (a.input_tokens + a.output_tokens),
                  )[0];
                  return (
                    <rect
                      x={x - 1}
                      y={y - 1}
                      width={CELL_SIZE + 2}
                      height={CELL_SIZE + 2}
                      rx={2}
                      fill="none"
                      stroke={colorForAccount(dominant.account_uuid, accounts)}
                      strokeWidth={1}
                      pointerEvents="none"
                    />
                  );
                })()}
```

3. Extend the hover tooltip to list the account split:

```tsx
                    <text
                      x={x + CELL_SIZE / 2}
                      y={y - 6}
                      textAnchor="middle"
                      className="mono"
                      style={{ fontSize: 9, fill: 'var(--color-text-secondary)' }}
                    >
                      {new Date(cell.date).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}
                    </text>
                    {(() => {
                      const dayAccounts = accountsByDate.get(cell.date) ?? [];
                      const total = dayAccounts.reduce((s, a) => s + a.input_tokens + a.output_tokens, 0);
                      if (total === 0) return null;
                      const parts = dayAccounts
                        .map((a) => `${labelForAccount(a.account_uuid, accounts)} ${Math.round(((a.input_tokens + a.output_tokens) / total) * 100)}%`)
                        .join(' · ');
                      return (
                        <text
                          x={x + CELL_SIZE / 2}
                          y={y - 16}
                          textAnchor="middle"
                          className="mono"
                          style={{ fontSize: 8, fill: 'var(--color-text-muted)' }}
                        >
                          {parts}
                        </text>
                      );
                    })()}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npx vitest run src/report/HeatmapTab.test.tsx`
Expected: PASS

- [ ] **Step 5: Run the full frontend test suite**

Run: `npx vitest run`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/report/HeatmapTab.tsx src/report/HeatmapTab.test.tsx
git commit -m "feat(heatmap-tab): show dominant account per day, full split on hover"
```

---

### Task 17: Cache tab — per-account cards

**Files:**
- Modify: `src/report/CacheTab.tsx`
- Test: `src/report/CacheTab.test.tsx` (create if it doesn't already exist)

**Interfaces:**
- Consumes: `ipc.getCacheStatsByAccount` (Task 11), `AccountCacheStats` (Task 9), `AccountBadge`/`colorForAccount`/`labelForAccount` (Task 6).

- [ ] **Step 1: Write the failing test**

Create `src/report/CacheTab.test.tsx`:

```tsx
import { render, screen } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { CacheStats, AccountCacheStats } from '../lib/types';

const ipcMock = vi.hoisted(() => ({
  getCacheStats: vi.fn(),
  getCacheStatsByAccount: vi.fn(),
}));
vi.mock('../lib/ipc', () => ({ ipc: ipcMock }));

vi.mock('../lib/store', async () => {
  const actual = await vi.importActual<typeof import('../lib/store')>('../lib/store');
  const state = {
    sessionDataVersion: 0,
    accounts: [
      { slot: 0, email: 'work@x.com', account_uuid: 'acc1', org_name: null, org_uuid: null, subscription_type: null, source: 'OAuth', is_active: true, cached_usage: null, last_error: null },
      { slot: 1, email: 'personal@x.com', account_uuid: 'acc2', org_name: null, org_uuid: null, subscription_type: null, source: 'OAuth', is_active: false, cached_usage: null, last_error: null },
    ],
  };
  const useAppStore: any = (sel: any) => sel(state);
  useAppStore.getState = () => state;
  return { ...actual, useAppStore };
});

import { CacheTab } from './CacheTab';

describe('CacheTab — per-account cards', () => {
  beforeEach(() => {
    ipcMock.getCacheStats.mockClear();
    ipcMock.getCacheStatsByAccount.mockClear();
  });

  it('shows one card per account alongside the total', async () => {
    const total: CacheStats = {
      total_cache_read_tokens: 110,
      total_cache_creation_tokens: 20,
      estimated_savings_usd: 1.5,
      hit_ratio: 0.85,
    };
    const byAccount: AccountCacheStats[] = [
      { account_uuid: 'acc1', total_cache_read_tokens: 100, total_cache_creation_tokens: 15, estimated_savings_usd: 1.3, hit_ratio: 0.87 },
      { account_uuid: 'acc2', total_cache_read_tokens: 10, total_cache_creation_tokens: 5, estimated_savings_usd: 0.2, hit_ratio: 0.67 },
    ];
    ipcMock.getCacheStats.mockResolvedValue(total);
    ipcMock.getCacheStatsByAccount.mockResolvedValue(byAccount);

    render(<CacheTab />);

    await screen.findByText('Total');
    expect(screen.getByText('work')).toBeInTheDocument();
    expect(screen.getByText('personal')).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `npx vitest run src/report/CacheTab.test.tsx`
Expected: FAIL — no "Total"/"work"/"personal" text (component doesn't fetch or render per-account data yet)

- [ ] **Step 3: Restructure `CacheTab` around per-account cards**

In `src/report/CacheTab.tsx`, add the second fetch and accounts selector:

```tsx
import { AccountBadge } from '../components/ui/AccountBadge';
import { colorForAccount } from './accountDisplay';
```

```tsx
export function CacheTab() {
  const version = useAppStore((s) => s.sessionDataVersion);
  const accounts = useAppStore((s) => s.accounts);
  const { data, error, loading, reload } = useTabData(
    () => ipc.getCacheStats(30),
    [version],
  );
  const { data: byAccount } = useTabData(
    () => ipc.getCacheStatsByAccount(30),
    [version],
  );
```

After the existing "Hero: cache hit rate ring" block (keep it as-is, but relabel its wrapping context as the totals summary — add a "Total" label above it):

```tsx
      <div className="flex items-center justify-between px-[var(--space-2xs)]">
        <span className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text-muted)]">
          Total
        </span>
      </div>
```

Then, after the existing "Breakdown bar" `<Card>` block (the last element in the current return), append the per-account cards:

```tsx
      {byAccount && byAccount.length > 1 && (
        <div className="flex flex-col gap-[var(--space-sm)]">
          <span className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text-muted)]">
            By account
          </span>
          {byAccount.map((a) => {
            const total = a.total_cache_read_tokens + a.total_cache_creation_tokens;
            if (total === 0) return null;
            return (
              <Card key={a.account_uuid ?? 'unknown'} className="p-[var(--space-sm)] flex items-center gap-[var(--space-sm)]">
                <AccountBadge accountUuid={a.account_uuid} accounts={accounts} />
                <div className="flex-1 h-[6px] rounded-[var(--radius-pill)] bg-[var(--color-track)] overflow-hidden">
                  <div
                    className="h-full rounded-[var(--radius-pill)]"
                    style={{ width: `${a.hit_ratio * 100}%`, background: colorForAccount(a.account_uuid, accounts) }}
                  />
                </div>
                <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)] tabular-nums min-w-[40px] text-right">
                  {Math.round(a.hit_ratio * 100)}%
                </span>
                <span className="mono text-[length:var(--text-label)] text-[color:var(--color-safe)] tabular-nums min-w-[56px] text-right">
                  ${a.estimated_savings_usd.toFixed(2)}
                </span>
              </Card>
            );
          })}
        </div>
      )}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `npx vitest run src/report/CacheTab.test.tsx`
Expected: PASS

- [ ] **Step 5: Run the full frontend test suite**

Run: `npx vitest run`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src/report/CacheTab.tsx src/report/CacheTab.test.tsx
git commit -m "feat(cache-tab): add per-account cards alongside the total"
```

---

## Final verification (manual)

After all 17 tasks are complete:

1. Run the full backend suite: `cd src-tauri && cargo test`
2. Run the full frontend suite: `npx vitest run`
3. Run `npx tsc --noEmit` from the repo root
4. Launch the app (`cd src-tauri && cargo run` or `pnpm tauri dev`) with at least two managed accounts. Swap between them via the Accounts sidebar, generate some usage under each, and confirm:
   - Cost tab rows show the right account badge.
   - Repo tab cards show badges for every account that's touched that repo.
   - Models tab rows show a split bar when a model was used under more than one account.
   - Trends tab's "Color by: Account" toggle recolors the day bars correctly.
   - Heatmap tab cells show the right dominant-account ring, and hovering shows the correct split.
   - Cache tab shows a card per account alongside the total.
5. Swap accounts via `claude login` in a terminal (bypassing Switchboard's own swap button) and confirm the poll loop picks up the change and attributes subsequent activity correctly within ~60 seconds.
