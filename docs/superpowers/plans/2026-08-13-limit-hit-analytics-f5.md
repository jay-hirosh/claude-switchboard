# F5 — Limit-hit analytics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Track, per managed account, the peak utilization reached in every finished 5H/7D rate-limit window, and surface a report answering "when do I actually run out?" — hit counts, hour-of-day distribution, and the projects that drove consumption beforehand.

**Architecture:** A new `window_peaks` SQLite table, upserted incrementally from the existing poll loop (one row per finished window, keyed by `(account_id, bucket, resets_at)` — a window's own `resets_at` is its identity, so a new value naturally starts a new row with no explicit rollover-detection code). A new read-only Tauri command aggregates it server-side and joins the existing `session_events` table for project attribution. A new frontend report tab renders it.

**Tech Stack:** Rust (rusqlite, tauri-specta), React 19 + TypeScript, Vitest + Testing Library.

**Spec:** `docs/superpowers/specs/2026-08-13-limit-hit-analytics-f5-design.md` — read it first; this plan implements it task-by-task.

## Global Constraints

- No change to `api_snapshots`, its prune policy, or the cold-start rehydration path — `window_peaks` is fully additive.
- Scoped to the two headline buckets only: `five_hour` and `seven_day`. No per-model (Opus/Sonnet) peak tracking.
- The "hit" threshold reuses the app's existing configurable danger threshold (`Settings.thresholds[1]`, default 90) — never hardcode 95.
- All DB writes on the poll-loop path are best-effort: log a `tracing::warn!` and never interrupt polling, matching the existing `insert_snapshot` call right next to where this wires in.
- The report covers every managed account, not just the active one.

---

## Task 1: `window_peaks` table — schema and migration

**Files:**
- Modify: `src-tauri/src/store/schema.sql`
- Create: `src-tauri/src/store/migrations/0010_window_peaks.sql`
- Modify: `src-tauri/src/store/mod.rs` (migrate() function, `create_fresh_db`'s stamped version, final schema_version insert)
- Test: `src-tauri/src/store/mod.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: table `window_peaks(id, account_id, bucket, resets_at, window_start, peak_pct, peak_at)` with `UNIQUE(account_id, bucket, resets_at)` — Task 2 upserts/reads against this exact shape.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/store/mod.rs` (place it near `migration_0009_adds_compactions_table_and_reingests`, same style):

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test migration_0010_adds_window_peaks_table`
Expected: FAIL — `no such table: window_peaks` (schema.sql doesn't define it yet) or the `DROP TABLE` line fails first with the same error.

- [ ] **Step 3: Add the table to `schema.sql`** (fresh installs)

In `src-tauri/src/store/schema.sql`, after the `session_compactions` block (after its `CREATE INDEX idx_compactions_ts` line), add:

```sql
-- One row per finished 5H/7D rate-limit window per account, tracking the
-- peak utilization reached and when. `(account_id, bucket, resets_at)` is
-- the window's identity: a poll reporting a new resets_at for a bucket
-- naturally creates a new row via UPSERT (see Db::record_window_peak),
-- so a window "rollover" needs no explicit detection code. Feeds the
-- limit-hit analytics report (F5) — starts empty, builds forward from
-- whenever this ships (api_snapshots' 50-row cap means there's nothing
-- to backfill from).
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
```

- [ ] **Step 4: Create the migration file** (existing installs)

Create `src-tauri/src/store/migrations/0010_window_peaks.sql`:

```sql
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
```

- [ ] **Step 5: Wire the migration into `migrate()`**

In `src-tauri/src/store/mod.rs`, inside `fn migrate()`, after the `if current < 9 { ... }` block, add:

```rust
        if current < 10 {
            tracing::info!("migrating v9 -> v10 (window_peaks for limit-hit analytics)");
            conn.execute_batch(include_str!("migrations/0010_window_peaks.sql"))
                .context("apply migration 0010")?;
        }
```

Then update the two version-stamping sites so fresh installs and upgraded installs both land on v10:
- In `create_fresh_db`: change `[9_i64]` to `[10_i64]` and its doc comment's "schema_version=9" to "schema_version=10".
- At the end of `migrate()`: change `[9_i64]` to `[10_i64]`.

- [ ] **Step 6: Run test to verify it passes**

Run: `cd src-tauri && cargo test migration_0010_adds_window_peaks_table`
Expected: PASS

- [ ] **Step 7: Run the full backend test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS — confirms the version-stamp bump didn't break any of the other migration tests (e.g. `migration_0009_...`, which opens its own fresh v8 DB and doesn't depend on the final stamped version).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/store/schema.sql src-tauri/src/store/migrations/0010_window_peaks.sql src-tauri/src/store/mod.rs
git commit -m "feat(db): add window_peaks table for limit-hit analytics (F5)"
```

---

## Task 2: `record_window_peak` upsert + `window_peaks_between` read

**Files:**
- Modify: `src-tauri/src/store/queries.rs`

**Interfaces:**
- Consumes: `window_peaks` table from Task 1.
- Produces: `Db::record_window_peak(account_id: &str, bucket: &str, resets_at: DateTime<Utc>, observed_at: DateTime<Utc>, pct: f64) -> Result<()>` (Task 4 calls this from the poll loop), `Db::window_peaks_between(account_id: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<WindowPeak>>` and the `WindowPeak` struct (Task 3 consumes both).

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block at the bottom of `src-tauri/src/store/queries.rs` (uses the existing `fresh_db()` helper already in that module):

```rust
    #[test]
    fn record_window_peak_tracks_the_max_within_one_window() {
        let (_dir, db) = fresh_db();
        let resets_at = Utc::now() + chrono::Duration::hours(3);
        let t1 = Utc::now();
        let t2 = t1 + chrono::Duration::minutes(5);
        let t3 = t1 + chrono::Duration::minutes(10);

        db.record_window_peak("acc1", "five_hour", resets_at, t1, 40.0).unwrap();
        db.record_window_peak("acc1", "five_hour", resets_at, t2, 82.0).unwrap();
        // A later, lower reading must not overwrite the peak.
        db.record_window_peak("acc1", "five_hour", resets_at, t3, 60.0).unwrap();

        let peaks = db
            .window_peaks_between("acc1", t1 - chrono::Duration::minutes(1), t3 + chrono::Duration::minutes(1))
            .unwrap();
        assert_eq!(peaks.len(), 1);
        assert_eq!(peaks[0].peak_pct, 82.0);
        assert_eq!(peaks[0].peak_at.timestamp(), t2.timestamp());
        assert_eq!(peaks[0].window_start.timestamp(), t1.timestamp());
    }

    #[test]
    fn record_window_peak_starts_a_new_row_when_resets_at_changes() {
        let (_dir, db) = fresh_db();
        let window1_resets = Utc::now() + chrono::Duration::hours(1);
        let window2_resets = Utc::now() + chrono::Duration::hours(6);
        let now = Utc::now();

        db.record_window_peak("acc1", "five_hour", window1_resets, now, 95.0).unwrap();
        db.record_window_peak("acc1", "five_hour", window2_resets, now, 10.0).unwrap();

        let peaks = db
            .window_peaks_between("acc1", now - chrono::Duration::minutes(1), now + chrono::Duration::minutes(1))
            .unwrap();
        assert_eq!(peaks.len(), 2, "a new resets_at must create a new row, not overwrite the old one");
    }

    #[test]
    fn window_peaks_between_scopes_to_the_requested_account() {
        let (_dir, db) = fresh_db();
        db.upsert_account(&StoredAccount { id: "acc2".into(), email: "b@example.com".into(), display_name: None }).unwrap();
        let resets_at = Utc::now() + chrono::Duration::hours(1);
        let now = Utc::now();

        db.record_window_peak("acc1", "five_hour", resets_at, now, 91.0).unwrap();
        db.record_window_peak("acc2", "five_hour", resets_at, now, 91.0).unwrap();

        let peaks = db
            .window_peaks_between("acc1", now - chrono::Duration::minutes(1), now + chrono::Duration::minutes(1))
            .unwrap();
        assert_eq!(peaks.len(), 1);
        assert_eq!(peaks[0].bucket, "five_hour");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test record_window_peak && cargo test window_peaks_between`
Expected: FAIL to compile — `record_window_peak` and `window_peaks_between` don't exist yet.

- [ ] **Step 3: Write the minimal implementation**

Add to `src-tauri/src/store/queries.rs`, near `StoredCompaction` (same struct-definition area, top of the file):

```rust
/// A finished (or in-progress) 5H/7D window's peak utilization, as tracked
/// incrementally by `record_window_peak`. `(account_id, bucket, resets_at)`
/// is the window's identity.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct WindowPeak {
    pub bucket: String,
    #[specta(type = String)]
    pub resets_at: DateTime<Utc>,
    #[specta(type = String)]
    pub window_start: DateTime<Utc>,
    pub peak_pct: f64,
    #[specta(type = String)]
    pub peak_at: DateTime<Utc>,
}
```

Add to the `impl Db` block (near `record_notification_fired`, since it's the closest existing precedent for this exact upsert shape):

```rust
    /// Record one poll's reading for a window, keeping the running peak.
    /// `(account_id, bucket, resets_at)` identifies the window — a new
    /// `resets_at` value creates a fresh row rather than updating the old
    /// one, which is how a window rollover gets detected with no explicit
    /// bookkeeping. Best-effort from the caller's perspective: errors
    /// propagate but callers on the poll-loop path treat them as
    /// log-and-continue (see poll_loop.rs).
    pub fn record_window_peak(
        &self,
        account_id: &str,
        bucket: &str,
        resets_at: DateTime<Utc>,
        observed_at: DateTime<Utc>,
        pct: f64,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO window_peaks (account_id, bucket, resets_at, window_start, peak_pct, peak_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?4)
             ON CONFLICT(account_id, bucket, resets_at) DO UPDATE SET
                 peak_pct = MAX(peak_pct, excluded.peak_pct),
                 peak_at = CASE WHEN excluded.peak_pct > peak_pct THEN excluded.peak_at ELSE peak_at END,
                 window_start = MIN(window_start, excluded.window_start)",
            params![account_id, bucket, resets_at.timestamp(), observed_at.timestamp(), pct],
        )?;
        Ok(())
    }

    /// All window peaks for one account whose `window_start` falls in
    /// `[from, to]`, oldest first.
    pub fn window_peaks_between(
        &self,
        account_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<WindowPeak>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT bucket, resets_at, window_start, peak_pct, peak_at
             FROM window_peaks
             WHERE account_id = ?1 AND window_start BETWEEN ?2 AND ?3
             ORDER BY peak_at ASC",
        )?;
        let rows = stmt.query_map(params![account_id, from.timestamp(), to.timestamp()], |r| {
            Ok(WindowPeak {
                bucket: r.get(0)?,
                resets_at: DateTime::from_timestamp(r.get(1)?, 0).unwrap(),
                window_start: DateTime::from_timestamp(r.get(2)?, 0).unwrap(),
                peak_pct: r.get(3)?,
                peak_at: DateTime::from_timestamp(r.get(4)?, 0).unwrap(),
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test record_window_peak && cargo test window_peaks_between`
Expected: PASS (all 3 new tests)

- [ ] **Step 5: Run the full backend test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/store/queries.rs
git commit -m "feat(db): add record_window_peak upsert and window_peaks_between read"
```

---

## Task 3: `limit_hit_stats` aggregation (hit counts, hourly distribution, project attribution)

**Files:**
- Modify: `src-tauri/src/store/queries.rs`

**Interfaces:**
- Consumes: `Db::window_peaks_between` and the existing `Db::events_between(from, to) -> Result<Vec<StoredSessionEvent>>` (already in this file — reused as-is, no new session_events query needed).
- Produces: `Db::limit_hit_stats(account_id: &str, email: &str, from: DateTime<Utc>, to: DateTime<Utc>, danger_threshold: f64) -> Result<AccountLimitHits>`, and structs `AccountLimitHits { account_id: String, email: String, five_hour_hits: u32, seven_day_hits: u32, hourly_distribution: Vec<u32>, top_projects: Vec<ProjectAttribution> }` / `ProjectAttribution { project: String, cost_usd: f64 }`. Task 5's `get_limit_hit_history` command calls this once per managed account.

- [ ] **Step 1: Write the failing test**

Add to `queries.rs`'s test module:

```rust
    #[test]
    fn limit_hit_stats_counts_hits_and_attributes_projects() {
        let (_dir, db) = fresh_db();
        let resets_at = Utc::now() + chrono::Duration::hours(2);
        let window_start = Utc::now() - chrono::Duration::hours(3);
        let peak_at = window_start + chrono::Duration::hours(1);

        // A hit (>= 90 threshold) and a miss (< 90) — only the hit should count.
        db.record_window_peak("acc1", "five_hour", resets_at, peak_at, 95.0).unwrap();
        db.record_window_peak("acc1", "seven_day", resets_at, peak_at, 50.0).unwrap();

        // One event inside the hit window, one outside it — only the inside one attributes.
        db.insert_events(&[StoredSessionEvent {
            ts: window_start + chrono::Duration::minutes(30),
            project: "switchboard".into(),
            model: "claude-sonnet-4-6".into(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 1.23,
            source_file: "/a.jsonl".into(),
            source_line: 1,
            event_id: "evt-in".into(),
        }])
        .unwrap();
        db.insert_events(&[StoredSessionEvent {
            ts: peak_at + chrono::Duration::hours(5),
            project: "other-project".into(),
            model: "claude-sonnet-4-6".into(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 9.99,
            source_file: "/b.jsonl".into(),
            source_line: 1,
            event_id: "evt-out".into(),
        }])
        .unwrap();

        let stats = db
            .limit_hit_stats("acc1", "a@example.com", Utc::now() - chrono::Duration::hours(4), Utc::now() + chrono::Duration::hours(4), 90.0)
            .unwrap();

        assert_eq!(stats.five_hour_hits, 1);
        assert_eq!(stats.seven_day_hits, 0);
        assert_eq!(stats.hourly_distribution.len(), 24);
        assert_eq!(stats.hourly_distribution.iter().sum::<u32>(), 1);
        assert_eq!(stats.top_projects.len(), 1);
        assert_eq!(stats.top_projects[0].project, "switchboard");
        assert_eq!(stats.top_projects[0].cost_usd, 1.23);
    }

    #[test]
    fn limit_hit_stats_returns_zeroed_struct_for_an_account_with_no_history() {
        let (_dir, db) = fresh_db();
        let stats = db
            .limit_hit_stats("acc1", "a@example.com", Utc::now() - chrono::Duration::days(30), Utc::now(), 90.0)
            .unwrap();
        assert_eq!(stats.five_hour_hits, 0);
        assert_eq!(stats.seven_day_hits, 0);
        assert!(stats.top_projects.is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test limit_hit_stats`
Expected: FAIL to compile — `limit_hit_stats` doesn't exist yet.

- [ ] **Step 3: Write the minimal implementation**

Add near `WindowPeak` (struct definitions area):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ProjectAttribution {
    pub project: String,
    pub cost_usd: f64,
}

/// One managed account's limit-hit history over a report window. `hits`
/// are windows whose peak_pct cleared the caller-supplied danger threshold
/// — a non-hit window still exists in `window_peaks` but never appears
/// here (changing the threshold in Settings retroactively reclassifies
/// history at read time; nothing is filtered at write time).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct AccountLimitHits {
    pub account_id: String,
    pub email: String,
    pub five_hour_hits: u32,
    pub seven_day_hits: u32,
    /// Always length 24; index = local hour (0-23) a hit's peak was observed.
    pub hourly_distribution: Vec<u32>,
    /// Top 5 by summed cost, across every hit window's [window_start, peak_at] span.
    pub top_projects: Vec<ProjectAttribution>,
}
```

Add `use chrono::Timelike;` to the top-of-file imports (needed for `.hour()` below) — change:
```rust
use chrono::{DateTime, Utc};
```
to:
```rust
use chrono::{DateTime, Timelike, Utc};
```

Add to `impl Db` (near `window_peaks_between`):

```rust
    /// Aggregate one account's limit-hit history: hit counts per bucket,
    /// an hour-of-day distribution of when hits peaked, and the projects
    /// that consumed each hit window beforehand. Read-only, computed
    /// entirely from already-stored data — reuses `events_between` rather
    /// than adding a new session_events query.
    pub fn limit_hit_stats(
        &self,
        account_id: &str,
        email: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        danger_threshold: f64,
    ) -> Result<AccountLimitHits> {
        let peaks = self.window_peaks_between(account_id, from, to)?;
        let mut five_hour_hits = 0u32;
        let mut seven_day_hits = 0u32;
        let mut hourly_distribution = vec![0u32; 24];
        let mut project_totals: std::collections::HashMap<String, f64> =
            std::collections::HashMap::new();

        for peak in &peaks {
            if peak.peak_pct < danger_threshold {
                continue;
            }
            match peak.bucket.as_str() {
                "five_hour" => five_hour_hits += 1,
                "seven_day" => seven_day_hits += 1,
                _ => {}
            }
            let hour = peak.peak_at.with_timezone(&chrono::Local).hour() as usize;
            hourly_distribution[hour] += 1;

            for event in self.events_between(peak.window_start, peak.peak_at)? {
                *project_totals.entry(event.project).or_insert(0.0) += event.cost_usd;
            }
        }

        let mut top_projects: Vec<ProjectAttribution> = project_totals
            .into_iter()
            .map(|(project, cost_usd)| ProjectAttribution { project, cost_usd })
            .collect();
        top_projects.sort_by(|a, b| b.cost_usd.partial_cmp(&a.cost_usd).unwrap());
        top_projects.truncate(5);

        Ok(AccountLimitHits {
            account_id: account_id.to_string(),
            email: email.to_string(),
            five_hour_hits,
            seven_day_hits,
            hourly_distribution,
            top_projects,
        })
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test limit_hit_stats`
Expected: PASS (both tests)

- [ ] **Step 5: Run the full backend test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/store/queries.rs
git commit -m "feat(db): add limit_hit_stats aggregation for F5"
```

---

## Task 4: Wire the upsert into the poll loop

**Files:**
- Modify: `src-tauri/src/poll_loop.rs`

**Interfaces:**
- Consumes: `Db::record_window_peak` (Task 2), `crate::notifier::rules::Bucket` (existing — `Bucket::FiveHour.label()` / `Bucket::SevenDay.label()` give the exact `"five_hour"` / `"seven_day"` strings `window_peaks.bucket` stores).

- [ ] **Step 1: Add the import**

In `src-tauri/src/poll_loop.rs`, add to the imports:

```rust
use crate::notifier::rules::Bucket;
```

- [ ] **Step 2: Add the write, right after the existing `insert_snapshot` call**

In `apply_fetch_outcome`'s `FetchOutcome::Ok(snapshot) => { ... }` arm, immediately after the existing block that does `state.db.insert_snapshot(...)` (the one ending with `Err(e) => tracing::warn!("serialize snapshot for slot {slot} failed: {e}"),`) and before the `let _ = handle.emit("usage_updated", ...)` line, add:

```rust
            // Track per-window peaks for the limit-hit analytics report
            // (F5). Runs for every slot, not just the active one — the
            // report covers every managed account. Best-effort, same
            // rationale as insert_snapshot above: a storage hiccup must
            // never interrupt polling.
            for (bucket, data) in [
                (Bucket::FiveHour, snapshot.five_hour.as_ref()),
                (Bucket::SevenDay, snapshot.seven_day.as_ref()),
            ] {
                let Some(u) = data else { continue };
                let Some(resets_at) = u.resets_at else { continue };
                if let Err(e) = state.db.record_window_peak(
                    &acc.account_uuid,
                    bucket.label(),
                    resets_at,
                    Utc::now(),
                    u.utilization,
                ) {
                    tracing::warn!("record_window_peak for slot {slot} ({}) failed: {e}", bucket.label());
                }
            }
```

Note: this task intentionally has no new automated test. `apply_fetch_outcome` takes `&AppHandle`, and nothing in this codebase constructs one outside a running Tauri app (confirmed: no existing test exercises `apply_fetch_outcome` directly — the `hydrate` test module in this same file tests `hydrated_caches` instead, a plain function with no Tauri dependency). The upsert's actual behavior is already proven by Task 2's tests; this step is wiring only. Correctness here is a code-review concern (right fields, right placement, right bucket labels), not a new-test concern.

- [ ] **Step 3: Verify the crate builds and the full suite still passes**

Run: `cd src-tauri && cargo build && cargo test`
Expected: builds clean, all tests PASS (no new failures introduced by this wiring).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/poll_loop.rs
git commit -m "feat(poll-loop): record window peaks for limit-hit analytics on every poll"
```

---

## Task 5: `get_limit_hit_history` command, registration, and bindings

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs` (both `collect_commands!` blocks)

**Interfaces:**
- Consumes: `state.accounts.list() -> Result<Vec<ManagedAccount>>` (existing, `ManagedAccount { account_uuid, email, .. }`), `state.db.limit_hit_stats(...)` (Task 3), `state.settings.read().thresholds: Vec<u8>` (existing).
- Produces: Tauri command `get_limit_hit_history(days: u32) -> Result<LimitHitReport, String>`, struct `LimitHitReport { accounts: Vec<AccountLimitHits> }`. Task 6's `ipc.ts` wrapper and Task 7's frontend tab consume the generated TS binding for this.

- [ ] **Step 1: Add the command**

In `src-tauri/src/commands.rs`, near `get_compactions` / `get_daily_trends` (same "read-only report query" section), add:

```rust
#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct LimitHitReport {
    pub accounts: Vec<crate::store::AccountLimitHits>,
}

#[command]
#[specta::specta]
pub async fn get_limit_hit_history(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<LimitHitReport, String> {
    let to = Utc::now();
    let from = to - Duration::days(days as i64);
    let danger_threshold = {
        let s = state.settings.read();
        s.thresholds.get(1).copied().unwrap_or(90) as f64
    };
    let accounts = state.accounts.list().map_err(err_to_string)?;
    let mut out = Vec::new();
    for acc in accounts {
        let hits = state
            .db
            .limit_hit_stats(&acc.account_uuid, &acc.email, from, to, danger_threshold)
            .map_err(err_to_string)?;
        out.push(hits);
    }
    Ok(LimitHitReport { accounts: out })
}
```

No re-export step needed: `src-tauri/src/store/mod.rs` already has `pub use queries::*;`, so `AccountLimitHits` (and `ProjectAttribution`, `WindowPeak`) are automatically visible as `crate::store::AccountLimitHits` the moment Tasks 2/3 add them as `pub struct`s in `queries.rs`.

- [ ] **Step 2: Register the command in both `collect_commands!` blocks**

In `src-tauri/src/lib.rs`, add `commands::get_limit_hit_history,` to **both** the `#[cfg(not(debug_assertions))]` and `#[cfg(debug_assertions)]` `collect_commands!` lists (right after `commands::get_live_sessions,` in each — the last entry in both lists currently). Both blocks must stay in sync; tauri-specta replaces rather than appends, so a handler missing from either build config silently isn't callable in that config.

- [ ] **Step 3: Regenerate the TypeScript bindings**

Bindings are regenerated automatically the next time the app runs in a debug build (`src-tauri/src/lib.rs`'s `#[cfg(debug_assertions)] specta_builder.export(...)` call, which runs before the database even opens). From `src-tauri/`, run:

```bash
cargo build
```

then run the app once (`cargo tauri dev`, or the repo's existing dev script) far enough for `run()` to execute the export — a plain `cargo build` alone does not trigger it, since the export call lives inside `run()`, not in a build script.

**Before doing this**, check whether another instance of the app is already running (this repo has a documented history of SQLite lock contention between concurrent instances) — quit it first if so, since the bindings export happens early but the subsequent DB open in the same startup path can still fail against a locked file.

Confirm `src/lib/generated/bindings.ts` now contains `getLimitHitHistory`, `LimitHitReport`, `AccountLimitHits`, `ProjectAttribution`, and `WindowPeak` types.

- [ ] **Step 4: Run the full backend test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs src/lib/generated/bindings.ts
git commit -m "feat(ipc): add get_limit_hit_history command"
```

---

## Task 6: `ipc.ts` wrapper

**Files:**
- Modify: `src/lib/ipc.ts`

**Interfaces:**
- Consumes: `commands.getLimitHitHistory(days)` from the regenerated `src/lib/generated/bindings.ts` (Task 5).
- Produces: `ipc.getLimitHitHistory(days: number) -> Promise<LimitHitReport>`. Task 7's tab calls this.

- [ ] **Step 1: Add the wrapper**

In `src/lib/ipc.ts`, next to `getDailyTrends`, add:

```ts
  getLimitHitHistory: (days: number) => commands.getLimitHitHistory(days).then(unwrap),
```

- [ ] **Step 2: Typecheck**

Run: `npx tsc --noEmit`
Expected: no errors (fails here if Task 5's bindings regeneration was skipped or incomplete — `commands.getLimitHitHistory` wouldn't exist).

- [ ] **Step 3: Commit**

```bash
git add src/lib/ipc.ts
git commit -m "feat(ipc): add getLimitHitHistory wrapper"
```

---

## Task 7: `LimitHitsTab` — new report tab

**Files:**
- Create: `src/report/LimitHitsTab.tsx`
- Test: `src/report/LimitHitsTab.test.tsx`
- Modify: `src/report/ExpandedReport.tsx`

**Interfaces:**
- Consumes: `ipc.getLimitHitHistory(days)` (Task 6), `LimitHitReport` / `AccountLimitHits` types from `../lib/generated/bindings`, existing `Card`, `EmptyState`, `Button` UI primitives, `useTabData`, `useAppStore((s) => s.sessionDataVersion)`, `formatCost` from `../lib/format`, `IconWarning` from `../lib/icons`.
- Produces: `LimitHitsTab` component, wired into `ExpandedReport.tsx`'s tab system as `id: 'limits'`.

- [ ] **Step 1: Write the failing test**

Create `src/report/LimitHitsTab.test.tsx`:

```tsx
import { render, screen } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { LimitHitReport } from '../lib/generated/bindings';

const REPORT: LimitHitReport = {
  accounts: [
    {
      account_id: 'acc-1',
      email: 'work@example.com',
      five_hour_hits: 2,
      seven_day_hits: 1,
      hourly_distribution: Array.from({ length: 24 }, (_, h) => (h === 9 ? 2 : h === 14 ? 1 : 0)),
      top_projects: [
        { project: 'switchboard', cost_usd: 12.5 },
        { project: 'other-repo', cost_usd: 3.1 },
      ],
    },
  ],
};

const EMPTY_REPORT: LimitHitReport = { accounts: [] };

const ipcMock = vi.hoisted(() => ({ getLimitHitHistory: vi.fn() }));
vi.mock('../lib/ipc', () => ({ ipc: ipcMock }));

vi.mock('../lib/store', async () => {
  const actual = await vi.importActual<typeof import('../lib/store')>('../lib/store');
  const state = { sessionDataVersion: 0 };
  const useAppStore: any = (sel: any) => sel(state);
  useAppStore.getState = () => state;
  return { ...actual, useAppStore };
});

import { LimitHitsTab } from './LimitHitsTab';

describe('LimitHitsTab', () => {
  beforeEach(() => {
    ipcMock.getLimitHitHistory.mockClear();
  });

  it('shows an empty state when no account has any hits', async () => {
    ipcMock.getLimitHitHistory.mockResolvedValue(EMPTY_REPORT);
    render(<LimitHitsTab />);
    expect(await screen.findByText(/no limit hits yet/i)).toBeTruthy();
  });

  it('renders hit counts and top projects for accounts with history', async () => {
    ipcMock.getLimitHitHistory.mockResolvedValue(REPORT);
    render(<LimitHitsTab />);
    expect(await screen.findByText('work@example.com')).toBeTruthy();
    expect(screen.getByText(/2 × 5H/)).toBeTruthy();
    expect(screen.getByText(/1 × 7D/)).toBeTruthy();
    expect(screen.getByText('switchboard')).toBeTruthy();
    expect(screen.getByText('$12.50')).toBeTruthy();
  });
});
```

Check `formatCost`'s exact output format in `src/lib/format.ts` before asserting `'$12.50'` literally — adjust the assertion to match whatever that function actually returns for `12.5`.

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/report/LimitHitsTab.test.tsx`
Expected: FAIL — `Failed to resolve import "./LimitHitsTab"` (component doesn't exist yet).

- [ ] **Step 3: Write the minimal implementation**

Create `src/report/LimitHitsTab.tsx`:

```tsx
import { Card } from '../components/ui/Card';
import { EmptyState } from '../components/ui/EmptyState';
import { Button } from '../components/ui/Button';
import { formatCost } from '../lib/format';
import { IconWarning } from '../lib/icons';
import { ipc } from '../lib/ipc';
import { useTabData } from '../lib/useTabData';
import { useAppStore } from '../lib/store';
import type { AccountLimitHits } from '../lib/generated/bindings';

export function LimitHitsTab() {
  const version = useAppStore((s) => s.sessionDataVersion);
  const { data, error, loading, reload } = useTabData(
    () => ipc.getLimitHitHistory(30),
    [version],
  );

  if (error) {
    return (
      <EmptyState
        icon={<IconWarning size={32} />}
        title="Couldn't load limit-hit history"
        description={error}
        action={<Button variant="ghost" size="sm" onClick={reload}>Retry</Button>}
      />
    );
  }
  if (loading || !data) {
    return <p className="text-[color:var(--color-text-muted)]">Loading…</p>;
  }

  const accountsWithHits = data.accounts.filter(
    (a) => a.five_hour_hits + a.seven_day_hits > 0,
  );
  if (accountsWithHits.length === 0) {
    return (
      <EmptyState
        icon={<IconWarning size={32} />}
        title="No limit hits yet"
        description="This report tracks rate-limit peaks going forward — check back after using Claude for a while."
      />
    );
  }

  return (
    <div className="flex flex-col gap-[var(--space-lg)]">
      {accountsWithHits.map((a) => (
        <AccountLimitHitsCard key={a.account_id} account={a} />
      ))}
    </div>
  );
}

function AccountLimitHitsCard({ account }: { account: AccountLimitHits }) {
  const maxCount = Math.max(...account.hourly_distribution, 1);
  return (
    <Card>
      <div className="flex items-center justify-between px-[var(--space-md)] pt-[var(--space-md)]">
        <span className="text-[length:var(--text-label)] font-[var(--weight-medium)]">
          {account.email}
        </span>
        <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          {account.five_hour_hits} × 5H · {account.seven_day_hits} × 7D
        </span>
      </div>
      <div
        className="flex items-end gap-[2px] px-[var(--space-md)] py-[var(--space-md)]"
        style={{ height: 80 }}
      >
        {account.hourly_distribution.map((count, hour) => (
          <div
            key={hour}
            className="flex-1 flex flex-col justify-end"
            title={`${hour}:00 — ${count} hit${count === 1 ? '' : 's'}`}
          >
            <div
              className="rounded-t-sm bg-[var(--color-danger)]"
              style={{ height: `${(count / maxCount) * 100}%`, minHeight: count > 0 ? 2 : 0 }}
            />
          </div>
        ))}
      </div>
      {account.top_projects.length > 0 && (
        <div className="flex flex-col gap-[var(--space-2xs)] px-[var(--space-md)] pb-[var(--space-md)]">
          {account.top_projects.map((p) => (
            <div
              key={p.project}
              className="flex items-center justify-between text-[length:var(--text-micro)]"
            >
              <span className="text-[color:var(--color-text-secondary)] truncate">{p.project}</span>
              <span className="mono text-[color:var(--color-text-muted)]">{formatCost(p.cost_usd)}</span>
            </div>
          ))}
        </div>
      )}
    </Card>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/report/LimitHitsTab.test.tsx`
Expected: PASS

- [ ] **Step 5: Wire the tab into `ExpandedReport.tsx`**

In `src/report/ExpandedReport.tsx`:

Add the import, next to `import { CacheTab } from './CacheTab';`:
```ts
import { LimitHitsTab } from './LimitHitsTab';
```

Add a `TAB_CONFIG` entry, after the `{ id: 'trends', label: 'Trends' }` line:
```ts
  { id: 'limits', label: 'Limit hits' },
```

Add to `TAB_WINDOW_DAYS` (matches the `days` argument `LimitHitsTab` passes to `ipc.getLimitHitHistory`):
```ts
  limits: 30,
```

Add to `TAB_COMPONENTS`:
```ts
  limits: LimitHitsTab,
```

- [ ] **Step 6: Run the full frontend test suite and typecheck**

Run: `npx vitest run && npx tsc --noEmit`
Expected: all tests PASS, no typecheck errors.

- [ ] **Step 7: Commit**

```bash
git add src/report/LimitHitsTab.tsx src/report/LimitHitsTab.test.tsx src/report/ExpandedReport.tsx
git commit -m "feat(report): add Limit hits tab (F5)"
```
