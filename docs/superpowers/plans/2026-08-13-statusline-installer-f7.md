# F7 — Statusline installer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One-click install of a Switchboard-provided `switchboard statusline` command into `~/.claude/settings.json`, showing 5-hour rate-limit usage % in the terminal prompt, with the same guarded-write posture (backup, undo, foreign-value confirmation) the custom-model-providers feature already established for the same file.

**Architecture:** A new `statusline_installer` module performs a guarded, single-key `statusLine` write/undo, built on `providers::default_env`'s existing file-I/O primitives (widened from private to `pub(crate)`, not duplicated). The poll loop gains a small writer for the pre-existing (currently read-only) shared-snapshot file. A new `switchboard statusline` CLI subcommand reads that file and prints one line. Three new Tauri commands wire install/uninstall/status to a new Settings UI section.

**Tech Stack:** Rust (rusqlite, serde_json, tauri-specta), React 19 + TypeScript, Vitest + Testing Library.

**Spec:** `docs/superpowers/specs/2026-08-13-statusline-installer-f7-design.md` — read it first; this plan implements it task-by-task, with one correction found during planning (see Task 4).

## Global Constraints

- Every mutation of `~/.claude/settings.json` must go through the existing backup + atomic-write + concurrent-writer-detection machinery in `providers/default_env.rs` — no parallel file-write path.
- The statusline only works while the Switchboard GUI is running. Missing/stale shared-snapshot data must print an honest "Switchboard: not running" placeholder, never a stale or wrong-looking number.
- A pre-existing, non-Switchboard-owned `statusLine` value must never be silently overwritten — the install command must return `NeedsConfirmation` and the frontend must confirm before retrying with `force=true`, mirroring `set_default_provider`'s exact pattern.
- No parsing of the session-context JSON Claude Code pipes via stdin — V1 drains and discards it.
- Scoped to 5H% only, not 7D or any other bucket.

---

## Task 1: Widen `default_env.rs`'s file-I/O primitives to `pub(crate)`

**Files:**
- Modify: `src-tauri/src/providers/default_env.rs`

**Interfaces:**
- Produces: `pub(crate) fn read_settings(path: &Path) -> Result<Map<String, Value>>`, `pub(crate) fn backup(path: &Path) -> Result<()>`, `pub(crate) fn stamp(path: &Path) -> Result<Option<FileStamp>>`, `pub(crate) fn write_atomic(path: &Path, map: &Map<String, Value>, expected: Option<FileStamp>) -> Result<()>`, `pub(crate) struct FileStamp` (fields stay private — callers only move the value opaquely between `stamp` and `write_atomic`, never construct or read it). Task 2 imports and calls all four.

This is a pure visibility change — no behavior change, no new code, no new tests (the existing test suite is the regression check).

- [ ] **Step 1: Widen the four functions and the struct**

In `src-tauri/src/providers/default_env.rs`, change:
```rust
fn read_settings(path: &Path) -> Result<Map<String, Value>> {
```
to:
```rust
pub(crate) fn read_settings(path: &Path) -> Result<Map<String, Value>> {
```

Change:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
```
to:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileStamp {
```

Change:
```rust
fn stamp(path: &Path) -> Result<Option<FileStamp>> {
```
to:
```rust
pub(crate) fn stamp(path: &Path) -> Result<Option<FileStamp>> {
```

Change:
```rust
fn backup(path: &Path) -> Result<()> {
```
to:
```rust
pub(crate) fn backup(path: &Path) -> Result<()> {
```

Change:
```rust
fn write_atomic(path: &Path, map: &Map<String, Value>, expected: Option<FileStamp>) -> Result<()> {
```
to:
```rust
pub(crate) fn write_atomic(path: &Path, map: &Map<String, Value>, expected: Option<FileStamp>) -> Result<()> {
```

Leave `backup_path`, `prune_backups`, and `env_object` as module-private (`fn`, no `pub(crate)`) — Task 2 doesn't need them; `backup` already calls `prune_backups` internally, and `env_object` is specific to the `env`-map shape this feature doesn't use.

- [ ] **Step 2: Run the existing test suite to confirm no regression**

Run: `cd src-tauri && cargo test default_env`
Expected: PASS — same tests, same count, as before this change (visibility widening cannot change behavior, only compile errors would indicate a mistake).

- [ ] **Step 3: Run the full backend suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/providers/default_env.rs
git commit -m "refactor(providers): widen default_env's file-I/O primitives to pub(crate)

Statusline installer (F7) needs to reuse the same guarded-write machinery
for a single statusLine key instead of duplicating it. Pure visibility
change, no behavior change."
```

---

## Task 2: `statusline_installer.rs` — guarded file apply/clear

**Files:**
- Create: `src-tauri/src/statusline_installer.rs`
- Modify: `src-tauri/src/lib.rs` (register the new module)

**Interfaces:**
- Consumes: `crate::providers::default_env::{read_settings, backup, stamp, write_atomic, FileStamp}` (Task 1).
- Produces: `pub fn apply(path: &Path, command: &str) -> Result<Option<Value>>`, `pub fn clear(path: &Path, prior: &Option<Value>, written: &Value) -> Result<bool>`. Task 6's Tauri commands call both.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/statusline_installer.rs` with just the test module first:

```rust
//! Guarded, single-key write/undo for `~/.claude/settings.json`'s
//! `statusLine` field. Mirrors `providers::default_env`'s `apply`/`clear`
//! shape, but for one object-valued top-level key instead of a flat map
//! merged into `env`.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use tempfile::tempdir;

    fn write(path: &std::path::Path, s: &str) {
        std::fs::write(path, s).unwrap();
    }

    #[test]
    fn apply_creates_statusline_when_settings_missing() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        let prior = apply(&p, "/usr/local/bin/switchboard statusline").unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["statusLine"]["type"], "command");
        assert_eq!(v["statusLine"]["command"], "/usr/local/bin/switchboard statusline");
        assert_eq!(prior, None);
    }

    #[test]
    fn apply_preserves_unrelated_keys_and_reports_prior_statusline() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(
            &p,
            r#"{
          "hooks": {"PreToolUse": [{"matcher": "Bash"}]},
          "statusLine": {"type": "command", "command": "bash x.sh"},
          "model": "opus"
        }"#,
        );
        let prior = apply(&p, "/usr/local/bin/switchboard statusline").unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        assert_eq!(v["model"], "opus");
        assert_eq!(v["statusLine"]["command"], "/usr/local/bin/switchboard statusline");
        assert_eq!(prior, Some(json!({"type": "command", "command": "bash x.sh"})));
    }

    #[test]
    fn clear_removes_the_key_when_it_was_absent_before() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        let written = json!({"type": "command", "command": "/usr/local/bin/switchboard statusline"});
        apply(&p, "/usr/local/bin/switchboard statusline").unwrap();
        let ok = clear(&p, &None, &written).unwrap();
        assert!(ok);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(v.get("statusLine").is_none());
    }

    #[test]
    fn clear_restores_the_prior_value_when_one_existed() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"statusLine": {"type": "command", "command": "bash x.sh"}}"#);
        let prior = apply(&p, "/usr/local/bin/switchboard statusline").unwrap();
        let written = json!({"type": "command", "command": "/usr/local/bin/switchboard statusline"});
        let ok = clear(&p, &prior, &written).unwrap();
        assert!(ok);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["statusLine"]["command"], "bash x.sh");
    }

    #[test]
    fn clear_skips_and_reports_false_when_the_user_changed_it_since() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        apply(&p, "/usr/local/bin/switchboard statusline").unwrap();
        // User hand-edits it after Switchboard installed its own.
        write(&p, r#"{"statusLine": {"type": "command", "command": "bash hand-edited.sh"}}"#);
        let written = json!({"type": "command", "command": "/usr/local/bin/switchboard statusline"});
        let ok = clear(&p, &None, &written).unwrap();
        assert!(!ok);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["statusLine"]["command"], "bash hand-edited.sh");
    }
}
```

- [ ] **Step 2: Register the module**

In `src-tauri/src/lib.rs`, add to the module declaration list (alphabetically near `pub mod store;` / `mod tray;` is fine, exact position doesn't matter):
```rust
pub mod statusline_installer;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd src-tauri && cargo test statusline_installer`
Expected: FAIL to compile — `apply`/`clear` don't exist yet.

- [ ] **Step 4: Write the minimal implementation**

Add above the test module in `src-tauri/src/statusline_installer.rs`:

```rust
use crate::providers::default_env::{backup, read_settings, stamp, write_atomic};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::Path;

/// Write `{"type": "command", "command": command}` as `statusLine`. Returns
/// the prior value (`None` if the key was absent) as the undo record.
pub fn apply(path: &Path, command: &str) -> Result<Option<Value>> {
    let before = stamp(path)?;
    let mut settings = read_settings(path)?;
    let prior = settings.get("statusLine").cloned();

    backup(path)?;
    settings.insert(
        "statusLine".to_string(),
        json!({ "type": "command", "command": command }),
    );
    write_atomic(path, &settings, before)?;
    Ok(prior)
}

/// Restore `prior` (or remove the key if `prior` is `None`). Drift check: if
/// the current `statusLine` no longer equals `written` (the value `apply`
/// last set), the user or another tool changed it since — leave it alone and
/// return `false` rather than silently reverting their edit.
pub fn clear(path: &Path, prior: &Option<Value>, written: &Value) -> Result<bool> {
    let before = stamp(path)?;
    let mut settings = read_settings(path)?;
    let current = settings.get("statusLine");

    if current != Some(written) {
        return Ok(false);
    }

    backup(path)?;
    match prior {
        Some(v) => {
            settings.insert("statusLine".to_string(), v.clone());
        }
        None => {
            settings.remove("statusLine");
        }
    }
    write_atomic(path, &settings, before)?;
    Ok(true)
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test statusline_installer`
Expected: PASS (5 tests)

- [ ] **Step 6: Run the full backend suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/statusline_installer.rs src-tauri/src/lib.rs
git commit -m "feat(statusline): add guarded statusLine apply/clear"
```

---

## Task 3: DB migration + singleton install-state row

**Files:**
- Modify: `src-tauri/src/store/schema.sql`
- Create: `src-tauri/src/store/migrations/0011_statusline_install.sql`
- Modify: `src-tauri/src/store/mod.rs` (migrate() function, `create_fresh_db`'s stamped version, final schema_version insert)
- Modify: `src-tauri/src/statusline_installer.rs` (DB query methods + struct + tests)

**Interfaces:**
- Produces: table `statusline_install(id, prior_value, installed_command, installed_at)`; struct `StatuslineInstallState { installed_command: String, installed_at: i64 }` (specta-typed, for the frontend); `Db` methods `get_statusline_install(&self) -> Result<Option<(StatuslineInstallState, Option<Value>)>>` (state + the raw prior-value for `clear`), `set_statusline_install(&self, prior: &Option<Value>, command: &str, installed_at: i64) -> Result<()>`, `clear_statusline_install(&self) -> Result<()>`. Task 6's commands call all three.

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/src/statusline_installer.rs`'s test module (needs `Db` in scope):

```rust
    use crate::store::Db;

    fn fresh_db() -> (tempfile::TempDir, Db) {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        (dir, db)
    }

    #[test]
    fn statusline_install_roundtrips_through_the_db() {
        let (_dir, db) = fresh_db();
        assert!(db.get_statusline_install().unwrap().is_none());

        let prior = Some(json!({"type": "command", "command": "bash x.sh"}));
        db.set_statusline_install(&prior, "/usr/local/bin/switchboard statusline", 1_700_000_000)
            .unwrap();

        let (state, got_prior) = db.get_statusline_install().unwrap().expect("row present");
        assert_eq!(state.installed_command, "/usr/local/bin/switchboard statusline");
        assert_eq!(state.installed_at, 1_700_000_000);
        assert_eq!(got_prior, prior);

        db.clear_statusline_install().unwrap();
        assert!(db.get_statusline_install().unwrap().is_none());
    }

    #[test]
    fn set_statusline_install_overwrites_the_singleton_row() {
        let (_dir, db) = fresh_db();
        db.set_statusline_install(&None, "first", 1).unwrap();
        db.set_statusline_install(&None, "second", 2).unwrap();
        let (state, _) = db.get_statusline_install().unwrap().expect("row present");
        assert_eq!(state.installed_command, "second");
        assert_eq!(state.installed_at, 2);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test statusline_install_roundtrips`
Expected: FAIL to compile — `statusline_install` table and the `Db` methods don't exist yet.

- [ ] **Step 3: Add the table to `schema.sql`** (fresh installs)

In `src-tauri/src/store/schema.sql`, after the `window_peaks` block (after its `CREATE UNIQUE INDEX idx_window_peaks_identity` line), add:

```sql
-- Singleton row recording Switchboard's own statusLine install, mirroring
-- provider_default's shape. prior_value is the statusLine JSON that existed
-- before install (NULL if absent) — the undo record. installed_command is
-- the exact command string written, used for drift detection on uninstall.
CREATE TABLE IF NOT EXISTS statusline_install (
    id                 INTEGER PRIMARY KEY CHECK (id = 1),
    prior_value        TEXT,
    installed_command  TEXT NOT NULL,
    installed_at       INTEGER NOT NULL
);
```

- [ ] **Step 4: Create the migration file** (existing installs)

Create `src-tauri/src/store/migrations/0011_statusline_install.sql`:

```sql
-- v10 → v11: track Switchboard's own statusLine install (F7). Additive
-- only — no existing table changes.
CREATE TABLE IF NOT EXISTS statusline_install (
    id                 INTEGER PRIMARY KEY CHECK (id = 1),
    prior_value        TEXT,
    installed_command  TEXT NOT NULL,
    installed_at       INTEGER NOT NULL
);
```

- [ ] **Step 5: Wire the migration into `migrate()`**

In `src-tauri/src/store/mod.rs`, inside `fn migrate()`, after the `if current < 10 { ... }` block, add:

```rust
        if current < 11 {
            tracing::info!("migrating v10 -> v11 (statusline_install for F7)");
            conn.execute_batch(include_str!("migrations/0011_statusline_install.sql"))
                .context("apply migration 0011")?;
        }
```

Update the two version-stamping sites: in `create_fresh_db`, change `[10_i64]` to `[11_i64]` and its doc comment's "schema_version=10" to "schema_version=11". At the end of `migrate()`, change `[10_i64]` to `[11_i64]`.

- [ ] **Step 6: Add the struct and `Db` methods**

Add to `src-tauri/src/statusline_installer.rs`, above the test module:

```rust
use crate::store::Db;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// What Switchboard wrote as `statusLine`, for the settings UI to display.
/// The prior value (the undo record) is not part of this — it's an
/// implementation detail `get_statusline_install` returns alongside it for
/// `clear`, not something the frontend needs to render.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct StatuslineInstallState {
    pub installed_command: String,
    pub installed_at: i64,
}

impl Db {
    pub fn get_statusline_install(&self) -> Result<Option<(StatuslineInstallState, Option<Value>)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT prior_value, installed_command, installed_at FROM statusline_install WHERE id = 1",
        )?;
        let row = stmt
            .query_row([], |r| {
                let prior_value: Option<String> = r.get(0)?;
                let installed_command: String = r.get(1)?;
                let installed_at: i64 = r.get(2)?;
                Ok((prior_value, installed_command, installed_at))
            })
            .optional()?;
        let Some((prior_value, installed_command, installed_at)) = row else {
            return Ok(None);
        };
        let prior: Option<Value> = prior_value.and_then(|s| serde_json::from_str(&s).ok());
        Ok(Some((StatuslineInstallState { installed_command, installed_at }, prior)))
    }

    pub fn set_statusline_install(
        &self,
        prior: &Option<Value>,
        command: &str,
        installed_at: i64,
    ) -> Result<()> {
        let prior_json = match prior {
            Some(v) => Some(serde_json::to_string(v).context("serialize prior statusLine")?),
            None => None,
        };
        self.conn().execute(
            "INSERT INTO statusline_install (id, prior_value, installed_command, installed_at)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               prior_value = excluded.prior_value,
               installed_command = excluded.installed_command,
               installed_at = excluded.installed_at",
            params![prior_json, command, installed_at],
        )?;
        Ok(())
    }

    pub fn clear_statusline_install(&self) -> Result<()> {
        self.conn()
            .execute("DELETE FROM statusline_install WHERE id = 1", [])?;
        Ok(())
    }
}
```

Add `use anyhow::Context;` to the top-of-file imports (needed for `.context(...)` above) — change:
```rust
use anyhow::Result;
```
to:
```rust
use anyhow::{Context, Result};
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cd src-tauri && cargo test statusline`
Expected: PASS (7 tests total in this file — 5 from Task 2, 2 new)

- [ ] **Step 8: Run the full backend suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/store/schema.sql src-tauri/src/store/migrations/0011_statusline_install.sql src-tauri/src/store/mod.rs src-tauri/src/statusline_installer.rs
git commit -m "feat(db): add statusline_install table + Db methods"
```

---

## Task 4: Poll-loop writer for the shared-snapshot file

**Files:**
- Modify: `src-tauri/src/poll_loop.rs`

**Interfaces:**
- Produces: `pub(crate) fn write_shared_snapshot(path: &Path, snapshot: &UsageSnapshot) -> Result<()>`. Wired into `apply_fetch_outcome`, active-slot-gated only.
- Also widens `shared_usage_file_path` from private to `pub(crate)` — Task 5's CLI subcommand calls it directly so both the writer and the reader always agree on the file's location.

**Correction to the spec found while planning:** the spec's §3a says to serialize `UsageSnapshot` "as-is, already `Serialize`." That's not quite right — `read_shared_snapshot` (`poll_loop.rs:595-632`) expects the file's top-level `fetched_at` to be a **bare epoch-seconds integer** (`obj.remove("fetched_at")?.as_i64()?`), but `UsageSnapshot::fetched_at` is a `DateTime<Utc>` that serializes natively as an RFC3339 **string**. Serializing the struct directly would produce a `fetched_at` the reader's `.as_i64()` call rejects, silently breaking every snapshot the writer produces. The fix: serialize to a `serde_json::Value`, then overwrite the `fetched_at` key with an epoch-integer before writing.

- [ ] **Step 1: Write the failing test**

Add to the `mod shared_snapshot { ... }` test module in `poll_loop.rs` (it already has `write`/`payload` helpers and imports `read_shared_snapshot`):

```rust
        #[test]
        fn write_shared_snapshot_round_trips_through_read_shared_snapshot() {
            let dir = tempdir().unwrap();
            let p = dir.path().join("statusline-usage.json");
            let snap: UsageSnapshot = serde_json::from_str(
                r#"{"five_hour": {"utilization": 55.0, "resets_at": "2026-04-24T18:00:00Z"}, "seven_day": null}"#,
            )
            .unwrap();

            write_shared_snapshot(&p, &snap).unwrap();

            let read_back = read_shared_snapshot(&p, Duration::from_secs(120), None)
                .expect("just-written snapshot must be readable back");
            assert_eq!(read_back.five_hour.unwrap().utilization, 55.0);
        }

        #[test]
        fn write_shared_snapshot_stamps_fetched_at_as_an_epoch_integer() {
            let dir = tempdir().unwrap();
            let p = dir.path().join("statusline-usage.json");
            let snap: UsageSnapshot = serde_json::from_str(r#"{"five_hour": null, "seven_day": null}"#).unwrap();

            write_shared_snapshot(&p, &snap).unwrap();

            let raw = std::fs::read_to_string(&p).unwrap();
            let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
            assert!(
                v["fetched_at"].is_i64() || v["fetched_at"].is_u64(),
                "fetched_at must be a bare epoch-seconds integer, got: {:?}",
                v["fetched_at"]
            );
        }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test write_shared_snapshot`
Expected: FAIL to compile — `write_shared_snapshot` doesn't exist yet.

- [ ] **Step 3: Write the minimal implementation**

In `src-tauri/src/poll_loop.rs`, change:
```rust
fn shared_usage_file_path() -> std::path::PathBuf {
```
to:
```rust
pub(crate) fn shared_usage_file_path() -> std::path::PathBuf {
```

`read_shared_snapshot` currently ends at line 632 (`Some(snap)` then a closing `}`), immediately followed by the doc comment for `pub fn hydrated_caches`. Add the new function in between those two, right after `read_shared_snapshot`'s closing `}` and before `hydrated_caches`'s doc comment:

```rust
/// Write the active account's snapshot to the shared-usage file, in the
/// exact format `read_shared_snapshot` parses. `fetched_at` must be a bare
/// epoch-seconds integer at the top level — `UsageSnapshot`'s own
/// `fetched_at` field serializes as an RFC3339 string, which the reader's
/// `.as_i64()` call would reject, so it's stripped and replaced rather than
/// serialized as-is.
pub(crate) fn write_shared_snapshot(
    path: &std::path::Path,
    snapshot: &UsageSnapshot,
) -> anyhow::Result<()> {
    let mut value = serde_json::to_value(snapshot)?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("fetched_at".to_string(), serde_json::json!(Utc::now().timestamp()));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, serde_json::to_string(&value)?)?;
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test write_shared_snapshot`
Expected: PASS (2 tests)

- [ ] **Step 5: Wire the writer into `apply_fetch_outcome`**

In `src-tauri/src/poll_loop.rs`'s `apply_fetch_outcome`, the `if Some(slot) == active_slot { ... }` block (the same block the tray update and notifier evaluation already live in — active-slot only, since the shared-snapshot file has no account identity and can only ever describe the one account Claude Code sessions are authenticated as) currently opens with `*state.cached_usage.write() = Some(cached.clone());` as its first statement. Add the write immediately before that line, as the new first statement in the block:

```rust
                if let Err(e) = write_shared_snapshot(&shared_usage_file_path(), &snapshot) {
                    tracing::warn!("write_shared_snapshot failed: {e:#}");
                }
```

Best-effort, same posture as every other write on this path — never interrupts polling.

- [ ] **Step 6: Run the full backend suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/poll_loop.rs
git commit -m "feat(statusline): write the shared-snapshot file from the poll loop

Adds the writer half of infrastructure that previously only had a reader
(built for a third-party statusline daemon's output). fetched_at must be
a bare epoch-seconds integer, not UsageSnapshot's native RFC3339 string —
see the round-trip test."
```

---

## Task 5: `switchboard statusline` CLI subcommand

**Files:**
- Modify: `src-tauri/src/cli.rs`
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Consumes: `crate::poll_loop::{read_shared_snapshot, shared_usage_file_path}` (Task 4).
- Produces: `CliMode::Statusline` variant, `pub async fn run_statusline() -> String` (returns the line to print — kept separate from the actual `println!`/process-exit so it's unit-testable without capturing stdout).

- [ ] **Step 1: Write the failing tests**

Add to `cli.rs`'s existing `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn parses_statusline_subcommand() {
        assert_eq!(
            parse_args(["claude-switchboard", "statusline"]),
            CliMode::Statusline,
        );
    }
```

Add a new test module (needs its own file-based setup, mirroring `poll_loop.rs`'s `shared_snapshot` test pattern):

```rust
#[cfg(test)]
mod statusline_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn prints_the_five_hour_percentage_when_the_snapshot_is_fresh() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("statusline-usage.json");
        let now = chrono::Utc::now().timestamp();
        std::fs::write(
            &p,
            format!(
                r#"{{"five_hour": {{"utilization": 42.5, "resets_at": null}}, "seven_day": null, "fetched_at": {now}}}"#
            ),
        )
        .unwrap();

        let line = run_statusline_for_path(&p);
        assert_eq!(line, "5H 43%");
    }

    #[test]
    fn reports_not_running_when_the_file_is_missing() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("does-not-exist.json");
        assert_eq!(run_statusline_for_path(&p), "Switchboard: not running");
    }

    #[test]
    fn reports_not_running_when_the_snapshot_is_stale() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("statusline-usage.json");
        let old = (chrono::Utc::now() - chrono::Duration::minutes(30)).timestamp();
        std::fs::write(
            &p,
            format!(
                r#"{{"five_hour": {{"utilization": 42.5, "resets_at": null}}, "seven_day": null, "fetched_at": {old}}}"#
            ),
        )
        .unwrap();
        assert_eq!(run_statusline_for_path(&p), "Switchboard: not running");
    }

    #[test]
    fn reports_not_running_when_five_hour_is_absent() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("statusline-usage.json");
        let now = chrono::Utc::now().timestamp();
        std::fs::write(
            &p,
            format!(r#"{{"five_hour": null, "seven_day": null, "fetched_at": {now}}}"#),
        )
        .unwrap();
        assert_eq!(run_statusline_for_path(&p), "Switchboard: not running");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test statusline`
Expected: FAIL to compile — `CliMode::Statusline` and `run_statusline_for_path` don't exist yet.

- [ ] **Step 3: Write the minimal implementation**

In `src-tauri/src/cli.rs`, change:
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliMode {
    Tick,
    Migrate,
    Gui, // default — start the Tauri runtime as usual
}

pub fn parse_args<I, S>(args: I) -> CliMode
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    for a in args {
        match a.as_ref() {
            "--tick" => return CliMode::Tick,
            "--migrate" => return CliMode::Migrate,
            _ => {}
        }
    }
    CliMode::Gui
}
```
to:
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliMode {
    Tick,
    Migrate,
    Statusline,
    Gui, // default — start the Tauri runtime as usual
}

pub fn parse_args<I, S>(args: I) -> CliMode
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    for a in args {
        match a.as_ref() {
            "--tick" => return CliMode::Tick,
            "--migrate" => return CliMode::Migrate,
            "statusline" => return CliMode::Statusline,
            _ => {}
        }
    }
    CliMode::Gui
}
```

Add, after `run_tick`'s closing brace:

```rust
/// Freshness window for the shared-snapshot file. A headless one-shot
/// invocation has no `Settings.polling_interval_secs` to read (no DB, no
/// AppState) — a fixed, generous constant errs toward "shows a number a
/// few minutes longer than strictly necessary after Switchboard quits"
/// rather than plumbing settings into a process that must stay fast and
/// simple (Claude Code invokes this on every prompt render).
const STATUSLINE_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(600);

/// Core logic for `run_statusline`, taking the shared-snapshot path
/// explicitly so it's testable without touching the real `~/.claude/`
/// directory. `run_statusline` calls this with the real path.
fn run_statusline_for_path(path: &Path) -> String {
    match crate::poll_loop::read_shared_snapshot(path, STATUSLINE_MAX_AGE, None) {
        Some(snap) => match snap.five_hour {
            Some(u) => format!("5H {}%", u.utilization.round() as i64),
            None => "Switchboard: not running".to_string(),
        },
        None => "Switchboard: not running".to_string(),
    }
}

/// Run `statusline`. Drains stdin (Claude Code pipes session-context JSON
/// in) without parsing it — out of scope for V1, which only shows 5H%.
/// Prints exactly one line to stdout and always exits 0: Claude Code
/// renders whatever this command prints, so erroring would show nothing
/// useful rather than the honest "not running" placeholder.
pub async fn run_statusline() {
    use std::io::Read as _;
    let mut discard = String::new();
    let _ = std::io::stdin().read_to_string(&mut discard);

    let path = crate::poll_loop::shared_usage_file_path();
    println!("{}", run_statusline_for_path(&path));
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test statusline`
Expected: PASS (5 tests: 1 parse test + 4 in `statusline_tests`)

- [ ] **Step 5: Wire the dispatch in `main.rs`**

In `src-tauri/src/main.rs`, change:
```rust
        claude_switchboard_lib::cli::CliMode::Migrate
        | claude_switchboard_lib::cli::CliMode::Gui => {
            claude_switchboard_lib::run();
        }
```
to:
```rust
        claude_switchboard_lib::cli::CliMode::Statusline => {
            let rt = tokio::runtime::Runtime::new().expect("tokio rt");
            rt.block_on(claude_switchboard_lib::cli::run_statusline());
        }
        claude_switchboard_lib::cli::CliMode::Migrate
        | claude_switchboard_lib::cli::CliMode::Gui => {
            claude_switchboard_lib::run();
        }
```

- [ ] **Step 6: Run the full backend suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/cli.rs src-tauri/src/main.rs
git commit -m "feat(statusline): add switchboard statusline CLI subcommand"
```

---

## Task 6: Tauri commands + bindings

**Files:**
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/src/lib.rs` (both `collect_commands!` blocks)

**Interfaces:**
- Consumes: `crate::statusline_installer::{apply, clear, StatuslineInstallState}` (Tasks 2/3), `state.db.{get_statusline_install, set_statusline_install, clear_statusline_install}` (Task 3), `claude_settings_path()` (existing helper, `commands.rs:1468`).
- Produces: `get_statusline_install_state() -> Option<StatuslineInstallState>`, `install_statusline(force: bool) -> InstallStatuslineOutcome`, `uninstall_statusline() -> bool`. Task 7's `ipc.ts` wraps all three.

- [ ] **Step 1: Add the commands**

In `src-tauri/src/commands.rs`, near `get_default_provider`/`set_default_provider`/`clear_default_provider` (same "external config write" section), add:

```rust
#[derive(Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum InstallStatuslineOutcome {
    Applied,
    /// `settings.json` already carries a `statusLine` we do not own. The UI
    /// must confirm before we overwrite hand-written (or another tool's)
    /// configuration.
    NeedsConfirmation { foreign_value: serde_json::Value },
}

#[command]
#[specta::specta]
pub async fn get_statusline_install_state(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<crate::statusline_installer::StatuslineInstallState>, String> {
    Ok(state
        .db
        .get_statusline_install()
        .map_err(|e| e.to_string())?
        .map(|(s, _)| s))
}

#[command]
#[specta::specta]
pub async fn install_statusline(
    force: bool,
    state: State<'_, Arc<AppState>>,
) -> Result<InstallStatuslineOutcome, String> {
    let path = claude_settings_path()?;
    let existing = state.db.get_statusline_install().map_err(|e| e.to_string())?;

    // ORDER IS LOAD-BEARING, same reasoning as set_default_provider (spec
    // §4.1 of the providers feature): the foreign-value check must run
    // BEFORE clearing any previous Switchboard-owned value. Clearing first
    // would restore the pre-Switchboard value into the file, which the
    // check would then misreport as foreign.
    if !force {
        let settings = std::fs::read_to_string(&path).unwrap_or_default();
        let current: serde_json::Value =
            serde_json::from_str(&settings).unwrap_or(serde_json::json!({}));
        let current_statusline = current.get("statusLine").cloned();
        let ours = existing.as_ref().map(|(s, _)| {
            serde_json::json!({ "type": "command", "command": s.installed_command })
        });
        if let Some(foreign) = current_statusline {
            if Some(&foreign) != ours.as_ref() {
                return Ok(InstallStatuslineOutcome::NeedsConfirmation { foreign_value: foreign });
            }
        }
    }

    if let Some((prev_state, prev_prior)) = existing {
        let written = serde_json::json!({ "type": "command", "command": prev_state.installed_command });
        crate::statusline_installer::clear(&path, &prev_prior, &written).map_err(|e| e.to_string())?;
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let command = format!("\"{}\" statusline", exe.display());
    let prior = crate::statusline_installer::apply(&path, &command).map_err(|e| e.to_string())?;
    state
        .db
        .set_statusline_install(&prior, &command, Utc::now().timestamp())
        .map_err(|e| e.to_string())?;
    Ok(InstallStatuslineOutcome::Applied)
}

#[command]
#[specta::specta]
pub async fn uninstall_statusline(state: State<'_, Arc<AppState>>) -> Result<bool, String> {
    let Some((install_state, prior)) = state.db.get_statusline_install().map_err(|e| e.to_string())? else {
        return Ok(true);
    };
    let path = claude_settings_path()?;
    let written = serde_json::json!({ "type": "command", "command": install_state.installed_command });
    let ok = crate::statusline_installer::clear(&path, &prior, &written).map_err(|e| e.to_string())?;
    if ok {
        state.db.clear_statusline_install().map_err(|e| e.to_string())?;
    }
    Ok(ok)
}
```

- [ ] **Step 2: Register all three commands in both `collect_commands!` blocks**

In `src-tauri/src/lib.rs`, add `commands::get_statusline_install_state,`, `commands::install_statusline,`, `commands::uninstall_statusline,` to **both** the `#[cfg(not(debug_assertions))]` and `#[cfg(debug_assertions)]` `collect_commands!` lists (right after `commands::get_warmup_suggestion,`, the current last entry in both — added by F6).

- [ ] **Step 3: Regenerate the TypeScript bindings**

Same procedure as F5/F6: check for a running instance of the app first (`ps aux | grep -i claude-switchboard | grep -v grep`) and quit it if found (SQLite lock contention risk on DB open). From the repo root, run `cargo build --manifest-path src-tauri/Cargo.toml` to confirm a clean compile, then run the dev app (`pnpm tauri dev`) just long enough for `src/lib/generated/bindings.ts` to be rewritten (the specta export runs early in `run()`, well before the window appears or the DB opens) — poll for `getInstallStatusline` or `installStatusline` to appear in the file rather than guessing a fixed sleep — then stop the dev process. Do not leave a dev server running afterward.

Confirm `src/lib/generated/bindings.ts` now contains `getStatuslineInstallState`, `installStatusline`, `uninstallStatusline`, `StatuslineInstallState`, and `InstallStatuslineOutcome`.

- [ ] **Step 4: Run the full backend suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs src/lib/generated/bindings.ts
git commit -m "feat(ipc): add statusline install/uninstall/status commands"
```

---

## Task 7: `ipc.ts` wrapper

**Files:**
- Modify: `src/lib/ipc.ts`

**Interfaces:**
- Consumes: `commands.{getStatuslineInstallState, installStatusline, uninstallStatusline}` from the regenerated bindings (Task 6).
- Produces: `ipc.getStatuslineInstallState()`, `ipc.installStatusline(force)`, `ipc.uninstallStatusline()`. Task 8's frontend component calls all three.

- [ ] **Step 1: Add the wrappers**

In `src/lib/ipc.ts`, near `getWarmupSuggestion` (F6's addition — same "settings-adjacent" area), add:

```ts
  getStatuslineInstallState: () => commands.getStatuslineInstallState().then(unwrap),
  installStatusline: (force: boolean) => commands.installStatusline(force).then(unwrap),
  uninstallStatusline: () => commands.uninstallStatusline().then(unwrap),
```

- [ ] **Step 2: Typecheck**

Run: `npx tsc --noEmit`
Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add src/lib/ipc.ts
git commit -m "feat(ipc): add statusline install/uninstall/status wrappers"
```

---

## Task 8: `StatuslineSettings` — new Settings section

**Files:**
- Create: `src/settings/StatuslineSettings.tsx`
- Test: `src/settings/__tests__/StatuslineSettings.test.tsx`
- Modify: `src/settings/SettingsPanel.tsx`

**Interfaces:**
- Consumes: `ipc.{getStatuslineInstallState, installStatusline, uninstallStatusline}` (Task 7), `StatuslineInstallState`/`InstallStatuslineOutcome` types from `../lib/generated/bindings`, existing `Card`, `Button` UI primitives.
- Produces: `StatuslineSettings` component, rendered inside a new section in `SettingsPanel.tsx`.

- [ ] **Step 1: Write the failing test**

Create `src/settings/__tests__/StatuslineSettings.test.tsx`:

```tsx
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { StatuslineInstallState, InstallStatuslineOutcome } from '../../lib/generated/bindings';

const ipcMock = vi.hoisted(() => ({
  getStatuslineInstallState: vi.fn(),
  installStatusline: vi.fn(),
  uninstallStatusline: vi.fn(),
}));
vi.mock('../../lib/ipc', () => ({ ipc: ipcMock }));

import { StatuslineSettings } from '../StatuslineSettings';

describe('StatuslineSettings', () => {
  beforeEach(() => {
    ipcMock.getStatuslineInstallState.mockReset();
    ipcMock.installStatusline.mockReset();
    ipcMock.uninstallStatusline.mockReset();
  });

  it('shows an Install button when not installed', async () => {
    ipcMock.getStatuslineInstallState.mockResolvedValue(null);
    render(<StatuslineSettings />);
    expect(await screen.findByRole('button', { name: /install/i })).toBeInTheDocument();
  });

  it('shows an Uninstall button when already installed', async () => {
    const state: StatuslineInstallState = {
      installed_command: '/usr/local/bin/switchboard statusline',
      installed_at: 1_700_000_000,
    };
    ipcMock.getStatuslineInstallState.mockResolvedValue(state);
    render(<StatuslineSettings />);
    expect(await screen.findByRole('button', { name: /uninstall/i })).toBeInTheDocument();
  });

  it('clicking Install applies directly when there is nothing to confirm', async () => {
    // Exactly 2 getStatuslineInstallState calls happen: the initial mount
    // fetch, then the reload() after handleInstall finishes. Queue exactly
    // those 2 values — a 3rd queued value would never be consumed and could
    // mask a queue-order mistake instead of catching one.
    ipcMock.getStatuslineInstallState
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce({
        installed_command: '/usr/local/bin/switchboard statusline',
        installed_at: 1_700_000_000,
      });
    const applied: InstallStatuslineOutcome = { status: 'applied' };
    ipcMock.installStatusline.mockResolvedValue(applied);

    render(<StatuslineSettings />);
    fireEvent.click(await screen.findByRole('button', { name: /install/i }));

    await waitFor(() => {
      expect(ipcMock.installStatusline).toHaveBeenCalledWith(false);
    });
    expect(await screen.findByRole('button', { name: /uninstall/i })).toBeInTheDocument();
  });

  it('clicking Install confirms before overwriting a foreign statusLine, and re-invokes with force on accept', async () => {
    // Same 2-call accounting as above: mount fetch, then post-install reload.
    ipcMock.getStatuslineInstallState
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce({
        installed_command: '/usr/local/bin/switchboard statusline',
        installed_at: 1_700_000_000,
      });
    const needsConfirmation: InstallStatuslineOutcome = {
      status: 'needs_confirmation',
      foreign_value: { type: 'command', command: 'bash x.sh' },
    };
    const applied: InstallStatuslineOutcome = { status: 'applied' };
    ipcMock.installStatusline
      .mockResolvedValueOnce(needsConfirmation)
      .mockResolvedValueOnce(applied);
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);

    render(<StatuslineSettings />);
    fireEvent.click(await screen.findByRole('button', { name: /install/i }));

    await waitFor(() => {
      expect(ipcMock.installStatusline).toHaveBeenNthCalledWith(2, true);
    });
    confirmSpy.mockRestore();
  });

  it('does not re-invoke when the user declines the confirmation', async () => {
    ipcMock.getStatuslineInstallState.mockResolvedValue(null);
    const needsConfirmation: InstallStatuslineOutcome = {
      status: 'needs_confirmation',
      foreign_value: { type: 'command', command: 'bash x.sh' },
    };
    ipcMock.installStatusline.mockResolvedValue(needsConfirmation);
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);

    render(<StatuslineSettings />);
    fireEvent.click(await screen.findByRole('button', { name: /install/i }));

    await waitFor(() => {
      expect(ipcMock.installStatusline).toHaveBeenCalledTimes(1);
    });
    confirmSpy.mockRestore();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `npx vitest run src/settings/__tests__/StatuslineSettings.test.tsx`
Expected: FAIL — `Failed to resolve import "../StatuslineSettings"` (component doesn't exist yet).

- [ ] **Step 3: Write the minimal implementation**

Create `src/settings/StatuslineSettings.tsx`:

```tsx
import { useCallback, useEffect, useState } from 'react';
import { Button } from '../components/ui/Button';
import { ipc } from '../lib/ipc';
import type { StatuslineInstallState } from '../lib/generated/bindings';

export function StatuslineSettings() {
  const [state, setState] = useState<StatuslineInstallState | null>(null);
  const [busy, setBusy] = useState(false);

  const reload = useCallback(async () => {
    setState(await ipc.getStatuslineInstallState());
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const handleInstall = useCallback(async () => {
    setBusy(true);
    try {
      const outcome = await ipc.installStatusline(false);
      if (outcome.status === 'needs_confirmation') {
        const cmd =
          typeof outcome.foreign_value === 'object' &&
          outcome.foreign_value !== null &&
          'command' in outcome.foreign_value
            ? String((outcome.foreign_value as { command: unknown }).command)
            : JSON.stringify(outcome.foreign_value);
        const ok = window.confirm(
          `~/.claude/settings.json already has a statusLine command (${cmd}). Switchboard did not write this — another tool or a manual edit did.\n\nOverwrite it?`,
        );
        if (!ok) return;
        await ipc.installStatusline(true);
      }
    } finally {
      setBusy(false);
      await reload();
    }
  }, [reload]);

  const handleUninstall = useCallback(async () => {
    setBusy(true);
    try {
      await ipc.uninstallStatusline();
    } finally {
      setBusy(false);
      await reload();
    }
  }, [reload]);

  return (
    <div className="flex items-center justify-between">
      <div className="flex flex-col gap-[var(--space-2xs)]">
        <span className="text-[length:var(--text-body)] text-[color:var(--color-text)]">
          Terminal statusline
        </span>
        <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          Shows your 5-hour usage % in the Claude Code terminal prompt. Only works while
          Switchboard is running.
        </span>
      </div>
      {state ? (
        <Button variant="ghost" size="sm" onClick={handleUninstall} disabled={busy}>
          Uninstall
        </Button>
      ) : (
        <Button variant="ghost" size="sm" onClick={handleInstall} disabled={busy}>
          Install
        </Button>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `npx vitest run src/settings/__tests__/StatuslineSettings.test.tsx`
Expected: PASS (6 tests)

- [ ] **Step 5: Wire the section into `SettingsPanel.tsx`**

In `src/settings/SettingsPanel.tsx`, add the import next to `import { WarmupSettings } from './WarmupSettings';`:
```ts
import { StatuslineSettings } from './StatuslineSettings';
```

Add a new section right after the "Warm-up" section's closing `</section>` (before the "Save" comment block):
```tsx
      {/* Statusline */}
      <section className="flex flex-col gap-[var(--space-sm)]">
        <h2 className="text-[length:var(--text-label)] font-[var(--weight-semibold)] text-[color:var(--color-text-muted)] uppercase tracking-[0.04em] px-[var(--space-2xs)]">
          Statusline
        </h2>
        <Card className="p-[var(--space-md)]">
          <StatuslineSettings />
        </Card>
      </section>
```

- [ ] **Step 6: Run the full frontend test suite and typecheck**

Run: `npx vitest run && npx tsc --noEmit`
Expected: all tests PASS, no typecheck errors.

- [ ] **Step 7: Commit**

```bash
git add src/settings/StatuslineSettings.tsx src/settings/__tests__/StatuslineSettings.test.tsx src/settings/SettingsPanel.tsx
git commit -m "feat(settings): add Terminal statusline install/uninstall section"
```
