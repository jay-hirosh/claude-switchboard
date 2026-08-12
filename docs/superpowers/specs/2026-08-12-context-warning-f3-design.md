# F3 — Context-window warning

**Status:** Design approved (user selected: fixed 80% threshold, no settings slider)
**Date:** 2026-08-12
**Depends on:** F1's live-session registry (`2026-08-12-live-sessions-f1-design.md`) — consumes its per-session `context_tokens`.

## 1. Problem

Sessions hit auto-compaction with no warning; users lose the chance to wrap up a thought, commit, or hand off state cleanly before context gets summarized out from under them.

## 2. Goals / non-goals

**Goals**
- Native notification when a live session's context crosses 80% of its window: `"Session context at 82%"` / `"claude-switchboard — approaching compaction"`.
- A context % chip on the session's "Now running" row once it becomes relevant (≥ 60%), amber at ≥ 80%.
- A Settings toggle (`Context warnings`, default ON).

**Non-goals**
- No configurable threshold (fixed 80%; re-arm at 70%).
- No compaction *prediction* (time-to-compaction estimates) — just the level.
- No handling of subagent context windows (each subagent has its own window; the row tracks the parent session only, per F1).

## 3. Behavior

### Window resolution (Rust port of the frontend logic)

`src/sessions/contextWindow.ts` already resolves windows: an explicit `NATIVE_1M_MODELS` set → 1,000,000 tokens, everything else → 200,000. Port to `live_sessions.rs`:

```rust
/// Models whose registry entry sets context.native_1m — MUST stay in sync
/// with NATIVE_1M_MODELS in src/sessions/contextWindow.ts (the comment
/// there points back here). Explicit list, not a family prefix: the split
/// runs within families (Sonnet 5 is 1M, Sonnet 4.6 is 200K).
fn context_window_for(model: &str) -> u64
```

Matching mirrors the TS module's normalization (strip provider prefixes/`[1m]` suffix the same way it does). A one-line cross-reference comment goes into BOTH files; drift between the two lists is the known maintenance cost of having no shared source (acceptable at this list size, revisit if it grows).

### Trigger

Evaluated inside `note_ingest` (context only changes when the transcript writes):

- `pct = context_tokens * 100 / context_window_for(model)`
- Crossing ≥ 80 while armed → notify (if `settings.notify_context_warning`), disarm.
- Falling below 70 (compaction happened, or a resumed session started a fresh window) → re-arm. The 10-point hysteresis gap prevents flapping right at the boundary.
- Armed/disarmed is per registry entry (in-memory, dies with the entry — same lifetime rationale as F2's notified-set).

Title carries the actual figure (`Session context at 82%`), body `{project} — approaching compaction`. Fired from the ingest path via the same notification builder as F2.

### Popover chip (on F1's row)

`NowRunningSection` row gains a trailing mono chip `82%` rendered only when `pct ≥ 60`: `--color-text-muted` below 80, `--color-warn` at ≥ 80. Tokens only, no new colors. The chip reads as "context fullness" without a label — the Sessions tab already trains users on context readouts (`contextReadout`).

## 4. Settings

- `Settings.notify_context_warning: bool`, default `true` (custom `Default` impl, same pattern as F2's toggle).
- `SettingsPanel.tsx` Notifications card: `Toggle` labeled "Context warnings" with description "Notify when a session's context passes 80% of its window." — sits next to F2's toggle.

## 5. Edge cases

- **Compaction between two ingests**: next event's context sum is already post-compaction (F1 carries the latest value); pct drops below 70 → re-arm. If a session compacts and refills repeatedly, each genuine climb past 80 notifies — that's correct, each is a real approach to compaction.
- **Model switch mid-session** (`/model sonnet`): window resolves per the *latest* event's model, matching what the session is actually running.
- **Unknown/relay models**: default 200K window — conservative (warns early rather than never).
- **`context_tokens == 0`** (no assistant event yet in main file): pct 0, chip hidden, armed.

## 6. Testing

Rust unit tests (`live_sessions.rs`):
- `context_window_for`: each 1M model → 1M; unknown → 200K; `[1m]`-suffixed and prefixed ids normalize like the TS module's cases.
- Hysteresis: 79→81 fires once; 81→85 no refire; 85→65→82 fires again; toggle off → never fires.

TS: `NowRunningSection` chip — hidden at 45%, muted at 65%, warn-colored at 85% (extends F1's render tests).

## 7. File-level checklist

Modified: `src-tauri/src/live_sessions.rs` (window fn, hysteresis state, trigger in `note_ingest`), `src-tauri/src/app_state.rs` (settings field + Default), `src/sessions/contextWindow.ts` (cross-reference comment only), `src/lib/generated/bindings.ts` (Settings field), `src/popover/NowRunningSection.tsx` (chip, + test), `src/settings/SettingsPanel.tsx` (+ test).
No new files. No DB change. No new IPC.
