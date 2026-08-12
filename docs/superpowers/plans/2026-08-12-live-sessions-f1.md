# F1 — Live Session Registry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A backend registry that tracks which Claude Code sessions are actively writing to their JSONL transcript right now, with accruing per-session totals, surfaced as a "Now running" section in the popover.

**Architecture:** The watcher's per-file touch loop (already the only place that knows "this file's bytes just grew") feeds a new `LiveSessionRegistry` in `AppState`. Two new single-session-scoped DB queries compute a touched session's totals and its latest event's project/model/context — cheap compared to the full-table `session_totals()` scan used for historical reporting. A 30s prune tick walks Live→Cooling→Departed transitions and emits the current Live set to the frontend on every change. The frontend mirrors that list in the store and renders it as a new popover section.

**Tech Stack:** Rust (Tauri v2 backend, rusqlite, tokio), React 19 + TypeScript, Vitest, cargo test.

**Spec:** `docs/superpowers/specs/2026-08-12-live-sessions-f1-design.md`

## Global Constraints

- Liveness thresholds: Live while `last_activity` is within 120s; Cooling from 120s to 300s quiet; removed (Departed) at 300s quiet. Prune tick interval: 30s.
- Session identity = transcript path relative to the projects root (`source_file`); subagent files fold to their parent via the existing `/subagents/` split convention (`store/queries.rs` `session_totals()`).
- Only post-launch activity registers — the startup backfill must NOT feed the registry.
- `get_live_sessions` returns Live entries only, sorted by `last_activity` descending.
- No DB schema change. Registered IPC commands go in **both** `collect_commands!` blocks in `lib.rs` (release list ~line 117, debug list ~line 168 — grep `collect_commands!` to confirm current line numbers, they shift as commands are added).
- Package manager pnpm. Rust tests: `cd src-tauri && cargo test` (or `cargo test --manifest-path src-tauri/Cargo.toml` from repo root). TS: `pnpm lint`, `pnpm test`.
- `src/lib/generated/bindings.ts` is generated-but-committed: hand-edit in the file's exact existing alphabetical-sort style (confirmed convention from the PAYG feature's review).
- Known pre-existing baseline failure (NOT yours): `src/lib/__tests__/theme.test.ts` fails at collection (localStorage error). Bar = no new failures.
- F2 and F3 (separate future plans) will extend `LiveEntry` with a notified-set and armed/disarmed hysteresis state respectively — keep the struct's fields additive-friendly (no assumptions baked in that block adding fields later), but do not build those fields now.

---

### Task 1: Single-session DB query helpers

**Files:**
- Modify: `src-tauri/src/store/queries.rs` (new methods near `session_totals` ~line 317-339; new pub struct near the top of the file with the other `Stored*` structs ~line 17-33)

**Interfaces:**
- Consumes: existing `session_events` table (columns: `ts, project, model, input_tokens, output_tokens, cache_read_tokens, cache_creation_5m_tokens, cache_creation_1h_tokens, cost_usd, source_file, source_line, event_id`), `Db::ingest_atomic` (existing public method, used by this task's tests to seed rows) and `Db::conn()`.
- Produces: `Db::live_session_totals(&self, parent_source_file: &str) -> Result<(u64, f64)>` (tokens, cost — parent + folded subagents); `Db::latest_event_for_file(&self, source_file: &str) -> Result<Option<LatestEventInfo>>` with `pub struct LatestEventInfo { pub project: String, pub model: String, pub context_tokens: u64 }`. Task 2 calls both.

- [ ] **Step 1: Write the failing tests**

Add to `src-tauri/src/store/queries.rs`'s existing `#[cfg(test)] mod tests` (reuse the module's `fresh_db()` helper — it's already defined there):

```rust
    fn mk_event(source_file: &str, source_line: i64, input: u64, cost: f64) -> StoredSessionEvent {
        StoredSessionEvent {
            ts: Utc::now(),
            project: "my-proj".into(),
            model: "claude-opus-5".into(),
            input_tokens: input,
            output_tokens: 5,
            cache_read_tokens: 100,
            cache_creation_5m_tokens: 10,
            cache_creation_1h_tokens: 0,
            cost_usd: cost,
            source_file: source_file.into(),
            source_line,
            event_id: format!("{source_file}:{source_line}"),
        }
    }

    #[test]
    fn live_session_totals_folds_subagents_onto_parent() {
        let (_dir, db) = fresh_db();
        let parent = "-Users-me-proj/sess1.jsonl";
        db.ingest_atomic(parent, &[mk_event(parent, 0, 100, 0.5), mk_event(parent, 1, 100, 0.5)], &[], 1, 200).unwrap();
        let sub = "-Users-me-proj/sess1/subagents/agent-a.jsonl";
        db.ingest_atomic(sub, &[mk_event(sub, 0, 50, 0.1)], &[], 1, 100).unwrap();
        // A different session must not leak in.
        let other = "-Users-me-proj/sess2.jsonl";
        db.ingest_atomic(other, &[mk_event(other, 0, 999, 9.0)], &[], 1, 100).unwrap();

        let (tokens, cost) = db.live_session_totals(parent).unwrap();
        // input+output per event: (100+5)+(100+5)+(50+5) = 265
        assert_eq!(tokens, 265);
        assert!((cost - 1.1).abs() < 1e-9);
    }

    #[test]
    fn live_session_totals_zero_for_unknown_session() {
        let (_dir, db) = fresh_db();
        let (tokens, cost) = db.live_session_totals("-Users-me-proj/never-seen.jsonl").unwrap();
        assert_eq!(tokens, 0);
        assert_eq!(cost, 0.0);
    }

    #[test]
    fn latest_event_for_file_returns_most_recent_by_source_line() {
        let (_dir, db) = fresh_db();
        let parent = "-Users-me-proj/sess1.jsonl";
        db.ingest_atomic(
            parent,
            &[
                mk_event(parent, 0, 1000, 0.1),
                { let mut e = mk_event(parent, 5, 2000, 0.2); e.model = "claude-sonnet-5".into(); e.cache_read_tokens = 500; e.cache_creation_5m_tokens = 50; e },
            ],
            &[],
            1,
            300,
        ).unwrap();
        let info = db.latest_event_for_file(parent).unwrap().expect("row present");
        assert_eq!(info.project, "my-proj");
        assert_eq!(info.model, "claude-sonnet-5");
        // 2000 input + 500 cache_read + 50 cache_5m + 0 cache_1h
        assert_eq!(info.context_tokens, 2550);
    }

    #[test]
    fn latest_event_for_file_ignores_subagent_files() {
        let (_dir, db) = fresh_db();
        let parent = "-Users-me-proj/sess1.jsonl";
        db.ingest_atomic(parent, &[mk_event(parent, 0, 100, 0.1)], &[], 1, 100).unwrap();
        let sub = "-Users-me-proj/sess1/subagents/agent-a.jsonl";
        db.ingest_atomic(sub, &[mk_event(sub, 99, 50000, 5.0)], &[], 1, 100).unwrap();
        // Querying the PARENT file must not pick up the subagent's huge event.
        let info = db.latest_event_for_file(parent).unwrap().expect("row present");
        assert_eq!(info.context_tokens, 100 + 100 + 10); // input+cache_read+cache_5m from mk_event
    }

    #[test]
    fn latest_event_for_file_none_when_no_rows() {
        let (_dir, db) = fresh_db();
        assert!(db.latest_event_for_file("-Users-me-proj/never.jsonl").unwrap().is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test live_session_totals latest_event_for_file`
Expected: compile errors — `live_session_totals`, `latest_event_for_file`, `LatestEventInfo` don't exist yet.

- [ ] **Step 3: Implement**

Add near the top of `queries.rs`, alongside the other `Stored*` structs (after `StoredCompaction`'s definition):

```rust
/// The parent transcript's most recent event — used by the live-session
/// registry to answer "what is this session doing right now." Subagent
/// transcripts are deliberately excluded: they have their own context
/// windows and aren't part of what the parent session's readout shows.
#[derive(Debug, Clone)]
pub struct LatestEventInfo {
    pub project: String,
    pub model: String,
    /// input + cache_read + cache_creation_5m + cache_creation_1h tokens on
    /// this one event — the same context-size formula used elsewhere in
    /// this codebase's peak-context tracking.
    pub context_tokens: u64,
}
```

Add near `session_totals` (after it):

```rust
    /// Aggregate tokens/cost for one live session: the parent transcript
    /// plus any subagent transcripts folded under it via the same
    /// `/subagents/` convention `session_totals()` uses for the full-table
    /// historical report. Scoped to a single session (two bound params),
    /// unlike `session_totals()`'s full-table GROUP BY — this runs on every
    /// ingest touch, so it stays cheap regardless of total history size.
    pub fn live_session_totals(&self, parent_source_file: &str) -> Result<(u64, f64)> {
        let conn = self.conn();
        let prefix = parent_source_file
            .strip_suffix(".jsonl")
            .unwrap_or(parent_source_file);
        let subagent_pattern = format!("{prefix}/subagents/%");
        conn.query_row(
            "SELECT COALESCE(SUM(input_tokens + output_tokens), 0), COALESCE(SUM(cost_usd), 0.0)
             FROM session_events WHERE source_file = ?1 OR source_file LIKE ?2",
            params![parent_source_file, subagent_pattern],
            |r| {
                let tokens: i64 = r.get(0)?;
                let cost: f64 = r.get(1)?;
                Ok((tokens.max(0) as u64, cost))
            },
        )
        .map_err(Into::into)
    }

    /// The parent file's most recent event (by `source_line`), or `None` if
    /// nothing has been ingested for it yet. Subagent files are excluded by
    /// the exact `source_file` match (no LIKE) — see `LatestEventInfo`.
    pub fn latest_event_for_file(&self, source_file: &str) -> Result<Option<LatestEventInfo>> {
        let conn = self.conn();
        conn.query_row(
            "SELECT project, model, input_tokens, cache_read_tokens,
                    cache_creation_5m_tokens, cache_creation_1h_tokens
             FROM session_events WHERE source_file = ?1
             ORDER BY source_line DESC LIMIT 1",
            params![source_file],
            |r| {
                let input: i64 = r.get(2)?;
                let cache_read: i64 = r.get(3)?;
                let cache_5m: i64 = r.get(4)?;
                let cache_1h: i64 = r.get(5)?;
                Ok(LatestEventInfo {
                    project: r.get(0)?,
                    model: r.get(1)?,
                    context_tokens: (input.max(0) + cache_read.max(0) + cache_5m.max(0) + cache_1h.max(0))
                        as u64,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test live_session_totals latest_event_for_file`
Expected: PASS (6 tests). Then `cargo test` (full suite) to confirm no regressions.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/store/queries.rs
git commit -m "feat(store): add single-session query helpers for the live-session registry"
```

---

### Task 2: `LiveSessionRegistry` state machine (pure logic)

**Files:**
- Create: `src-tauri/src/live_sessions.rs`
- Modify: `src-tauri/src/lib.rs` (module declaration — add `pub mod live_sessions;` alongside the other `pub mod` lines near the top)

**Interfaces:**
- Consumes: `Db::live_session_totals`, `Db::latest_event_for_file`, `Db::LatestEventInfo` from Task 1.
- Produces: `pub struct LiveSessionInfo { session_id, source_file, project, model, total_tokens, total_cost_usd, context_tokens, first_seen, last_activity }` (serde+specta derives); `pub struct LiveSessionRegistry` with `note_ingest(&self, db: &Db, touched_file: &Path, projects_root: &Path, now: DateTime<Utc>)`, `prune(&self, now: DateTime<Utc>)`, `live_snapshot(&self) -> Vec<LiveSessionInfo>` (Live entries only, sorted `last_activity` desc). Task 3 wires this into the watcher/lib.rs/AppState/IPC.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/live_sessions.rs` with this test module first (the implementation follows in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Db;
    use crate::store::queries::StoredSessionEvent;
    use chrono::{Duration, Utc};
    use std::path::PathBuf;

    fn fresh() -> (tempfile::TempDir, Db) {
        let d = tempfile::tempdir().unwrap();
        let db = Db::open(d.path()).unwrap();
        (d, db)
    }

    fn seed(db: &Db, source_file: &str, tokens: u64, model: &str) {
        let ev = StoredSessionEvent {
            ts: Utc::now(),
            project: "proj".into(),
            model: model.into(),
            input_tokens: tokens,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 0.01,
            source_file: source_file.into(),
            source_line: 0,
            event_id: format!("{source_file}:seed:{tokens}"),
        };
        db.ingest_atomic(source_file, &[ev], &[], 1, 100).unwrap();
    }

    // projects_root/touched_file relationship: note_ingest computes the
    // registry key as the path relative to projects_root, matching how the
    // watcher and walker already derive `source_file`.
    fn root_and_file() -> (PathBuf, PathBuf, &'static str) {
        let root = PathBuf::from("/home/me/.claude/projects");
        let rel = "-proj/sess1.jsonl";
        (root.clone(), root.join(rel), rel)
    }

    #[test]
    fn fresh_touch_becomes_live() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let now = Utc::now();
        reg.note_ingest(&db, &file, &root, now);
        let snap = reg.live_snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].session_id, "sess1");
        assert_eq!(snap[0].model, "claude-opus-5");
        assert_eq!(snap[0].first_seen, now.timestamp());
        assert_eq!(snap[0].last_activity, now.timestamp());
    }

    #[test]
    fn subagent_touch_folds_onto_parent_key() {
        let (_d, db) = fresh();
        let root = PathBuf::from("/home/me/.claude/projects");
        let parent_key = "-proj/sess1.jsonl";
        let sub_key = "-proj/sess1/subagents/agent-a.jsonl";
        seed(&db, parent_key, 100, "claude-opus-5");
        seed(&db, sub_key, 50, "claude-haiku-4-5");
        let reg = LiveSessionRegistry::default();
        let now = Utc::now();
        reg.note_ingest(&db, &root.join(sub_key), &root, now);
        let snap = reg.live_snapshot();
        assert_eq!(snap.len(), 1, "subagent touch must fold onto the parent, not create a second entry");
        assert_eq!(snap[0].session_id, "sess1");
        assert_eq!(snap[0].total_tokens, (100 + 1) + (50 + 1)); // input+output per seeded event
    }

    #[test]
    fn quiet_120s_transitions_to_cooling_and_hides_from_snapshot() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &file, &root, t0);
        assert_eq!(reg.live_snapshot().len(), 1);
        reg.prune(t0 + Duration::seconds(121));
        assert_eq!(reg.live_snapshot().len(), 0, "Cooling entries are not Live");
    }

    #[test]
    fn write_during_cooling_returns_to_live_preserving_first_seen() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &file, &root, t0);
        reg.prune(t0 + Duration::seconds(121)); // -> Cooling
        let t1 = t0 + Duration::seconds(150);
        reg.note_ingest(&db, &file, &root, t1); // write during Cooling
        let snap = reg.live_snapshot();
        assert_eq!(snap.len(), 1, "a write during Cooling returns the entry to Live");
        assert_eq!(snap[0].first_seen, t0.timestamp(), "first_seen must survive the Cooling round-trip");
        assert_eq!(snap[0].last_activity, t1.timestamp());
    }

    #[test]
    fn quiet_300s_removes_the_entry() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &file, &root, t0);
        reg.prune(t0 + Duration::seconds(301));
        assert_eq!(reg.live_snapshot().len(), 0);
        // And it's gone entirely, not just hidden: a later write starts a
        // brand new entry with a fresh first_seen, proving removal not just
        // a permanently-hidden state.
        let t1 = t0 + Duration::seconds(1000);
        reg.note_ingest(&db, &file, &root, t1);
        assert_eq!(reg.live_snapshot()[0].first_seen, t1.timestamp());
    }

    #[test]
    fn snapshot_sorted_by_last_activity_descending() {
        let (_d, db) = fresh();
        let root = PathBuf::from("/home/me/.claude/projects");
        seed(&db, "-proj/a.jsonl", 10, "claude-opus-5");
        seed(&db, "-proj/b.jsonl", 10, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &root.join("-proj/a.jsonl"), &root, t0);
        reg.note_ingest(&db, &root.join("-proj/b.jsonl"), &root, t0 + Duration::seconds(5));
        let snap = reg.live_snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].session_id, "b", "most recently touched sorts first");
        assert_eq!(snap[1].session_id, "a");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test live_sessions::`
Expected: compile errors — `LiveSessionRegistry`, `LiveSessionInfo` don't exist yet (this is a new, empty-except-tests file).

- [ ] **Step 3: Write the implementation**

Prepend to `src-tauri/src/live_sessions.rs` (before the `#[cfg(test)]` block already written):

```rust
use crate::store::Db;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const LIVE_QUIET_SECS: i64 = 120;
const COOLING_QUIET_SECS: i64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionState {
    Live,
    Cooling,
}

#[derive(Debug, Clone)]
struct LiveEntry {
    session_id: String,
    source_file: String,
    project: String,
    model: String,
    total_tokens: u64,
    total_cost_usd: f64,
    context_tokens: u64,
    first_seen: DateTime<Utc>,
    last_activity: DateTime<Utc>,
    state: SessionState,
}

/// One row of `get_live_sessions`. Deliberately structured so future
/// features (session-finished notifications, context-window warnings) can
/// add fields without reshaping this one — see the plan's Global
/// Constraints.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LiveSessionInfo {
    pub session_id: String,
    pub source_file: String,
    pub project: String,
    pub model: String,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    pub context_tokens: u64,
    /// Unix seconds — when the registry first saw this session THIS APP
    /// RUN (not the transcript's real age; the registry is in-memory only
    /// and starts empty on every launch, by design).
    pub first_seen: i64,
    /// Unix seconds of the most recent ingest touch.
    pub last_activity: i64,
}

/// Tracks which Claude Code sessions are actively writing to their JSONL
/// transcript right now. Fed exclusively by the watcher's per-file touch
/// loop (never by the startup backfill — see `note_ingest`'s caller in
/// `lib.rs`), so only post-launch activity ever registers.
#[derive(Default)]
pub struct LiveSessionRegistry {
    sessions: RwLock<HashMap<String, LiveEntry>>,
}

/// Folds a subagent transcript path onto its parent's key, and derives the
/// bare session id (file stem) — same `/subagents/` convention as
/// `Db::session_totals`. `touched_file` and `projects_root` are both
/// absolute paths; the returned key is projects-root-relative, matching
/// `source_file` as stored in `session_events`.
fn registry_key(touched_file: &Path, projects_root: &Path) -> Option<(String, String)> {
    let rel = touched_file.strip_prefix(projects_root).ok()?;
    let rel_str = rel.to_str()?.replace('\\', "/"); // Windows path separators
    let parent_key = match rel_str.find("/subagents/") {
        Some(i) => format!("{}.jsonl", &rel_str[..i]),
        None => rel_str,
    };
    let session_id = parent_key
        .rsplit('/')
        .next()
        .unwrap_or(&parent_key)
        .strip_suffix(".jsonl")
        .unwrap_or(&parent_key)
        .to_string();
    Some((parent_key, session_id))
}

impl LiveSessionRegistry {
    /// Called once per watcher-touched file (parent or subagent). Refreshes
    /// the PARENT session's entry — subagent touches update the parent's
    /// totals but the parent's own `latest_event_for_file` (project/model/
    /// context) is untouched by a subagent write, since that query is
    /// scoped to the parent file only.
    pub fn note_ingest(&self, db: &Db, touched_file: &Path, projects_root: &Path, now: DateTime<Utc>) {
        let Some((parent_key, session_id)) = registry_key(touched_file, projects_root) else {
            return;
        };
        let Ok((total_tokens, total_cost_usd)) = db.live_session_totals(&parent_key) else {
            return;
        };
        // The parent file's own latest event may not exist yet on the very
        // first subagent touch (main file written after its first
        // subagent in rare orderings) — fall back to sensible empties
        // rather than dropping the touch entirely.
        let latest = db.latest_event_for_file(&parent_key).ok().flatten();

        let mut sessions = self.sessions.write();
        let first_seen = sessions
            .get(&parent_key)
            .map(|e| e.first_seen)
            .unwrap_or(now);
        sessions.insert(
            parent_key.clone(),
            LiveEntry {
                session_id,
                source_file: parent_key,
                project: latest.as_ref().map(|l| l.project.clone()).unwrap_or_default(),
                model: latest.as_ref().map(|l| l.model.clone()).unwrap_or_default(),
                total_tokens,
                total_cost_usd,
                context_tokens: latest.map(|l| l.context_tokens).unwrap_or(0),
                first_seen,
                last_activity: now,
                state: SessionState::Live,
            },
        );
    }

    /// Walks every entry: Live -> Cooling at `LIVE_QUIET_SECS` quiet,
    /// Cooling -> removed at `COOLING_QUIET_SECS` quiet. Call on a timer
    /// (the 30s tick in lib.rs); pure state transition, no I/O.
    pub fn prune(&self, now: DateTime<Utc>) {
        let mut sessions = self.sessions.write();
        sessions.retain(|_, e| {
            let quiet = (now - e.last_activity).num_seconds();
            if quiet >= COOLING_QUIET_SECS {
                false
            } else {
                if quiet >= LIVE_QUIET_SECS {
                    e.state = SessionState::Cooling;
                }
                true
            }
        });
    }

    /// Live entries only, sorted by most-recently-active first.
    pub fn live_snapshot(&self) -> Vec<LiveSessionInfo> {
        let sessions = self.sessions.read();
        let mut out: Vec<LiveSessionInfo> = sessions
            .values()
            .filter(|e| e.state == SessionState::Live)
            .map(|e| LiveSessionInfo {
                session_id: e.session_id.clone(),
                source_file: e.source_file.clone(),
                project: e.project.clone(),
                model: e.model.clone(),
                total_tokens: e.total_tokens,
                total_cost_usd: e.total_cost_usd,
                context_tokens: e.context_tokens,
                first_seen: e.first_seen.timestamp(),
                last_activity: e.last_activity.timestamp(),
            })
            .collect();
        out.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
        out
    }
}
```

Register the module in `src-tauri/src/lib.rs`: find the existing `pub mod` list near the top of the file (e.g. `pub mod jsonl_parser;`, `pub mod sessions;`) and add `pub mod live_sessions;` alongside them, alphabetically if the list is sorted.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test live_sessions::`
Expected: PASS (6 tests). Then `cargo build` to confirm `lib.rs`'s new `pub mod` line compiles (the registry isn't wired into `AppState` yet — that's Task 3 — so nothing else should break).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/live_sessions.rs src-tauri/src/lib.rs
git commit -m "feat(live-sessions): add the live-session registry state machine"
```

---

### Task 3: Backend wiring — watcher, AppState, IPC command

**Files:**
- Modify: `src-tauri/src/jsonl_parser/watcher.rs` (channel payload type ~line 19-56)
- Modify: `src-tauri/src/lib.rs` (watcher setup/consumer ~line 651-694, AppState construction ~line 274-292, `collect_commands!` both blocks)
- Modify: `src-tauri/src/app_state.rs` (new field on `AppState` ~line 172-210)
- Modify: `src-tauri/src/commands.rs` (new `get_live_sessions` command, near other simple read commands like `get_session_history` ~line 108)

**Interfaces:**
- Consumes: `LiveSessionRegistry`, `LiveSessionInfo` from Task 2.
- Produces: `AppState.live_sessions: live_sessions::LiveSessionRegistry`; IPC command `get_live_sessions() -> Vec<LiveSessionInfo>`; Tauri event `live_sessions_changed` with payload `Vec<LiveSessionInfo>`. Task 4 (frontend) consumes both.

- [ ] **Step 1: Update the watcher's channel payload**

In `src-tauri/src/jsonl_parser/watcher.rs`, change `start`'s signature and the per-file send to carry the touched path alongside the count (the consumer needs the path to call `note_ingest`; today it only gets a bare count):

```rust
pub fn start(
    db: Arc<Db>,
    pricing: Arc<PricingTable>,
    root: PathBuf,
    tx: mpsc::UnboundedSender<(PathBuf, usize)>,
) -> Result<WatcherHandle> {
```

And in the per-file match arm:

```rust
            for p in touched {
                match walker::ingest_file(&db_clone, &pricing_clone, &p, &root_clone) {
                    Ok(n) if n > 0 => {
                        let _ = tx.send((p.clone(), n));
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("ingest {} failed: {}", p.display(), e),
                }
            }
```

- [ ] **Step 2: Update `AppState`**

In `src-tauri/src/app_state.rs`, add a field to the `AppState` struct (after `sessions_cache`):

```rust
    /// Which Claude Code sessions are actively writing to their transcript
    /// right now. In-memory only — starts empty on every launch (see
    /// `live_sessions::LiveSessionRegistry`'s doc comment for why).
    pub live_sessions: crate::live_sessions::LiveSessionRegistry,
```

And in `lib.rs`'s `AppState { ... }` construction, add:

```rust
        live_sessions: crate::live_sessions::LiveSessionRegistry::default(),
```

(Match the existing construction's field order — add it right after `sessions_cache: parking_lot::RwLock::new(None),`.)

- [ ] **Step 3: Update the watcher consumer + prune tick in `lib.rs`**

Replace the existing consumer block (the one at ~line 665-675 that creates the `usize` channel and only emits `session_ingested`):

```rust
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(std::path::PathBuf, usize)>();
                let handle_for_events = handle.clone();
                let state_for_ingest = state.clone();
                let root_for_ingest = root.clone();
                tauri::async_runtime::spawn(async move {
                    use tauri::Emitter;
                    while let Some((path, n)) = rx.recv().await {
                        state_for_ingest.live_sessions.note_ingest(
                            &state_for_ingest.db,
                            &path,
                            &root_for_ingest,
                            chrono::Utc::now(),
                        );
                        let _ = handle_for_events.emit(
                            "live_sessions_changed",
                            state_for_ingest.live_sessions.live_snapshot(),
                        );
                        let _ = handle_for_events.emit("session_ingested", n);
                    }
                });
```

Note the backfill spawn immediately above this block (the one that calls `jsonl_parser::walker::ingest_file` directly in a loop over `discover_jsonl_files`) is UNCHANGED — it must NOT call `note_ingest`, per the Global Constraint that only post-launch activity registers.

Add the prune tick — a new spawn alongside the existing warm-up dispatcher's 30s-interval pattern (copy that pattern; it's a few lines above the JSONL backfill block). It needs both the shared state and an `AppHandle` to emit through — capture `handle.clone()` (the same `AppHandle` the backfill/watcher block below already has in scope) alongside `state.clone()`:

```rust
            {
                let prune_state = state.clone();
                let prune_handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    loop {
                        interval.tick().await;
                        let before = prune_state.live_sessions.live_snapshot();
                        prune_state.live_sessions.prune(chrono::Utc::now());
                        let after = prune_state.live_sessions.live_snapshot();
                        if before.len() != after.len() {
                            use tauri::Emitter;
                            let _ = prune_handle.emit("live_sessions_changed", after);
                        }
                    }
                });
            }
```

Place this new block right before the `if let Some(root) = jsonl_parser::walker::claude_projects_root() { ... }` block (the prune tick should run regardless of whether a projects root was found, though in practice it's a no-op with an empty registry either way).

- [ ] **Step 4: Add the IPC command**

In `src-tauri/src/commands.rs`, near `get_session_history`:

```rust
/// Sessions whose JSONL transcript received a write within the last
/// LIVE_QUIET_SECS (120s). Purely a read of the in-memory registry — no DB
/// query of its own (the registry's entries are already the result of one).
#[command]
#[specta::specta]
pub async fn get_live_sessions(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<crate::live_sessions::LiveSessionInfo>, String> {
    Ok(state.live_sessions.live_snapshot())
}
```

Register `commands::get_live_sessions` in **both** `collect_commands!` blocks in `lib.rs` — grep `collect_commands!` first to find their current exact contents; add the new command as a new line in each list, matching the existing list's style (one command per line, trailing comma).

- [ ] **Step 5: Run the backend suite**

Run: `cd src-tauri && cargo test`
Expected: all tests pass (Task 1 + Task 2's tests plus every pre-existing test). Run `cargo build` to confirm the whole binary compiles — this task touches the most call sites of the four, so a missed reference is the likely failure mode; the compiler will name every site.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/jsonl_parser/watcher.rs src-tauri/src/lib.rs src-tauri/src/app_state.rs src-tauri/src/commands.rs
git commit -m "feat(live-sessions): wire the registry into the watcher, AppState, and IPC"
```

---

### Task 4: Frontend — store, events, and the "Now running" section

**Files:**
- Modify: `src/lib/generated/bindings.ts` (new `LiveSessionInfo` type, alphabetically placed)
- Modify: `src/lib/types.ts` (re-export, alphabetically placed)
- Modify: `src/lib/ipc.ts` (new `getLiveSessions` wrapper)
- Modify: `src/lib/events.ts` (new `live_sessions_changed` event)
- Modify: `src/lib/store.ts` (new `liveSessions` field + init fetch + event case)
- Create: `src/popover/NowRunningSection.tsx` (+ test)
- Modify: `src/popover/CompactPopover.tsx` (render the new section)

**Interfaces:**
- Consumes: `get_live_sessions` IPC command and `live_sessions_changed` event from Task 3; `formatDurationMinutes` (already exists in `src/lib/format.ts`); `shortName`/`modelKey` from `src/report/modelDisplay.ts`; `formatCost` from `src/lib/format.ts`.
- Produces: `useAppStore((s) => s.liveSessions)` — `LiveSessionInfo[]`, always the current Live set.

- [ ] **Step 1: Bindings + types + ipc + events**

`src/lib/generated/bindings.ts` — insert alphabetically (this file is fully alphabetically sorted, confirmed by the PAYG feature's final review):

```ts
export type LiveSessionInfo = { session_id: string; source_file: string; project: string; model: string; total_tokens: number; total_cost_usd: number; context_tokens: number; first_seen: number; last_activity: number }
```

`src/lib/types.ts` — add `LiveSessionInfo` to the `export type { ... } from './generated/bindings';` list, alphabetically.

`src/lib/ipc.ts` — add near `getSessionHistory`:

```ts
  getLiveSessions: () => commands.getLiveSessions().then(unwrap),
```

`src/lib/events.ts` — add to the `AppEvent` union and the `subscribe` function:

```ts
  | { type: "live_sessions_changed"; payload: LiveSessionInfo[] }
```

```ts
    listen<LiveSessionInfo[]>("live_sessions_changed", (e) =>
      handler({ type: "live_sessions_changed", payload: e.payload }),
    ),
```

Add `LiveSessionInfo` to the `import type { ... } from "./generated/bindings";` list at the top of `events.ts`.

- [ ] **Step 2: Store wiring**

`src/lib/store.ts`:
- Add to the `AppStore` interface: `liveSessions: LiveSessionInfo[];`
- Add `LiveSessionInfo` to the top-level `import type { ... } from './generated/bindings';`.
- Add `liveSessions: [],` to the store's initial state (alongside `sessionDataVersion: 0,`).
- In `init()`, add `ipc.getLiveSessions().catch(() => [])` to the existing `Promise.all([...])` and destructure it, then include it in the `set({ usage, settings, accounts, activeSlot: active, liveSessions })` call. (Check the exact current `Promise.all` shape before editing — it destructures `[usage, settings, accounts]` today.)
- In the `subscribe` switch, add a case:

```ts
        case 'live_sessions_changed':
          set({ liveSessions: e.payload });
          break;
```

- [ ] **Step 3: Write the failing component tests**

Create `src/popover/__tests__/NowRunningSection.test.tsx`:

```tsx
import { render, screen } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import type { LiveSessionInfo } from '../../lib/types';
import { NowRunningSection } from '../NowRunningSection';

function session(overrides: Partial<LiveSessionInfo>): LiveSessionInfo {
  const now = Math.floor(Date.now() / 1000);
  return {
    session_id: 's1',
    source_file: '-proj/s1.jsonl',
    project: 'my-project',
    model: 'claude-opus-5',
    total_tokens: 12000,
    total_cost_usd: 1.84,
    context_tokens: 5000,
    first_seen: now - 12 * 60,
    last_activity: now,
    ...overrides,
  };
}

describe('NowRunningSection', () => {
  it('renders nothing when there are no live sessions', () => {
    const { container } = render(<NowRunningSection sessions={[]} />);
    expect(container.firstChild).toBeNull();
  });

  it('renders a row per session with project, model, cost, and elapsed', () => {
    render(
      <NowRunningSection
        sessions={[
          session({ session_id: 'a', project: 'proj-a', total_cost_usd: 1.84, first_seen: Math.floor(Date.now() / 1000) - 12 * 60 }),
          session({ session_id: 'b', project: 'proj-b', total_cost_usd: 0.02 }),
        ]}
      />,
    );
    expect(screen.getByText('proj-a')).toBeTruthy();
    expect(screen.getByText('proj-b')).toBeTruthy();
    expect(screen.getByText('$1.84')).toBeTruthy();
    expect(screen.getByText(/12m/)).toBeTruthy();
  });

  it('caps at 3 rows and shows a "+N more" line beyond that', () => {
    const sessions = Array.from({ length: 5 }, (_, i) => session({ session_id: `s${i}`, project: `proj-${i}` }));
    render(<NowRunningSection sessions={sessions} />);
    expect(screen.getByText('proj-0')).toBeTruthy();
    expect(screen.getByText('proj-1')).toBeTruthy();
    expect(screen.getByText('proj-2')).toBeTruthy();
    expect(screen.queryByText('proj-3')).toBeNull();
    expect(screen.getByText(/\+2 more/)).toBeTruthy();
  });
});
```

- [ ] **Step 4: Run to verify it fails**

Run: `pnpm exec vitest run src/popover/__tests__/NowRunningSection.test.tsx`
Expected: FAIL — `../NowRunningSection` doesn't exist yet.

- [ ] **Step 5: Implement `NowRunningSection`**

Create `src/popover/NowRunningSection.tsx`:

```tsx
import type { LiveSessionInfo } from '../lib/types';
import { formatCost } from '../lib/format';
import { formatDurationMinutes } from '../lib/format';
import { shortName } from '../report/modelDisplay';

const MAX_ROWS = 3;

function elapsedLabel(firstSeen: number): string {
  const mins = Math.max(0, Math.floor((Date.now() / 1000 - firstSeen) / 60));
  return formatDurationMinutes(mins);
}

function Row({ session }: { session: LiveSessionInfo }) {
  return (
    <div className="flex items-center gap-[var(--space-xs)] px-[var(--popover-pad)] py-[2px]">
      <span className="flex-1 min-w-0 truncate text-[length:var(--text-micro)] text-[color:var(--color-text)]">
        {session.project}
      </span>
      <span className="shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] uppercase">
        {shortName(session.model)}
      </span>
      <span className="mono shrink-0 text-[length:var(--text-micro)] tabular-nums text-[color:var(--color-text-secondary)]">
        {formatCost(session.total_cost_usd)}
      </span>
      <span className="mono shrink-0 text-[length:var(--text-micro)] tabular-nums text-[color:var(--color-text-muted)]">
        {elapsedLabel(session.first_seen)}
      </span>
    </div>
  );
}

export function NowRunningSection({ sessions }: { sessions: LiveSessionInfo[] }) {
  if (sessions.length === 0) return null;
  const shown = sessions.slice(0, MAX_ROWS);
  const overflow = sessions.length - shown.length;
  return (
    <div className="flex flex-col gap-[2px] border-t border-[var(--color-rule)] py-[var(--space-2xs)]">
      <span className="px-[var(--popover-pad)] text-[length:var(--text-micro)] font-[var(--weight-semibold)] text-[color:var(--color-text-muted)] uppercase tracking-[var(--tracking-label)]">
        Now running
      </span>
      {shown.map((s) => (
        <Row key={s.session_id} session={s} />
      ))}
      {overflow > 0 && (
        <span className="px-[var(--popover-pad)] text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          +{overflow} more
        </span>
      )}
    </div>
  );
}
```

(Fix the `gap-[2px]` to `gap-[var(--space-2xs)]` if that token isn't already what you used — this codebase requires design tokens only, no hard-coded pixel values; the PAYG feature's final review caught exactly this mistake once already.)

- [ ] **Step 6: Run to verify it passes**

Run: `pnpm exec vitest run src/popover/__tests__/NowRunningSection.test.tsx`
Expected: PASS (3 tests).

- [ ] **Step 7: Wire into `CompactPopover.tsx`**

Add the import: `import { NowRunningSection } from './NowRunningSection';`

Add `const liveSessions = useAppStore((s) => s.liveSessions);` alongside the component's other `useAppStore` reads near the top of `CompactPopover()`.

Render it after `<UsageSummary .../>` and before the footer `<div style={{ marginTop: 'auto' }} ...>` block:

```tsx
      <NowRunningSection sessions={liveSessions} />
```

- [ ] **Step 8: Run the full frontend gate**

Run: `pnpm lint` → clean. Run: `pnpm test` → no new failures beyond the known pre-existing `theme.test.ts` collection error.

- [ ] **Step 9: Commit**

```bash
git add src/lib/generated/bindings.ts src/lib/types.ts src/lib/ipc.ts src/lib/events.ts src/lib/store.ts src/popover/NowRunningSection.tsx src/popover/__tests__/NowRunningSection.test.tsx src/popover/CompactPopover.tsx
git commit -m "feat(popover): add the Now running section"
```

---

## Verification checklist (after all tasks)

- `cargo test` (from `src-tauri/`) fully green; `pnpm lint` clean; `pnpm test` no new failures.
- Manual: start a real Claude Code session in a terminal, open the popover — within ~2s of the first tool call, a "Now running" section appears with project/model/cost/elapsed; leave it idle 2+ minutes and it disappears from the popover on the next prune tick (up to 30s after the 120s mark); the startup backfill (relaunching the app with old, already-finished sessions on disk) must NOT populate the section.
- No DB schema change; `git diff` touches no `schema.sql` or `migrations/` files.
