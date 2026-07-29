# Session Browser & One-Click Resume Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Browse past Claude Code sessions with a recap rich enough to identify them, and resume any one in a new terminal running the provider it originally used.

**Architecture:** A read-only scan of `~/.claude/projects/*/*.jsonl` (top level only — subagent transcripts live one level deeper and are not sessions) produces `SessionSummary` rows, memoized in `AppState` against `max(mtime)`. Provider resolution happens in TypeScript beside `modelLabel`. Resume reuses the Spec A launcher with `--resume <id> --fork-session`.

**Tech Stack:** Rust (serde_json, anyhow, tempfile), Tauri 2.x + tauri-specta, React 19 + TypeScript, Tailwind v4 tokens, Vitest + Testing Library.

**Spec reference:** `docs/superpowers/specs/2026-07-29-session-browser-design.md`

**Depends on:** `docs/superpowers/plans/2026-07-29-custom-model-providers.md` (Spec A). Task 8 calls `launcher::launch` and reads the `providers` table, both of which Spec A creates. Tasks 1–7 do not depend on Spec A and can be built first.

## Global Constraints

- **Subagent transcripts are never listed.** Any path containing a `/subagents/` segment is excluded from the browser (spec §3). The opposite is true for ingestion — see Task 1, where they *must* be included. The two paths deliberately diverge; do not "fix" one to match the other.
- **`norm()` is applied to BOTH operands** when matching a session model against provider config (spec §6). Normalizing only the session side is a no-op and silently breaks GLM and k3.
- **"User message" means a real one.** A `type: "user"` record may carry only `tool_result` blocks. 58% of listed sessions end on such a record. Both `asked` and `left_off` draw from the filtered sequence.
- **Migration numbering assumes Spec A has landed** (it owns `0007`, bumping the schema to 7). This plan adds `0008` and bumps to 8. **If Spec B ships before Spec A**, renumber to `0007` and bump to 7 instead — and note that both `create_fresh_db` and the trailing stamp in `migrate()` must agree.
- **Both `collect_commands!` lists** in `src-tauri/src/lib.rs` (~line 160 and ~line 197) must receive every new command.
- **No hard-coded design values.** Every colour, radius, spacing and duration comes from `var(--…)` tokens. The spacing scale bottoms out at `--space-2xs`; there is no `--space-3xs`.
- **Icons come from `src/lib/icons.ts` (Lucide). No emojis.**
- Rust tests: `cd src-tauri && cargo test`. Frontend: `npm test`. Type-check: `npm run lint`.

---

## File Structure

**New — Rust**

| File | Responsibility |
|---|---|
| `src-tauri/src/sessions/mod.rs` | `SessionSummary`, re-exports |
| `src-tauri/src/sessions/scan.rs` | Discovery + inclusion filter (spec §3) |
| `src-tauri/src/sessions/recap.rs` | Per-transcript extraction (spec §4) |
| `src-tauri/src/store/migrations/0008_reingest_subagents.sql` | Re-ingest after the walker fix |

**New — TypeScript**

| File | Responsibility |
|---|---|
| `src/sessions/SessionsBrowserTab.tsx` | Tab container, search, grouping |
| `src/sessions/SessionRow.tsx` | Collapsed row + expansion toggle |
| `src/sessions/SessionRecapCard.tsx` | Asked / Left off / Touched / stats |
| `src/sessions/ResumeProviderPicker.tsx` | Unresolved-model prompt |
| `src/sessions/resolveProvider.ts` | `norm()` + resolution (spec §6) |
| `src/sessions/useResumableSessions.ts` | Data hook |
| `src/sessions/__tests__/*` | Component + resolution tests |

**Modified:** `src-tauri/src/jsonl_parser/walker.rs`, `src-tauri/src/store/mod.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/commands.rs`, `src-tauri/src/app_state.rs`, `src/report/ExpandedReport.tsx`, `src/lib/ipc.ts`, `docs/release-checklist.md`, `CHANGELOG.md`

---

## Task 1: Backfill subagent transcripts (prerequisite — fixes a shipped bug)

**Files:**
- Modify: `src-tauri/src/jsonl_parser/walker.rs`
- Create: `src-tauri/src/store/migrations/0008_reingest_subagents.sql`
- Modify: `src-tauri/src/store/mod.rs`

**Interfaces:**
- Produces: `discover_jsonl_files` returns subagent transcripts in addition to top-level ones.

**Why:** `watcher.rs:31` watches `RecursiveMode::Recursive`, so subagent transcripts written while Switchboard runs are ingested. `discover_jsonl_files` is a two-level `read_dir` that skips the `<sessionId>/` directory (`if !fmeta.is_file() { continue; }`), so anything written while the app was closed is never backfilled. Measured on the live DB: **13 of 138** subagent transcripts present, leaving 12.3M input and 104.5M cache-read tokens uncounted. `SessionsTab.tsx:119` already carries `SUBAGENT_SEGMENT = '/subagents/agent-'` and rollup logic keyed on it, so the frontend displays data the backfill cannot supply.

This is sequenced first because Task 9 renames that tab to **Cost** and adds a sibling Sessions tab, inviting direct comparison between the two.

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `src-tauri/src/jsonl_parser/walker.rs` (create one if absent, with `use super::*; use tempfile::tempdir;`):

```rust
    #[test]
    fn discovers_subagent_transcripts_one_level_deeper() {
        let root = tempdir().unwrap();
        let project = root.path().join("-Users-me-proj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("sess-1.jsonl"), "{}\n").unwrap();

        let subagents = project.join("sess-1").join("subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        std::fs::write(subagents.join("agent-aaa.jsonl"), "{}\n").unwrap();
        std::fs::write(subagents.join("agent-bbb.jsonl"), "{}\n").unwrap();

        let found = discover_jsonl_files(root.path()).unwrap();
        assert_eq!(found.len(), 3, "top-level transcript plus both subagents");
        assert!(
            found.iter().any(|p| p.ends_with("agent-aaa.jsonl")),
            "subagent transcripts must be backfilled — their API calls cost money"
        );
    }

    #[test]
    fn ignores_non_subagent_subdirectories() {
        let root = tempdir().unwrap();
        let project = root.path().join("-Users-me-proj");
        std::fs::create_dir_all(project.join("sess-1").join("something-else")).unwrap();
        std::fs::write(
            project.join("sess-1").join("something-else").join("x.jsonl"),
            "{}\n",
        )
        .unwrap();
        let found = discover_jsonl_files(root.path()).unwrap();
        assert!(found.is_empty(), "only subagents/ is descended into");
    }
```

- [ ] **Step 2: Run — must fail**

```bash
cd src-tauri && cargo test --lib jsonl_parser::walker::tests::discovers_subagent_transcripts_one_level_deeper
```

Expected: FAIL — `assert_eq!(found.len(), 3)` sees 1.

- [ ] **Step 3: Descend into `subagents/`**

In `src-tauri/src/jsonl_parser/walker.rs`, inside the inner `for f in fs::read_dir(&project_dir)?` loop, replace the early `if !fmeta.is_file() { continue; }` with a branch that descends one level into a session directory's `subagents/`:

```rust
            // A session directory (`<sessionId>/`) may hold subagent
            // transcripts at `<sessionId>/subagents/agent-*.jsonl`. The
            // watcher already picks these up live (RecursiveMode::Recursive),
            // so skipping them here made the backfill disagree with the
            // watcher — 125 of 138 files were never ingested, and their API
            // calls are real spend that the Cost tab is built to display.
            if fmeta.is_dir() {
                let subagents = fpath.join("subagents");
                let Ok(entries) = fs::read_dir(&subagents) else {
                    continue;
                };
                for s in entries {
                    let s = s?;
                    let spath = s.path();
                    let smeta = fs::symlink_metadata(&spath)?;
                    if smeta.file_type().is_symlink() || !smeta.is_file() {
                        continue;
                    }
                    if spath.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                        continue;
                    }
                    if smeta.len() > MAX_FILE_BYTES {
                        tracing::warn!("skipping oversized file (>100MB): {}", spath.display());
                        continue;
                    }
                    files.push(spath);
                }
                continue;
            }
            if !fmeta.is_file() {
                continue;
            }
```

- [ ] **Step 4: Run — must pass**

```bash
cd src-tauri && cargo test --lib jsonl_parser::walker
```

Expected: both new tests PASS, existing walker tests unaffected.

- [ ] **Step 5: Add the re-ingest migration**

Create `src-tauri/src/store/migrations/0008_reingest_subagents.sql`:

```sql
-- v7 → v8: re-ingest so subagent transcripts are counted.
--
-- `discover_jsonl_files` was a two-level read_dir that skipped
-- `<sessionId>/` directories outright, while `watcher.rs` watched with
-- RecursiveMode::Recursive. Subagent transcripts written while Switchboard
-- was running were therefore ingested; anything written while it was closed
-- never was. Measured before the fix: 13 of 138 files present, leaving
-- 12,261,121 input and 104,520,949 cache-read tokens uncounted.
--
-- Clearing jsonl_cursors forces a re-read from byte 0 on the next backfill.
-- session_events is NOT deleted: event_id is stable and UNIQUE, so re-reading
-- is idempotent for rows already stored and simply adds the missing ones.
-- (0006 had to delete because it changed the event_id derivation; this
-- migration does not.)
DELETE FROM jsonl_cursors;
```

- [ ] **Step 6: Wire it and bump the schema version**

In `src-tauri/src/store/mod.rs`, inside `migrate()`, after the `if current < 7 { … }` block added by Spec A:

```rust
        if current < 8 {
            tracing::info!("migrating v7 -> v8 (re-ingest to backfill subagent transcripts)");
            conn.execute_batch(include_str!("migrations/0008_reingest_subagents.sql"))
                .context("apply migration 0008")?;
        }
```

Change **both** version stamps from `7` to `8`: `create_fresh_db` (~line 108) and the trailing stamp in `migrate()` (~line 160).

- [ ] **Step 7: Verify the whole suite**

```bash
cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings
```

Expected: all green.

- [ ] **Step 8: Confirm the real backfill (manual, one-time)**

```bash
npm run tauri dev    # let it run ~60s so the backfill completes, then quit
sqlite3 "$HOME/Library/Application Support/com.claude-switchboard.ClaudeSwitchboard/data.db" \
  "SELECT COUNT(DISTINCT source_file) FROM session_events WHERE source_file LIKE '%/subagents/agent-%';"
```

Expected: **138** (was 13). If it is still 13, the migration did not run — check the schema-version stamps.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/jsonl_parser/walker.rs src-tauri/src/store
git commit -m "fix(ingest): backfill subagent transcripts the watcher already collected"
```

---

## Task 2: Session discovery and the inclusion filter

**Files:**
- Create: `src-tauri/src/sessions/mod.rs`, `src-tauri/src/sessions/scan.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `sessions::scan::discover_session_files(&Path) -> Vec<PathBuf>` (top-level only, newest first), `sessions::scan::is_subagent_path(&Path) -> bool`.

**Why:** This is the C1 regression surface. Subagent transcripts satisfy every inclusion condition — real `cwd`, real project, real user turns — so nothing but an explicit path rule keeps them out. A `walkdir`-based implementation would return 208 rows instead of 70.

- [ ] **Step 1: Register the module**

In `src-tauri/src/lib.rs`, alongside the other `pub mod` declarations:

```rust
pub mod sessions;
```

Create `src-tauri/src/sessions/mod.rs`:

```rust
//! Read-only browser over Claude Code transcripts: what each session was
//! about, and enough identity to resume it on the right provider.
//!
//! Deliberately divergent from `jsonl_parser`: ingestion *wants* subagent
//! transcripts (their API calls are real spend), while the browser must never
//! list them (they are not resumable sessions).

pub mod recap;
pub mod scan;

pub use recap::SessionSummary;
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/src/sessions/scan.rs`:

```rust
use std::path::{Path, PathBuf};

/// Subagent transcripts live at `<project>/<sessionId>/subagents/agent-*.jsonl`.
/// They pass every content-based inclusion test, so only the path distinguishes
/// them — and resuming one would run `claude --resume <agentId>` against an id
/// that is not a session.
pub fn is_subagent_path(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str() == "subagents")
}

/// Top-level transcripts only, newest first. Deliberately a two-level
/// `read_dir` rather than a recursive walk: recursion would pull in the 138
/// subagent transcripts, which outnumber real sessions in some projects.
pub fn discover_session_files(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let Ok(projects) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    for project in projects.flatten() {
        let ppath = project.path();
        let Ok(pmeta) = std::fs::symlink_metadata(&ppath) else { continue };
        if pmeta.file_type().is_symlink() || !pmeta.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&ppath) else { continue };
        for f in files.flatten() {
            let fpath = f.path();
            let Ok(fmeta) = std::fs::symlink_metadata(&fpath) else { continue };
            if fmeta.file_type().is_symlink() || !fmeta.is_file() {
                continue;
            }
            if fpath.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let mtime = fmeta.modified().unwrap_or(std::time::UNIX_EPOCH);
            out.push((mtime, fpath));
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn corpus() -> tempfile::TempDir {
        let root = tempdir().unwrap();
        let project = root.path().join("-Users-me-proj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("sess-1.jsonl"), "{}\n").unwrap();
        std::fs::write(project.join("sess-2.jsonl"), "{}\n").unwrap();
        let sub = project.join("sess-1").join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        for i in 0..5 {
            std::fs::write(sub.join(format!("agent-{i}.jsonl")), "{}\n").unwrap();
        }
        root
    }

    #[test]
    fn subagent_transcripts_are_never_discovered() {
        let root = corpus();
        let found = discover_session_files(root.path());
        assert_eq!(found.len(), 2, "only the two top-level transcripts");
        assert!(
            !found.iter().any(|p| is_subagent_path(p)),
            "a subagent transcript reached the browser"
        );
    }

    #[test]
    fn is_subagent_path_matches_the_segment_anywhere() {
        assert!(is_subagent_path(Path::new("/a/proj/sess/subagents/agent-x.jsonl")));
        assert!(!is_subagent_path(Path::new("/a/proj/sess-1.jsonl")));
        // Not fooled by a similarly-named file.
        assert!(!is_subagent_path(Path::new("/a/proj/subagents-notes.jsonl")));
    }

    #[test]
    fn results_are_newest_first() {
        let root = tempdir().unwrap();
        let project = root.path().join("p");
        std::fs::create_dir_all(&project).unwrap();
        let old = project.join("old.jsonl");
        let new = project.join("new.jsonl");
        std::fs::write(&old, "{}\n").unwrap();
        std::fs::write(&new, "{}\n").unwrap();
        filetime::set_file_mtime(
            &old,
            filetime::FileTime::from_system_time(
                std::time::SystemTime::now() - std::time::Duration::from_secs(7200),
            ),
        )
        .unwrap();
        let found = discover_session_files(root.path());
        assert!(found[0].ends_with("new.jsonl"), "newest first");
    }

    #[test]
    fn missing_root_is_not_an_error() {
        assert!(discover_session_files(Path::new("/definitely/not/here")).is_empty());
    }
}
```

- [ ] **Step 3: Run**

```bash
cd src-tauri && cargo test --lib sessions::scan
```

Expected: 4 tests PASS. (`filetime` is already a dev-dependency from the Spec A plan; if Spec B is built first, run `cargo add --dev filetime`.)

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/sessions src-tauri/src/lib.rs
git commit -m "feat(sessions): discovery that excludes subagent transcripts"
```

---

## Task 3: Recap extraction

**Files:**
- Create: `src-tauri/src/sessions/recap.rs`

**Interfaces:**
- Consumes: `scan::is_subagent_path` (Task 2).
- Produces: `recap::SessionSummary` (specta type), `recap::parse_session(&Path) -> Option<SessionSummary>`, `recap::is_real_user_text(&Value) -> Option<String>`.

**Why:** This is the H1 surface. 58% of listed sessions end on a `type: "user"` record carrying only `tool_result` blocks; taking "the last user record" renders tool output as *Left off* on the majority of rows.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/sessions/recap.rs`:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

const MAX_TOUCHED: usize = 4;
const TITLE_MAX_CHARS: usize = 80;

/// Markers Claude Code injects into `type: "user"` records that are not the
/// human speaking. Matched exactly rather than by a bare `<` prefix, which
/// would misfire on pasted markup or generics such as `Vec<T>`.
const INJECTION_MARKERS: [&str; 4] = [
    "<system-reminder>",
    "<command-name>",
    "<local-command-stdout>",
    "<command-message>",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct SessionSummary {
    pub session_id: String,
    pub cwd: String,
    pub project_name: String,
    pub git_branch: Option<String>,
    pub title: String,
    pub asked: String,
    pub left_off: Option<String>,
    pub touched_files: Vec<String>,
    pub touched_overflow: usize,
    pub model: Option<String>,
    pub turns: u32,
    pub started_at: String,
    pub ended_at: String,
}

/// The text of a *real* user message, or `None`.
///
/// A `type: "user"` record may carry only `tool_result` blocks — 58% of
/// listed sessions end on one. Those are not the user speaking and must never
/// surface as `asked` or `left_off`.
pub fn is_real_user_text(message: &Value) -> Option<String> {
    if message.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let text = match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => return None,
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if INJECTION_MARKERS.iter().any(|m| trimmed.starts_with(m)) {
        return None;
    }
    Some(trimmed.to_string())
}

fn truncate(s: &str, max: usize) -> String {
    let one_line = s.split('\n').next().unwrap_or(s).trim();
    if one_line.chars().count() <= max {
        return one_line.to_string();
    }
    let cut: String = one_line.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

/// `None` when the transcript does not qualify as a session (spec §3).
pub fn parse_session(path: &Path) -> Option<SessionSummary> {
    if crate::sessions::scan::is_subagent_path(path) {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;

    let mut cwd: Option<String> = None;
    let mut git_branch: Option<String> = None;
    let mut ai_title: Option<String> = None;
    let mut model: Option<String> = None;
    let mut first_user: Option<String> = None;
    let mut last_user: Option<String> = None;
    let mut turns: u32 = 0;
    let mut timestamps: Vec<String> = Vec::new();
    let mut touched: HashMap<String, usize> = HashMap::new();

    for line in text.lines() {
        // A malformed line is skipped, never fatal.
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };

        if let Some(c) = v.get("cwd").and_then(Value::as_str) {
            if !c.is_empty() {
                cwd = Some(c.to_string());
            }
        }
        if let Some(b) = v.get("gitBranch").and_then(Value::as_str) {
            if !b.is_empty() {
                git_branch = Some(b.to_string());
            }
        }
        if let Some(t) = v.get("aiTitle").and_then(Value::as_str) {
            if !t.is_empty() {
                ai_title = Some(t.to_string());
            }
        }
        if let Some(ts) = v.get("timestamp").and_then(Value::as_str) {
            timestamps.push(ts.to_string());
        }

        let Some(message) = v.get("message") else { continue };

        // `<synthetic>` is Claude Code's placeholder for locally generated
        // messages, not a model the session ran on.
        if let Some(m) = message.get("model").and_then(Value::as_str) {
            if !m.is_empty() && m != "<synthetic>" {
                model = Some(m.to_string());
            }
        }

        if let Some(t) = is_real_user_text(message) {
            turns += 1;
            if first_user.is_none() {
                first_user = Some(t.clone());
            }
            last_user = Some(t);
        }

        if message.get("role").and_then(Value::as_str) == Some("assistant") {
            if let Some(Value::Array(blocks)) = message.get("content") {
                for b in blocks {
                    if b.get("type").and_then(Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    let Some(fp) = b.pointer("/input/file_path").and_then(Value::as_str) else {
                        continue;
                    };
                    let name = Path::new(fp)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| fp.to_string());
                    *touched.entry(name).or_insert(0) += 1;
                }
            }
        }
    }

    let cwd = cwd?;
    let project_name = Path::new(&cwd).file_name()?.to_string_lossy().to_string();
    if project_name == "-" || turns == 0 {
        return None;
    }
    let asked = first_user?;

    let mut ranked: Vec<(String, usize)> = touched.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let touched_overflow = ranked.len().saturating_sub(MAX_TOUCHED);
    let touched_files: Vec<String> =
        ranked.into_iter().take(MAX_TOUCHED).map(|(n, _)| n).collect();

    let left_off = last_user.filter(|l| l != &asked).map(|l| truncate(&l, 160));

    Some(SessionSummary {
        session_id: path.file_stem()?.to_string_lossy().to_string(),
        cwd,
        project_name,
        git_branch,
        title: ai_title.unwrap_or_else(|| truncate(&asked, TITLE_MAX_CHARS)),
        asked: truncate(&asked, 160),
        left_off,
        touched_files,
        touched_overflow,
        model,
        turns,
        started_at: timestamps.first().cloned().unwrap_or_default(),
        ended_at: timestamps.last().cloned().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn write_session(dir: &Path, name: &str, lines: &[Value]) -> std::path::PathBuf {
        let p = dir.join(name);
        let body: String = lines
            .iter()
            .map(|l| serde_json::to_string(l).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&p, body).unwrap();
        p
    }

    fn user(text: &str) -> Value {
        json!({"cwd":"/w/proj","gitBranch":"main","timestamp":"2026-07-29T10:00:00Z",
               "message":{"role":"user","content":[{"type":"text","text":text}]}})
    }

    fn tool_result(text: &str) -> Value {
        json!({"cwd":"/w/proj","timestamp":"2026-07-29T10:05:00Z",
               "message":{"role":"user","content":[
                   {"type":"tool_result","tool_use_id":"toolu_1","content":text}]}})
    }

    fn assistant_edit(file: &str) -> Value {
        json!({"cwd":"/w/proj","timestamp":"2026-07-29T10:02:00Z",
               "message":{"role":"assistant","model":"claude-opus-5","content":[
                   {"type":"tool_use","name":"Edit","input":{"file_path":file}}]}})
    }

    /// H1 regression: 58% of real sessions end on a tool_result.
    #[test]
    fn tool_results_never_become_asked_or_left_off() {
        let d = tempdir().unwrap();
        let p = write_session(d.path(), "s.jsonl", &[
            tool_result("This command requires approval"),
            user("build the thing"),
            assistant_edit("/w/proj/src/main.rs"),
            user("now ship it"),
            tool_result("Applications\nLibrary\nSystem"),
        ]);
        let s = parse_session(&p).expect("session");
        assert_eq!(s.asked, "build the thing", "a leading tool_result must not become asked");
        assert_eq!(
            s.left_off.as_deref(),
            Some("now ship it"),
            "a trailing tool_result must not become left_off"
        );
        assert_eq!(s.turns, 2, "tool results are not turns");
    }

    #[test]
    fn injection_markers_are_not_user_messages() {
        let d = tempdir().unwrap();
        let p = write_session(d.path(), "s.jsonl", &[
            user("<system-reminder>be nice</system-reminder>"),
            user("real question"),
        ]);
        let s = parse_session(&p).expect("session");
        assert_eq!(s.asked, "real question");
        assert_eq!(s.turns, 1);
    }

    #[test]
    fn generic_angle_brackets_are_still_real_messages() {
        let d = tempdir().unwrap();
        let p = write_session(d.path(), "s.jsonl", &[user("<T> in Vec<T> confuses me")]);
        let s = parse_session(&p).expect("session");
        assert_eq!(s.asked, "<T> in Vec<T> confuses me", "bare '<' must not be a filter");
    }

    #[test]
    fn title_prefers_ai_title_then_falls_back_to_first_message() {
        let d = tempdir().unwrap();
        let p1 = write_session(d.path(), "a.jsonl", &[
            user("some long question about the thing"),
            json!({"aiTitle":"Fix the parser","cwd":"/w/proj"}),
        ]);
        assert_eq!(parse_session(&p1).unwrap().title, "Fix the parser");

        let p2 = write_session(d.path(), "b.jsonl", &[user("some long question about the thing")]);
        assert_eq!(parse_session(&p2).unwrap().title, "some long question about the thing");
    }

    #[test]
    fn left_off_is_none_when_it_equals_asked() {
        let d = tempdir().unwrap();
        let p = write_session(d.path(), "s.jsonl", &[user("only one turn")]);
        assert!(parse_session(&p).unwrap().left_off.is_none());
    }

    #[test]
    fn touched_files_rank_by_frequency_and_cap_at_four() {
        let d = tempdir().unwrap();
        let mut lines = vec![user("go")];
        for _ in 0..3 { lines.push(assistant_edit("/w/proj/a.rs")); }
        for _ in 0..2 { lines.push(assistant_edit("/w/proj/b.rs")); }
        for f in ["c.rs", "d.rs", "e.rs"] {
            lines.push(assistant_edit(&format!("/w/proj/{f}")));
        }
        let p = write_session(d.path(), "s.jsonl", &lines);
        let s = parse_session(&p).unwrap();
        assert_eq!(s.touched_files, vec!["a.rs", "b.rs", "c.rs", "d.rs"]);
        assert_eq!(s.touched_overflow, 1);
    }

    #[test]
    fn touched_is_empty_rather_than_placeholder_when_absent() {
        let d = tempdir().unwrap();
        let p = write_session(d.path(), "s.jsonl", &[user("just talking")]);
        let s = parse_session(&p).unwrap();
        assert!(s.touched_files.is_empty());
        assert_eq!(s.touched_overflow, 0);
    }

    #[test]
    fn synthetic_model_is_not_treated_as_a_model() {
        let d = tempdir().unwrap();
        let p = write_session(d.path(), "s.jsonl", &[
            user("hi"),
            json!({"cwd":"/w/proj","message":{"role":"assistant","model":"<synthetic>","content":[]}}),
        ]);
        assert!(parse_session(&p).unwrap().model.is_none());
    }

    #[test]
    fn headless_and_turnless_transcripts_are_excluded() {
        let d = tempdir().unwrap();
        // No cwd at all.
        let p1 = write_session(d.path(), "a.jsonl", &[
            json!({"message":{"role":"user","content":[{"type":"text","text":"hi"}]}}),
        ]);
        assert!(parse_session(&p1).is_none());
        // cwd resolves to project "-".
        let p2 = write_session(d.path(), "b.jsonl", &[
            json!({"cwd":"/-","message":{"role":"user","content":[{"type":"text","text":"hi"}]}}),
        ]);
        assert!(parse_session(&p2).is_none());
        // Real cwd but no real user turn.
        let p3 = write_session(d.path(), "c.jsonl", &[tool_result("x")]);
        assert!(parse_session(&p3).is_none());
    }

    #[test]
    fn malformed_lines_are_skipped_not_fatal() {
        let d = tempdir().unwrap();
        let p = d.path().join("s.jsonl");
        std::fs::write(
            &p,
            format!("not json at all\n{}\n{{ broken\n", serde_json::to_string(&user("hi")).unwrap()),
        )
        .unwrap();
        let s = parse_session(&p).expect("one good line is enough");
        assert_eq!(s.asked, "hi");
    }

    #[test]
    fn subagent_paths_are_refused_even_when_content_qualifies() {
        let d = tempdir().unwrap();
        let sub = d.path().join("sess").join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        let p = write_session(&sub, "agent-x.jsonl", &[user("do the subtask")]);
        assert!(parse_session(&p).is_none(), "content qualifies; the path must still exclude it");
    }
}
```

- [ ] **Step 2: Run**

```bash
cd src-tauri && cargo test --lib sessions::recap
```

Expected: 11 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/sessions/recap.rs
git commit -m "feat(sessions): recap extraction with real-user-message filtering"
```

---

## Task 4: Memoized command

**Files:**
- Modify: `src-tauri/src/app_state.rs`, `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`, `src/lib/ipc.ts`

**Interfaces:**
- Consumes: `scan::discover_session_files`, `recap::parse_session`, `recap::SessionSummary` (Tasks 2–3).
- Produces: command `list_resumable_sessions() -> Vec<SessionSummary>`; `ipc.listResumableSessions()`.

**Why:** `ExpandedReport` remounts tab components across its slide transition, so without a memo every tab switch re-reads and re-parses 100.7 MB.

- [ ] **Step 1: Add the memo to `AppState`**

In `src-tauri/src/app_state.rs`, add the field to `AppState`:

```rust
    /// Cached session-browser scan, invalidated when any transcript's mtime
    /// advances. `ExpandedReport` remounts tab components on every tab
    /// change, so an unmemoized scan would re-parse ~100 MB per switch.
    pub sessions_cache: RwLock<Option<(std::time::SystemTime, Vec<crate::sessions::SessionSummary>)>>,
```

Initialise it at every `AppState` construction site (find them with `grep -n "AppState {" src-tauri/src/`):

```rust
            sessions_cache: RwLock::new(None),
```

- [ ] **Step 2: Add the command**

Append to `src-tauri/src/commands.rs`:

```rust
use crate::sessions::{recap, scan, SessionSummary};

const MAX_SESSIONS: usize = 200;

#[command]
#[specta::specta]
pub async fn list_resumable_sessions(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<SessionSummary>, String> {
    let Some(root) = crate::jsonl_parser::walker::claude_projects_root() else {
        return Ok(Vec::new());
    };
    let files = scan::discover_session_files(&root);

    // Newest mtime is the cache key: any new or appended transcript advances it.
    let newest = files
        .first()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .unwrap_or(std::time::UNIX_EPOCH);

    if let Some((cached_at, rows)) = state.sessions_cache.read().await.as_ref() {
        if *cached_at == newest {
            return Ok(rows.clone());
        }
    }

    // The cap applies to sessions AFTER filtering, not files scanned — a
    // pre-filter cap would let one session's subagent transcripts evict real
    // sessions. Scanning is cheap; the result list is what needs bounding.
    let mut rows: Vec<SessionSummary> = Vec::new();
    for f in &files {
        if let Some(s) = recap::parse_session(f) {
            rows.push(s);
            if rows.len() >= MAX_SESSIONS {
                break;
            }
        }
    }

    *state.sessions_cache.write().await = Some((newest, rows.clone()));
    Ok(rows)
}
```

If `AppState`'s other `RwLock` fields are `parking_lot` rather than `tokio`, drop the `.await` calls to match — check the imports at the top of `app_state.rs` and be consistent with the file.

- [ ] **Step 3: Register in BOTH `collect_commands!` lists**

In `src-tauri/src/lib.rs`, add to the `#[cfg(not(debug_assertions))]` list **and** the `#[cfg(debug_assertions)]` list:

```rust
            commands::list_resumable_sessions,
```

- [ ] **Step 4: Build and add the TS wrapper**

```bash
cd src-tauri && cargo build
```

Then in `src/lib/ipc.ts`, inside the `ipc` object:

```ts
  // Session browser
  listResumableSessions: () => commands.listResumableSessions().then(unwrap),
```

- [ ] **Step 5: Verify**

```bash
cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings
cd .. && npm run lint
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/app_state.rs src-tauri/src/commands.rs src-tauri/src/lib.rs src/lib/ipc.ts src/lib/generated/bindings.ts
git commit -m "feat(sessions): memoized list_resumable_sessions command"
```

---

## Task 5: Provider resolution in TypeScript

**Files:**
- Create: `src/sessions/resolveProvider.ts`, `src/sessions/__tests__/resolveProvider.test.ts`

**Interfaces:**
- Consumes: `Provider` from `src/lib/generated/bindings` (Spec A Task 1).
- Produces: `norm(s: string): string`, `resolveProvider(model: string | null, providers: Provider[]): Resolution`, where `type Resolution = { kind: 'resolved'; providerId: string } | { kind: 'unresolved' }`.

**Why:** This is the C2 regression surface. The defect is *which operand* gets normalized, so the test must assert end-to-end resolution — a test of `norm` alone passes while the feature is broken.

- [ ] **Step 1: Write the failing tests**

Create `src/sessions/__tests__/resolveProvider.test.ts`:

```ts
import { describe, it, expect } from 'vitest';
import type { Provider } from '../../lib/generated/bindings';
import { norm, resolveProvider } from '../resolveProvider';

function provider(id: string, env: Record<string, string>, sortIndex = 1): Provider {
  return {
    id, name: id, kind: 'third_party',
    base_url: 'https://example.test', auth_token: 't',
    env, extra_args: [], preset_id: null, sort_index: sortIndex,
  };
}

const official: Provider = {
  id: 'official', name: 'Anthropic (official)', kind: 'official',
  base_url: null, auth_token: null, env: {}, extra_args: [], preset_id: null, sort_index: 0,
};

describe('resolveProvider', () => {
  it('resolves a [1m]-suffixed provider config against a stripped session model', () => {
    // The C2 regression: Claude Code strips [1m] before writing the
    // transcript, so the SESSION side is already normalized and the PROVIDER
    // side still carries the suffix.
    const glm = provider('glm', { ANTHROPIC_MODEL: 'glm-5.2[1m]' });
    expect(resolveProvider('glm-5.2', [official, glm])).toEqual({
      kind: 'resolved', providerId: 'glm',
    });
  });

  it('resolves k3 the same way', () => {
    const kimi = provider('kimi', { ANTHROPIC_MODEL: 'k3[1m]' });
    expect(resolveProvider('k3', [official, kimi])).toEqual({
      kind: 'resolved', providerId: 'kimi',
    });
  });

  it('matches case-insensitively', () => {
    const mm = provider('minimax', { ANTHROPIC_MODEL: 'MiniMax-M2.7-highspeed' });
    expect(resolveProvider('minimax-m2.7-highspeed', [mm])).toEqual({
      kind: 'resolved', providerId: 'minimax',
    });
  });

  it('matches on non-ANTHROPIC_MODEL keys', () => {
    // 519 recorded events are kimi-for-coding-highspeed, configured as the
    // small/fast model rather than the primary one.
    const kimi = provider('kimi', {
      ANTHROPIC_MODEL: 'k3[1m]',
      ANTHROPIC_SMALL_FAST_MODEL: 'kimi-for-coding-highspeed',
    });
    expect(resolveProvider('kimi-for-coding-highspeed', [kimi])).toEqual({
      kind: 'resolved', providerId: 'kimi',
    });
  });

  it('breaks ties by sort_index then id', () => {
    const a = provider('bbb', { ANTHROPIC_MODEL: 'dup' }, 5);
    const b = provider('aaa', { ANTHROPIC_MODEL: 'dup' }, 2);
    expect(resolveProvider('dup', [a, b])).toEqual({ kind: 'resolved', providerId: 'aaa' });
  });

  it('falls back to official for claude-* ids', () => {
    expect(resolveProvider('claude-opus-5', [official])).toEqual({
      kind: 'resolved', providerId: 'official',
    });
  });

  it('does not silently reroute a deleted provider to official', () => {
    // A relay that echoed an Anthropic-style id. With its provider removed,
    // this must prompt rather than resume on Anthropic.
    expect(resolveProvider('claude-sonnet-4-5-thinking', [official])).toEqual({
      kind: 'unresolved',
    });
  });

  it('is unresolved for an unknown model and for no model at all', () => {
    expect(resolveProvider('mystery-9', [official])).toEqual({ kind: 'unresolved' });
    expect(resolveProvider(null, [official])).toEqual({ kind: 'unresolved' });
  });
});

describe('norm', () => {
  it('lowercases and strips a trailing [1m]', () => {
    expect(norm('GLM-5.2[1M]')).toBe('glm-5.2');
    expect(norm('k3')).toBe('k3');
  });
});
```

- [ ] **Step 2: Run — must fail**

```bash
npm test -- src/sessions/__tests__/resolveProvider.test.ts
```

Expected: FAIL — module not found.

- [ ] **Step 3: Implement**

Create `src/sessions/resolveProvider.ts`:

```ts
import type { Provider } from '../lib/generated/bindings';

export type Resolution =
  | { kind: 'resolved'; providerId: string }
  | { kind: 'unresolved' };

const MODEL_KEYS = [
  'ANTHROPIC_MODEL',
  'ANTHROPIC_SMALL_FAST_MODEL',
  'ANTHROPIC_DEFAULT_OPUS_MODEL',
  'ANTHROPIC_DEFAULT_SONNET_MODEL',
  'ANTHROPIC_DEFAULT_HAIKU_MODEL',
  'ANTHROPIC_DEFAULT_FABLE_MODEL',
] as const;

/**
 * Applied to BOTH sides of the comparison. Claude Code strips the `[1m]`
 * context modifier before writing the transcript, so the session model
 * arrives pre-normalized while the provider config still carries it —
 * normalizing only one side silently fails to match GLM and k3.
 */
export function norm(s: string): string {
  return s.trim().toLowerCase().replace(/\[1m\]$/, '');
}

/** Anthropic-style ids a session may record when served by a relay. */
function looksAnthropic(model: string): boolean {
  return norm(model).startsWith('claude-');
}

export function resolveProvider(
  model: string | null,
  providers: Provider[],
): Resolution {
  if (!model) return { kind: 'unresolved' };
  const needle = norm(model);

  // Deterministic ordering: two providers may declare the same model id.
  const ordered = [...providers].sort(
    (a, b) => a.sort_index - b.sort_index || a.id.localeCompare(b.id),
  );

  for (const p of ordered) {
    for (const key of MODEL_KEYS) {
      const configured = p.env[key];
      if (configured && norm(configured) === needle) {
        return { kind: 'resolved', providerId: p.id };
      }
    }
  }

  // Only a genuine Anthropic id falls back to official. A relay id that
  // merely looks Anthropic-style (claude-sonnet-4-5-thinking) reaches here
  // when its provider has been deleted, and must prompt rather than resume
  // silently on the wrong model.
  if (looksAnthropic(model) && !/-thinking$|^claude-\d/.test(needle)) {
    const off = providers.find((p) => p.kind === 'official');
    if (off) return { kind: 'resolved', providerId: off.id };
  }

  return { kind: 'unresolved' };
}
```

- [ ] **Step 4: Run — must pass**

```bash
npm test -- src/sessions/__tests__/resolveProvider.test.ts
```

Expected: 9 tests PASS. If `does not silently reroute a deleted provider` fails, the `looksAnthropic` guard is too permissive — it must reject relay-style suffixes.

- [ ] **Step 5: Commit**

```bash
git add src/sessions
git commit -m "feat(sessions): model-to-provider resolution normalizing both operands"
```

---

## Task 6: Session rows and the recap card

**Files:**
- Create: `src/sessions/useResumableSessions.ts`, `src/sessions/SessionRecapCard.tsx`, `src/sessions/SessionRow.tsx`
- Create: `src/sessions/__tests__/SessionRow.test.tsx`

**Interfaces:**
- Consumes: `ipc.listResumableSessions` (Task 4), `modelLabel` from `src/report/SessionsTab.tsx`.
- Produces: `useResumableSessions()`, `<SessionRecapCard session />`, `<SessionRow session expanded onToggle onResume />`.

- [ ] **Step 1: Write the failing tests**

Create `src/sessions/__tests__/SessionRow.test.tsx`:

```tsx
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import type { SessionSummary } from '../../lib/generated/bindings';
import { SessionRow } from '../SessionRow';

function session(over: Partial<SessionSummary> = {}): SessionSummary {
  return {
    session_id: '029a3e04-fa36',
    cwd: '/Users/me/Developer/claude-switchboard',
    project_name: 'claude-switchboard',
    git_branch: 'main',
    title: 'Plan custom model swapping feature',
    asked: 'we need a to plan for another major feature',
    left_off: 'what about spec B?',
    touched_files: ['design.md', 'plan.md'],
    touched_overflow: 3,
    model: 'glm-5.2',
    turns: 12,
    started_at: '2026-07-28T23:07:00Z',
    ended_at: '2026-07-29T01:14:00Z',
    ...over,
  };
}

describe('SessionRow', () => {
  it('shows title, project, branch and model when collapsed', () => {
    render(<SessionRow session={session()} expanded={false} onToggle={vi.fn()} onResume={vi.fn()} />);
    expect(screen.getByText('Plan custom model swapping feature')).toBeTruthy();
    expect(screen.getByText(/claude-switchboard/)).toBeTruthy();
    expect(screen.getByText(/main/)).toBeTruthy();
    expect(screen.getByText('glm-5.2')).toBeTruthy();
  });

  it('hides the recap until expanded', () => {
    const { rerender } = render(
      <SessionRow session={session()} expanded={false} onToggle={vi.fn()} onResume={vi.fn()} />,
    );
    expect(screen.queryByText(/we need a to plan/)).toBeNull();
    rerender(<SessionRow session={session()} expanded onToggle={vi.fn()} onResume={vi.fn()} />);
    expect(screen.getByText(/we need a to plan/)).toBeTruthy();
    expect(screen.getByText(/what about spec B/)).toBeTruthy();
  });

  it('shows touched files with an overflow count', () => {
    render(<SessionRow session={session()} expanded onToggle={vi.fn()} onResume={vi.fn()} />);
    expect(screen.getByText('design.md')).toBeTruthy();
    expect(screen.getByText(/\+3 more/)).toBeTruthy();
  });

  it('omits the Touched row entirely when nothing was touched', () => {
    render(
      <SessionRow
        session={session({ touched_files: [], touched_overflow: 0 })}
        expanded onToggle={vi.fn()} onResume={vi.fn()}
      />,
    );
    expect(screen.queryByText(/touched/i)).toBeNull();
  });

  it('omits Left off when absent', () => {
    render(
      <SessionRow session={session({ left_off: null })} expanded onToggle={vi.fn()} onResume={vi.fn()} />,
    );
    expect(screen.queryByText(/left off/i)).toBeNull();
  });

  it('renders an unknown-model session without a badge', () => {
    render(
      <SessionRow session={session({ model: null })} expanded={false} onToggle={vi.fn()} onResume={vi.fn()} />,
    );
    expect(screen.getByText(/unknown/i)).toBeTruthy();
  });

  it('calls onResume with the session id', () => {
    const onResume = vi.fn();
    render(<SessionRow session={session()} expanded onToggle={vi.fn()} onResume={onResume} />);
    fireEvent.click(screen.getByRole('button', { name: /resume/i }));
    expect(onResume).toHaveBeenCalledWith(session().session_id);
  });
});
```

- [ ] **Step 2: Run — must fail**

```bash
npm test -- src/sessions/__tests__/SessionRow.test.tsx
```

- [ ] **Step 3: Implement the recap card**

Create `src/sessions/SessionRecapCard.tsx`:

```tsx
import type { SessionSummary } from '../lib/generated/bindings';

function span(startIso: string, endIso: string): string {
  const a = new Date(startIso).getTime();
  const b = new Date(endIso).getTime();
  if (!Number.isFinite(a) || !Number.isFinite(b) || b <= a) return '';
  const mins = Math.round((b - a) / 60000);
  if (mins < 60) return `${mins}m`;
  return `${Math.floor(mins / 60)}h${String(mins % 60).padStart(2, '0')}m`;
}

const labelClass =
  'shrink-0 w-[58px] text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]';

export function SessionRecapCard({ session }: { session: SessionSummary }) {
  const duration = span(session.started_at, session.ended_at);
  return (
    <div className="flex flex-col gap-[var(--space-2xs)] pt-[var(--space-2xs)]">
      <div className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
        {session.turns} turn{session.turns === 1 ? '' : 's'}
        {duration && ` over ${duration}`}
      </div>

      <div className="flex gap-[var(--space-xs)]">
        <span className={labelClass}>Asked</span>
        <span className="flex-1 text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]">
          {session.asked}
        </span>
      </div>

      {session.left_off && (
        <div className="flex gap-[var(--space-xs)]">
          <span className={labelClass}>Left off</span>
          <span className="flex-1 text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]">
            {session.left_off}
          </span>
        </div>
      )}

      {session.touched_files.length > 0 && (
        <div className="flex gap-[var(--space-xs)]">
          <span className={labelClass}>Touched</span>
          <span className="flex flex-1 flex-wrap gap-[var(--space-2xs)]">
            {session.touched_files.map((f) => (
              <span
                key={f}
                className="mono rounded-[var(--radius-sm)] bg-[var(--color-bg-card)] px-[var(--space-2xs)] text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]"
              >
                {f}
              </span>
            ))}
            {session.touched_overflow > 0 && (
              <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                +{session.touched_overflow} more
              </span>
            )}
          </span>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Implement the row**

Create `src/sessions/SessionRow.tsx`:

```tsx
import type { SessionSummary } from '../lib/generated/bindings';
import { Button } from '../components/ui/Button';
import { ChevronDown, ChevronRight, Play } from '../lib/icons';
import { modelLabel } from '../report/SessionsTab';
import { SessionRecapCard } from './SessionRecapCard';

interface Props {
  session: SessionSummary;
  expanded: boolean;
  onToggle: (id: string) => void;
  onResume: (id: string) => void;
}

function ago(iso: string): string {
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return '';
  const mins = Math.round((Date.now() - t) / 60000);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.round(mins / 60);
  if (hours < 48) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

export function SessionRow({ session, expanded, onToggle, onResume }: Props) {
  const Chevron = expanded ? ChevronDown : ChevronRight;
  return (
    <div className="rounded-[var(--radius-sm)] border border-[var(--color-border)] bg-[var(--color-bg-card)] px-[var(--space-sm)] py-[var(--space-xs)]">
      <button
        type="button"
        onClick={() => onToggle(session.session_id)}
        aria-expanded={expanded}
        className="flex w-full items-center gap-[var(--space-xs)] text-left"
      >
        <Chevron size={13} aria-hidden className="shrink-0 text-[color:var(--color-text-muted)]" />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-[length:var(--text-body)] text-[color:var(--color-text)]">
            {session.title}
          </span>
          <span className="truncate text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
            {session.project_name}
            {session.git_branch && ` · ${session.git_branch}`}
            {` · ${session.turns} turn${session.turns === 1 ? '' : 's'}`}
          </span>
        </span>
        <span className="mono shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]">
          {session.model ? modelLabel(session.model) : 'unknown'}
        </span>
        <span className="shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          {ago(session.ended_at)}
        </span>
      </button>

      {expanded && (
        <>
          <SessionRecapCard session={session} />
          <div className="flex justify-end pt-[var(--space-2xs)]">
            <Button
              variant="primary"
              size="sm"
              onClick={() => onResume(session.session_id)}
              aria-label={`Resume ${session.title}`}
            >
              <Play size={13} aria-hidden />
              Resume
            </Button>
          </div>
        </>
      )}
    </div>
  );
}
```

- [ ] **Step 5: Implement the data hook**

Create `src/sessions/useResumableSessions.ts`:

```ts
import { useCallback, useEffect, useState } from 'react';
import type { SessionSummary } from '../lib/generated/bindings';
import { ipc } from '../lib/ipc';

export function useResumableSessions() {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      setSessions(await ipc.listResumableSessions());
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  return { sessions, loading, error, reload };
}
```

- [ ] **Step 6: Run**

```bash
npm test -- src/sessions && npm run lint
```

Expected: 7 row tests plus the 9 resolution tests PASS.

- [ ] **Step 7: Commit**

```bash
git add src/sessions
git commit -m "feat(sessions): session rows with expandable recap card"
```

---

## Task 7: Tab with grouping and search

**Files:**
- Create: `src/sessions/SessionsBrowserTab.tsx`, `src/sessions/__tests__/SessionsBrowserTab.test.tsx`

**Interfaces:**
- Consumes: `useResumableSessions` (Task 6), `SessionRow` (Task 6).
- Produces: `<SessionsBrowserTab />`.

- [ ] **Step 1: Write the failing tests**

Create `src/sessions/__tests__/SessionsBrowserTab.test.tsx`:

```tsx
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { SessionSummary } from '../../lib/generated/bindings';

const ipcMock = vi.hoisted(() => ({
  listResumableSessions: vi.fn(),
  listProviders: vi.fn().mockResolvedValue([]),
  listAvailableTerminals: vi.fn().mockResolvedValue(['ghostty']),
  launchProviderSession: vi.fn().mockResolvedValue('/tmp/a.sh'),
  getSettings: vi.fn().mockResolvedValue({ terminal: null }),
}));
vi.mock('../../lib/ipc', () => ({ ipc: ipcMock }));

import { SessionsBrowserTab } from '../SessionsBrowserTab';

function s(over: Partial<SessionSummary>): SessionSummary {
  return {
    session_id: 'id-1', cwd: '/w/alpha', project_name: 'alpha', git_branch: 'main',
    title: 'Alpha work', asked: 'do alpha', left_off: null,
    touched_files: [], touched_overflow: 0, model: 'claude-opus-5', turns: 3,
    started_at: '2026-07-29T10:00:00Z', ended_at: '2026-07-29T11:00:00Z', ...over,
  };
}

describe('SessionsBrowserTab', () => {
  beforeEach(() => vi.clearAllMocks());

  it('groups sessions under their project', async () => {
    ipcMock.listResumableSessions.mockResolvedValue([
      s({ session_id: 'a', project_name: 'alpha', title: 'Alpha work' }),
      s({ session_id: 'b', project_name: 'beta', cwd: '/w/beta', title: 'Beta work' }),
    ]);
    render(<SessionsBrowserTab />);
    await waitFor(() => expect(screen.getByText('Alpha work')).toBeTruthy());
    expect(screen.getByRole('heading', { name: 'alpha' })).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'beta' })).toBeTruthy();
  });

  it('filters on search and flattens the grouping', async () => {
    ipcMock.listResumableSessions.mockResolvedValue([
      s({ session_id: 'a', project_name: 'alpha', title: 'Alpha work' }),
      s({ session_id: 'b', project_name: 'beta', cwd: '/w/beta', title: 'Beta work' }),
    ]);
    render(<SessionsBrowserTab />);
    await waitFor(() => expect(screen.getByText('Alpha work')).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/search/i), { target: { value: 'beta' } });
    await waitFor(() => expect(screen.queryByText('Alpha work')).toBeNull());
    expect(screen.getByText('Beta work')).toBeTruthy();
    expect(screen.queryByRole('heading', { name: 'beta' })).toBeNull();
  });

  it('searches left_off and touched files, not just the title', async () => {
    ipcMock.listResumableSessions.mockResolvedValue([
      s({ session_id: 'a', title: 'Opaque title', left_off: 'the migration broke' }),
      s({ session_id: 'b', title: 'Other', touched_files: ['walker.rs'], cwd: '/w/beta', project_name: 'beta' }),
    ]);
    render(<SessionsBrowserTab />);
    await waitFor(() => expect(screen.getByText('Opaque title')).toBeTruthy());

    fireEvent.change(screen.getByLabelText(/search/i), { target: { value: 'migration' } });
    await waitFor(() => expect(screen.getByText('Opaque title')).toBeTruthy());

    fireEvent.change(screen.getByLabelText(/search/i), { target: { value: 'walker' } });
    await waitFor(() => expect(screen.getByText('Other')).toBeTruthy());
  });

  it('expands only one row at a time', async () => {
    ipcMock.listResumableSessions.mockResolvedValue([
      s({ session_id: 'a', title: 'First', asked: 'first ask' }),
      s({ session_id: 'b', title: 'Second', asked: 'second ask' }),
    ]);
    render(<SessionsBrowserTab />);
    await waitFor(() => expect(screen.getByText('First')).toBeTruthy());
    fireEvent.click(screen.getByText('First'));
    await waitFor(() => expect(screen.getByText('first ask')).toBeTruthy());
    fireEvent.click(screen.getByText('Second'));
    await waitFor(() => expect(screen.getByText('second ask')).toBeTruthy());
    expect(screen.queryByText('first ask')).toBeNull();
  });

  it('distinguishes empty-corpus from no-match', async () => {
    ipcMock.listResumableSessions.mockResolvedValue([]);
    const { rerender } = render(<SessionsBrowserTab />);
    await waitFor(() => expect(screen.getByText(/no sessions yet/i)).toBeTruthy());

    ipcMock.listResumableSessions.mockResolvedValue([s({})]);
    rerender(<SessionsBrowserTab key="2" />);
    await waitFor(() => expect(screen.getByText('Alpha work')).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/search/i), { target: { value: 'zzzz' } });
    await waitFor(() => expect(screen.getByText(/no sessions match/i)).toBeTruthy());
  });
});
```

- [ ] **Step 2: Run — must fail**

```bash
npm test -- src/sessions/__tests__/SessionsBrowserTab.test.tsx
```

- [ ] **Step 3: Implement**

Create `src/sessions/SessionsBrowserTab.tsx`:

```tsx
import { useMemo, useState } from 'react';
import type { SessionSummary } from '../lib/generated/bindings';
import { EmptyState } from '../components/ui/EmptyState';
import { useResumableSessions } from './useResumableSessions';
import { SessionRow } from './SessionRow';
import { useResume } from './useResume';

function matches(s: SessionSummary, q: string): boolean {
  const hay = [
    s.title, s.project_name, s.git_branch ?? '', s.model ?? '',
    s.asked, s.left_off ?? '', ...s.touched_files,
  ].join(' ').toLowerCase();
  return hay.includes(q);
}

export function SessionsBrowserTab() {
  const { sessions, loading, error } = useResumableSessions();
  const [query, setQuery] = useState('');
  const [expanded, setExpanded] = useState<string | null>(null);
  const { resume, dialog, notice } = useResume();

  const q = query.trim().toLowerCase();
  const filtered = useMemo(
    () => (q ? sessions.filter((s) => matches(s, q)) : sessions),
    [sessions, q],
  );

  // Search flattens the grouping: a two-result query should not be split
  // across two project headers.
  const groups = useMemo(() => {
    if (q) return null;
    const by = new Map<string, SessionSummary[]>();
    for (const s of filtered) {
      const list = by.get(s.project_name) ?? [];
      list.push(s);
      by.set(s.project_name, list);
    }
    return [...by.entries()];
  }, [filtered, q]);

  function toggle(id: string) {
    setExpanded((cur) => (cur === id ? null : id));
  }

  const rows = (list: SessionSummary[]) =>
    list.map((s) => (
      <SessionRow
        key={s.session_id}
        session={s}
        expanded={expanded === s.session_id}
        onToggle={toggle}
        onResume={() => resume(s)}
      />
    ));

  return (
    <div className="flex flex-col gap-[var(--space-sm)] p-[var(--space-md)]">
      <input
        aria-label="Search sessions"
        placeholder="Search sessions…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        className="w-full rounded-[var(--radius-sm)] border border-[var(--color-border)] bg-[var(--color-bg-base)] px-[var(--space-xs)] py-[var(--space-2xs)] text-[length:var(--text-body)] text-[color:var(--color-text)]"
      />

      {error && (
        <div role="alert" className="rounded-[var(--radius-sm)] border border-[var(--color-danger)] bg-[var(--color-danger-dim)] px-[var(--space-sm)] py-[var(--space-2xs)] text-[length:var(--text-micro)]">
          {error}
        </div>
      )}
      {notice && (
        <div role="status" className="rounded-[var(--radius-sm)] border border-[var(--color-warn)] bg-[var(--color-warn-dim)] px-[var(--space-sm)] py-[var(--space-2xs)] text-[length:var(--text-micro)]">
          {notice}
        </div>
      )}

      {!loading && !error && sessions.length === 0 && (
        <EmptyState title="No sessions yet" description="Sessions you run with Claude Code will appear here." />
      )}
      {!loading && sessions.length > 0 && filtered.length === 0 && (
        <p className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          No sessions match “{query}”.
        </p>
      )}

      {groups
        ? groups.map(([project, list]) => (
            <section key={project} className="flex flex-col gap-[var(--space-2xs)]">
              <h3 className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">{project}</h3>
              {rows(list)}
            </section>
          ))
        : rows(filtered)}

      {dialog}
    </div>
  );
}
```

- [ ] **Step 4: Add `EmptyState` props check**

`EmptyState` already exists at `src/components/ui/EmptyState.tsx`. Read it and match its actual prop names — if it does not accept `title` / `description`, adapt the call rather than changing the component.

- [ ] **Step 5: Commit (tests will fail until Task 8 provides `useResume`)**

Task 8 creates `useResume`. To keep this task runnable in isolation, create a temporary stub now and replace it in Task 8:

```tsx
// src/sessions/useResume.ts — replaced in Task 8
export function useResume() {
  return { resume: (_: unknown) => {}, dialog: null, notice: null as string | null };
}
```

```bash
npm test -- src/sessions && npm run lint
git add src/sessions
git commit -m "feat(sessions): browser tab with project grouping and search"
```

---

## Task 8: Resume with provider resolution and picker

**Files:**
- Modify: `src/sessions/useResume.ts` (replacing the Task 7 stub)
- Create: `src/sessions/ResumeProviderPicker.tsx`, `src/sessions/__tests__/useResume.test.tsx`

**Interfaces:**
- Consumes: `resolveProvider` (Task 5), `ipc.listProviders`, `ipc.listAvailableTerminals`, `ipc.getSettings`, `ipc.launchProviderSession` (Spec A Task 6).
- Produces: `useResume() -> { resume(session), dialog, notice }`.

**Why:** This is where §1.3's promise — never silently resume on the wrong model — is either kept or broken.

- [ ] **Step 1: Write the failing tests**

Create `src/sessions/__tests__/useResume.test.tsx`:

```tsx
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';

const ipcMock = vi.hoisted(() => ({
  listProviders: vi.fn(),
  listAvailableTerminals: vi.fn().mockResolvedValue(['ghostty']),
  getSettings: vi.fn().mockResolvedValue({ terminal: null }),
  launchProviderSession: vi.fn().mockResolvedValue('/tmp/a.sh'),
}));
vi.mock('../../lib/ipc', () => ({ ipc: ipcMock }));

import { useResume } from '../useResume';

const glmProvider = {
  id: 'glm', name: 'GLM', kind: 'third_party',
  base_url: 'https://api.z.ai/api/anthropic', auth_token: 't',
  env: { ANTHROPIC_MODEL: 'glm-5.2[1m]' }, extra_args: [], preset_id: 'glm', sort_index: 1,
};

const session = {
  session_id: 'sess-1', cwd: '/w/proj', project_name: 'proj', git_branch: 'main',
  title: 'T', asked: 'a', left_off: null, touched_files: [], touched_overflow: 0,
  model: 'glm-5.2', turns: 2, started_at: '', ended_at: '',
};

function Harness({ s }: { s: unknown }) {
  const { resume, dialog } = useResume();
  return (
    <>
      <button onClick={() => resume(s as never)}>go</button>
      {dialog}
    </>
  );
}

describe('useResume', () => {
  beforeEach(() => vi.clearAllMocks());

  it('launches directly when the model resolves, with the session cwd', async () => {
    ipcMock.listProviders.mockResolvedValue([glmProvider]);
    render(<Harness s={session} />);
    fireEvent.click(screen.getByText('go'));
    await waitFor(() =>
      expect(ipcMock.launchProviderSession).toHaveBeenCalledWith(
        'glm', '/w/proj', 'ghostty', 'sess-1',
      ),
    );
  });

  it('prompts instead of launching when the model does not resolve', async () => {
    ipcMock.listProviders.mockResolvedValue([glmProvider]);
    render(<Harness s={{ ...session, model: 'mystery-9' }} />);
    fireEvent.click(screen.getByText('go'));
    await waitFor(() => expect(screen.getByRole('dialog')).toBeTruthy());
    expect(ipcMock.launchProviderSession).not.toHaveBeenCalled();
  });

  it('prompts when no model was recorded', async () => {
    ipcMock.listProviders.mockResolvedValue([glmProvider]);
    render(<Harness s={{ ...session, model: null }} />);
    fireEvent.click(screen.getByText('go'));
    await waitFor(() => expect(screen.getByRole('dialog')).toBeTruthy());
    expect(ipcMock.launchProviderSession).not.toHaveBeenCalled();
  });

  it('warns about cross-model resume in the picker', async () => {
    ipcMock.listProviders.mockResolvedValue([glmProvider]);
    render(<Harness s={{ ...session, model: 'mystery-9' }} />);
    fireEvent.click(screen.getByText('go'));
    await waitFor(() => expect(screen.getByRole('dialog')).toBeTruthy());
    expect(screen.getByText(/thinking/i)).toBeTruthy();
  });

  it('launches with the chosen provider after confirmation', async () => {
    ipcMock.listProviders.mockResolvedValue([glmProvider]);
    render(<Harness s={{ ...session, model: 'mystery-9' }} />);
    fireEvent.click(screen.getByText('go'));
    await waitFor(() => expect(screen.getByRole('dialog')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /^resume$/i }));
    await waitFor(() =>
      expect(ipcMock.launchProviderSession).toHaveBeenCalledWith(
        'glm', '/w/proj', 'ghostty', 'sess-1',
      ),
    );
  });
});
```

- [ ] **Step 2: Run — must fail**

```bash
npm test -- src/sessions/__tests__/useResume.test.tsx
```

- [ ] **Step 3: Implement the picker**

Create `src/sessions/ResumeProviderPicker.tsx`:

```tsx
import { useState } from 'react';
import type { Provider, SessionSummary } from '../lib/generated/bindings';
import { ModalShell } from '../components/modals/ModalShell';
import { Button } from '../components/ui/Button';

interface Props {
  session: SessionSummary;
  providers: Provider[];
  onCancel: () => void;
  onConfirm: (providerId: string) => void;
}

export function ResumeProviderPicker({ session, providers, onCancel, onConfirm }: Props) {
  const [choice, setChoice] = useState(providers[0]?.id ?? '');
  const chosen = providers.find((p) => p.id === choice);
  const recorded = session.model ?? 'unknown';
  const differs = Boolean(session.model) && chosen?.env['ANTHROPIC_MODEL'] !== session.model;

  return (
    <ModalShell id="resume-picker" title="Which provider?" onDismiss={onCancel}>
      <div className="flex flex-col gap-[var(--space-sm)]">
        <p className="text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]">
          This session ran on <span className="mono">{recorded}</span>, which doesn’t match any
          provider you’ve configured.
        </p>

        <label className="flex flex-col gap-[var(--space-2xs)] text-[length:var(--text-micro)]">
          Resume with
          <select
            aria-label="Provider"
            value={choice}
            onChange={(e) => setChoice(e.target.value)}
            className="w-full rounded-[var(--radius-sm)] border border-[var(--color-border)] bg-[var(--color-bg-base)] px-[var(--space-xs)] py-[var(--space-2xs)] text-[length:var(--text-body)]"
          >
            {providers.map((p) => (
              <option key={p.id} value={p.id}>{p.name}</option>
            ))}
          </select>
        </label>

        {differs && (
          <p className="text-[length:var(--text-micro)] text-[color:var(--color-warn)]">
            Continuing on a different model discards the recorded thinking blocks (their
            signatures won’t validate), cold-starts the prompt cache, and changes the effective
            context window.
          </p>
        )}

        <div className="flex justify-end gap-[var(--space-xs)]">
          <Button variant="ghost" size="sm" onClick={onCancel}>Cancel</Button>
          <Button variant="primary" size="sm" onClick={() => onConfirm(choice)} disabled={!choice}>
            Resume
          </Button>
        </div>
      </div>
    </ModalShell>
  );
}
```

- [ ] **Step 4: Implement `useResume`**

Replace `src/sessions/useResume.ts`:

```tsx
import { useCallback, useState } from 'react';
import type { Provider, SessionSummary, Terminal } from '../lib/generated/bindings';
import { ipc } from '../lib/ipc';
import { resolveProvider } from './resolveProvider';
import { ResumeProviderPicker } from './ResumeProviderPicker';

export function useResume() {
  const [pending, setPending] = useState<{ session: SessionSummary; providers: Provider[] } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const pickTerminal = useCallback(async (): Promise<Terminal | null> => {
    const [settings, available] = await Promise.all([
      ipc.getSettings(),
      ipc.listAvailableTerminals(),
    ]);
    const configured = settings.terminal;
    return configured && available.includes(configured) ? configured : (available[0] ?? null);
  }, []);

  const launch = useCallback(
    async (session: SessionSummary, providerId: string) => {
      const terminal = await pickTerminal();
      if (!terminal) {
        setNotice('No supported terminal found. Install Ghostty or use Copy command.');
        return;
      }
      try {
        // The launcher appends --fork-session, so resuming a session that is
        // still open elsewhere cannot put two processes on one transcript.
        await ipc.launchProviderSession(providerId, session.cwd, terminal, session.session_id);
        setNotice(null);
      } catch (e) {
        setNotice(e instanceof Error ? e.message : String(e));
      }
    },
    [pickTerminal],
  );

  const resume = useCallback(
    async (session: SessionSummary) => {
      const providers = await ipc.listProviders();
      const resolution = resolveProvider(session.model, providers);
      if (resolution.kind === 'resolved') {
        await launch(session, resolution.providerId);
        return;
      }
      // Never guess: an unresolved model must be confirmed, or we risk
      // silently continuing a conversation on the wrong model.
      setPending({ session, providers });
    },
    [launch],
  );

  const dialog = pending ? (
    <ResumeProviderPicker
      session={pending.session}
      providers={pending.providers}
      onCancel={() => setPending(null)}
      onConfirm={async (providerId) => {
        const { session } = pending;
        setPending(null);
        await launch(session, providerId);
      }}
    />
  ) : null;

  return { resume, dialog, notice };
}
```

Rename the file to `useResume.tsx` (it returns JSX) and update the import in `SessionsBrowserTab.tsx`.

- [ ] **Step 5: Run**

```bash
npm test -- src/sessions && npm run lint
```

Expected: all PASS. If `ModalShell` requires the app store, mock it as documented in the Spec A plan (Task 8 Step 4).

- [ ] **Step 6: Commit**

```bash
git add src/sessions
git commit -m "feat(sessions): resume with provider resolution and confirmation picker"
```

---

## Task 9: Tab wiring, checklist, changelog

**Files:**
- Modify: `src/report/ExpandedReport.tsx`, `docs/release-checklist.md`, `CHANGELOG.md`

- [ ] **Step 1: Rename the accounting tab and add the browser**

In `src/report/ExpandedReport.tsx`:

1. Add the import:
```tsx
import { SessionsBrowserTab } from '../sessions/SessionsBrowserTab';
```
2. In `TAB_CONFIG`, change `{ id: 'sessions', label: 'Sessions' }` to:
```tsx
  { id: 'browse', label: 'Sessions' },
  { id: 'cost', label: 'Cost' },
```
placing `browse` first.
3. In `TAB_COMPONENTS`, replace the `sessions` entry with:
```tsx
  browse: SessionsBrowserTab,
  cost: SessionsTab,
```
4. Change the two `useState<string>('sessions')` / `useRef<string>('sessions')` initialisers to `'browse'`.

The file `SessionsTab.tsx` is **not** renamed — only its tab id and label — so `modelLabel` and `isHeadlessProject` keep their import paths.

- [ ] **Step 2: Verify nothing else references the old id**

```bash
grep -rn "'sessions'" src/ | grep -v "__tests__"
```

Expected: no hits outside test fixtures. `TAB_COMPONENTS[activeTab] ?? SessionsTab` already falls back safely, and `activeTab` is never persisted, so no migration is needed.

- [ ] **Step 3: Add the release-checklist block**

Append to `docs/release-checklist.md`:

```markdown
## Session browser (added 2026-07-29)

- [ ] Sessions tab lists real sessions grouped by project, newest project first
- [ ] **No subagent transcripts appear** — cross-check `ls ~/.claude/projects/*/*/subagents/*.jsonl | wc -l` against the row count; the browser must show none of them
- [ ] No headless (`-` project) sessions appear
- [ ] Expanding a row shows Asked / Left off / Touched and collapses any other open row
- [ ] **Left off is never tool output** — spot-check a session that ended on a tool result
- [ ] Search matches title, project, model, Asked, Left off, and a touched filename; results are flat, not grouped
- [ ] Empty-corpus and no-match states are distinct
- [ ] **A `glm-5.2` session resolves to the GLM provider** and resumes without a prompt (regression: `[1m]` normalization)
- [ ] A session with an unconfigured model opens the picker and warns about cross-model resume
- [ ] Resume opens a new terminal in the session's own folder; `/status` shows the expected endpoint
- [ ] **Resume a session that is still open in another terminal** — both windows keep working, the original transcript is unchanged, and the fork appears as a new row on rescan
- [ ] Cost tab is unchanged apart from its label
- [ ] Subagent backfill: `SELECT COUNT(DISTINCT source_file) FROM session_events WHERE source_file LIKE '%/subagents/agent-%'` returns the on-disk count, not 13
```

- [ ] **Step 4: Add the changelog entry**

In the `## Unreleased` section of `CHANGELOG.md`:

```markdown
### Added
- **Session browser.** A new Sessions tab lists past Claude Code sessions grouped by project, each expanding to show what you asked, where you left off, and which files it touched. One click resumes any session in a new terminal running the provider it originally used — resuming always forks, so a session still open elsewhere is never disturbed. The previous Sessions tab, which reports tokens and cost, is now called **Cost**.

### Fixed
- **Subagent usage was under-counted.** The startup backfill skipped subagent transcripts that the live watcher already collected, so sessions run while the app was closed never had their subagent API calls counted. Existing data is re-ingested automatically on upgrade.
```

- [ ] **Step 5: Full verification**

```bash
cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings
cd .. && npm test && npm run build
```

Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add src/report/ExpandedReport.tsx docs/release-checklist.md CHANGELOG.md
git commit -m "feat(sessions): wire browser tab, rename accounting tab to Cost"
```

- [ ] **Step 7: Manual smoke before declaring done**

Run `npm run tauri dev` and work the checklist block from Step 3. The two items that must not be waved through are the subagent-exclusion count and the `glm-5.2` direct resume — they are the C1 and C2 regressions, and both fail silently rather than loudly.

---

## Self-Review Notes

**Spec coverage**

| Spec section | Tasks |
|---|---|
| §2 Tabs (Cost/Sessions split) | 9 |
| §3 Inclusion filter, subagent exclusion, post-filter cap | 2, 3, 4 |
| §4 Recap (title, collapsed row, expanded card) | 3, 6 |
| §5 Grouping and search | 7 |
| §6 Model → provider resolution | 5 |
| §7 Resume, §7.1 always-fork | 8 (fork flag is enforced in the Spec A launcher) |
| §8 Architecture, memoization | 2, 3, 4 |
| §8.1 Subagent backfill prerequisite | 1 |
| §9 Rejected alternatives | n/a — no code |
| §11 Testing | inline across all tasks; manual smoke in 9 |

**Review findings incorporated**

| Finding | Where |
|---|---|
| C1 — subagent transcripts are separate files and pass the filter | Task 2 (`is_subagent_path`, regression test), Task 3 (path refusal), Task 9 checklist |
| C2 — `norm` applied to the wrong operand | Task 5, with an end-to-end test rather than a `norm`-only test |
| H1 — `tool_result` records are not user messages | Task 3 `is_real_user_text` + regression test |
| H2 — backfill skips subagent transcripts | Task 1, sequenced first |
| M1 — cap must be post-filter | Task 4 |
| M2 — memoize rather than re-parse per tab switch | Task 4 |
| M3 — tie-break, and deleted-provider reroute | Task 5 |
| M4 — cross-model resume warning | Task 8 |
| M5 — corpus description | spec only |
| Low — basename collisions, search scope, marker matching, resolution in TS | Task 3, Task 7, Task 3, Task 5 |

**Type consistency**

`SessionSummary` is defined once in Task 3 with thirteen fields and used with those exact names in Tasks 4, 6, 7 and 8, and in the TS fixtures. `Resolution` is defined in Task 5 and consumed in Task 8. `Provider` and `Terminal` come from Spec A and are used with the field names Spec A defines — note `extra_args`, added to `Provider` after the Spec A review, appears in every TS fixture here.

**Known forward reference**

Task 7 imports `useResume`, which Task 8 implements; Task 7 Step 5 creates a stub and Task 8 Step 4 replaces it. This mirrors the `ProviderForm` arrangement in the Spec A plan and is the only forward dependency.

**Sequencing note**

Tasks 1–7 have no Spec A dependency. Task 8 requires `ipc.launchProviderSession` and the `providers` table, so Spec A must land first — or Task 8 alone deferred. Task 1 is independent of everything and fixes a live data bug; it can be cherry-picked and shipped on its own.
