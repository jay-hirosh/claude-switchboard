# F1 — Live session registry + "Now running" popover row

**Status:** Design approved (user selected: liveness = transcript activity within 2 min)
**Date:** 2026-08-12
**Roadmap:** Phase 1 anchor in `docs/superpowers/roadmap-2026-08-12.md`. F2 (finished notifications) and F3 (context warnings) build on this registry — its state machine deliberately includes a "cooling" phase F2 needs.

## 1. Problem

The app is a rear-view mirror: rich history tabs, but nothing shows what Claude Code is doing *right now*. Users flip to terminals to check whether a session is still running and what it has cost so far.

## 2. Goals / non-goals

**Goals**
- A backend live-session registry: which sessions' transcripts are actively growing, with per-session accruing totals.
- A "Now running" section in the compact popover: `{project} · {model} · {cost} · {elapsed}`, hidden when nothing is live.
- The registry exposes the state F2/F3 need (cooling phase, per-session context tokens) so they are thin rules on top.

**Non-goals**
- No process/PID correlation (user chose transcript-activity liveness; PID↔session mapping is brittle and adds nothing at 2-min granularity).
- No per-second cost animation — totals update when the transcript does.
- No touching of the usage-API path (`usage_updated`, CachedUsage) — this is a parallel, JSONL-sourced wire.
- No F2/F3 behavior in this spec (they're separate specs), beyond the registry states they consume.

## 3. Liveness model

Session identity = the transcript's path relative to the projects root (`source_file`), exactly as `session_events` stores it; the file stem is Claude Code's session UUID. Subagent transcripts (`…/subagents/agent-*.jsonl`) fold into their parent key using the same `/subagents/` split the codebase already uses twice (`queries.rs:330-333`, `SessionsTab.tsx:131-136`).

State machine per session (constants in one place):

```
ingest touch ──> Live (last_activity < 120s)          — shown in popover
                   │ no writes for 120s
                   ▼
                 Cooling (120s ≤ quiet < 300s)        — hidden; F2's grace window
                   │ any write ──> back to Live
                   │ quiet ≥ 300s
                   ▼
                 Departed — F2 hook fires here (next spec), entry dropped
```

Only post-launch activity registers: the startup backfill (`lib.rs:651-665`) does NOT feed the registry — a session idle since before launch isn't "running". An active session reappears on its next transcript write (moments, for genuinely active sessions).

## 4. Architecture

### 4.1 Backend — `src-tauri/src/live_sessions.rs` (new module)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LiveSessionInfo {
    pub session_id: String,      // file stem of the parent transcript
    pub source_file: String,     // parent transcript, relative path (registry key)
    pub project: String,         // `project` of the file's most recent event
    pub model: String,           // model of the file's most recent MAIN-file event
    pub total_tokens: u64,       // input+output over parent + its subagent files
    pub total_cost_usd: f64,     //   (same aggregation as session_totals)
    pub context_tokens: u64,     // last MAIN-file event: input + cache_read + cache_5m + cache_1h
    pub first_seen: i64,         // unix seconds when the registry first saw it this run
    pub last_activity: i64,      // unix seconds of last ingest touch
}
```

- `LiveSessionRegistry { sessions: RwLock<HashMap<String, LiveEntry>> }` stored in `AppState`. `LiveEntry` = `LiveSessionInfo` + `state: Live|Cooling` (Departed entries are removed; F2 will add a notified-set).
- **Feed**: the watcher's per-file channel currently sends bare counts (`watcher.rs:23,49` → `lib.rs:667-674`). Change the channel payload to `(PathBuf, usize)`; the `lib.rs` consumer keeps emitting `session_ingested` with the count (unchanged frontend contract) and additionally calls `registry.note_ingest(&db, &path, now)`.
- **`note_ingest`**: fold subagent path → parent key; refresh the entry with two cheap queries scoped to this session: (a) totals over `source_file = parent OR source_file LIKE '{parent_stem}/subagents/%'` (same shape as `session_totals`, but single-session); (b) the parent file's most recent event (max `source_line`) for `project`, `model`, and the context-token sum. Set `last_activity = now`, `first_seen` on insert only. `context_tokens` comes from the parent file only — subagents have their own windows.
- **Prune tick**: a `tokio` interval (every 30 s, spawned in `lib.rs` setup alongside the watcher) walks the map: Live → Cooling at 120 s quiet, Cooling → removed at 300 s quiet. Any state-set change (ingest insert/update, transition, removal) emits `live_sessions_changed` to the frontend with the current `Vec<LiveSessionInfo>` of **Live** entries only, sorted by `last_activity` desc.
- **IPC**: `get_live_sessions() -> Vec<LiveSessionInfo>` (Live entries only), registered in **both** `collect_commands!` lists (`lib.rs:130-131` and `:181-182`).

### 4.2 Frontend

- `src/lib/store.ts`: `liveSessions: LiveSessionInfo[]` — initialized from `ipc.getLiveSessions()`, replaced wholesale on each `live_sessions_changed` event (payload is the full list; no client-side merging).
- New `src/popover/NowRunningSection.tsx`: renders nothing when the list is empty. Otherwise an uppercase "Now running" section label (matching Settings' section-header conventions), then up to 3 rows: project name (truncating) · short model badge (reuse `src/report/modelDisplay.ts`) · `formatCost(total_cost_usd)` (mono) · elapsed since `first_seen` (compact `12m` / `1h 05m`, reusing the duration helper added to `format.ts` for the Sessions tab). More than 3 live sessions → `+N more` muted line.
- Placement: `CompactPopover.tsx` home view, after `<UsageSummary/>`, before the footer — supplementary to the glance heroes, never displacing them.
- The popover already resizes to content; verify the section respects the existing height-measurement path (`CompactPopover` measures and requests window resize — new section simply participates).

## 5. Edge cases

- **Resumed sessions**: `first_seen` is registry insertion time (this app run), so "elapsed" reads as *how long it's been running now* — not the transcript's multi-day age. Deliberate.
- **Compaction mid-session**: the post-compaction event's context sum naturally drops; `context_tokens` follows the latest event, no special handling here (F3 cares, this spec just carries the number).
- **Two sessions in the same project**: rows are per-session; identical project labels are acceptable (model/cost/elapsed differentiate).
- **Registry vs. DB restart**: registry is in-memory only; app relaunch starts empty by design.
- **Headless/scheduled sessions** (no cwd → project label "headless" per existing convention): shown like any other; the Sessions tab already demotes the label, `modelDisplay`/`project` conventions carry over.

## 6. Testing

- Rust unit tests (`live_sessions.rs`): state transitions (fresh insert → Live; quiet 120s → Cooling; write during Cooling → Live; quiet 300s → removed); subagent path folds to parent key; totals aggregation includes subagent files; context sum uses main file's last event only.
- Rust: `note_ingest` against an in-memory DB seeded via existing test helpers (walker tests show the pattern, `walker.rs:245-290`).
- TS: `NowRunningSection` render tests — empty → nothing; 2 sessions → 2 rows with model badge/cost/elapsed; 5 sessions → 3 rows + "+2 more".
- Store: `live_sessions_changed` replaces the list.

## 7. Open questions

None blocking.

## 8. File-level checklist

New: `src-tauri/src/live_sessions.rs`, `src/popover/NowRunningSection.tsx` (+ test).
Modified: `src-tauri/src/jsonl_parser/watcher.rs` (channel payload), `src-tauri/src/lib.rs` (consumer, prune spawn, command registration ×2), `src-tauri/src/app_state.rs` (registry field), `src-tauri/src/commands.rs` (`get_live_sessions`), `src/lib/store.ts`, `src/lib/events.ts`, `src/lib/ipc.ts`, `src/lib/generated/bindings.ts` (hand-added, style-exact), `src/popover/CompactPopover.tsx`.
No DB schema change.
