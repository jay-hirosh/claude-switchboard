# F2 — Session-finished notifications

**Status:** Design approved (user selected: notify only for sessions ≥ 10 min)
**Date:** 2026-08-12
**Depends on:** F1's live-session registry (`2026-08-12-live-sessions-f1-design.md`) — specifically its Cooling→Departed transition.

## 1. Problem

Long-running agent work finishes silently; users babysit terminals to know when a background task is done.

## 2. Goals / non-goals

**Goals**
- One native notification when a session that ran ≥ 10 minutes goes quiet for good: `"Claude Code session finished"` / `"claude-switchboard — $2.10, 34m"`.
- A Settings toggle (`Notify when sessions finish`, default ON) in the Notifications card.

**Non-goals**
- No per-session opt-in UI.
- No distinction between "completed" and "abandoned" — quiet is quiet. (Claude Code's `away_summary` recap records exist in only ~50% of sessions and can be rewritten mid-session — too unreliable as a finish signal.)
- No notification history/log.

## 3. Behavior

Fires exactly at F1's **Cooling → Departed** transition (quiet ≥ 300 s), when all of:

- session's total live span `(last_activity − first_seen) ≥ 600 s` (the ≥10 min floor — measured on registry residency this run, consistent with F1's elapsed display);
- `settings.notify_session_finished` is true;
- this `session_id` hasn't already been notified this app run.

Notification content: title `Claude Code session finished`; body `{project} — {cost}, {duration}` where cost (`formatCost`-style `$2.10`) is omitted when `total_cost_usd == 0` (relay/free models), duration is the compact `34m` / `1h 12m` form. Shown via the same `tauri_plugin_notification` builder the threshold alerts use (`poll_loop.rs:284-291` pattern), but fired from the registry's prune tick — **not** the poll loop, and **not** gated on the active account slot (sessions aren't account-scoped).

Dedup is an in-memory `HashSet<String>` of notified session IDs in the registry. No DB table: a restart empties the registry anyway, so the set's lifetime matches the states it guards. A session that resumes activity *after* departing re-enters as a fresh registry entry (new `first_seen`) and may legitimately notify again — that's a new run of work.

## 4. Settings

- `Settings.notify_session_finished: bool`, default `true` (custom `Default` impl; struct-level `#[serde(default)]` handles old blobs — note: plain `bool` serde-defaults to `false` on *field* level, which is why it must be set in the struct's custom Default, same as every other field).
- `SettingsPanel.tsx` Notifications card: `Toggle` labeled "Session finished" with description "Notify when a session that ran 10+ minutes goes quiet." — first per-type notification toggle; placed after the threshold sliders.

## 5. Edge cases

- **App launched mid-session, session ends 5 min later**: registry residency < 10 min → no notification. Acceptable miss; the floor exists to prevent noise, and launch-adjacent misses are rare.
- **Rapid quiet/resume cycling around the 300 s boundary**: any write during Cooling returns the entry to Live with `first_seen` preserved, so a stuttering long session still accumulates span and notifies once at its true end.
- **Multiple sessions finish together**: one notification each — OS-level stacking is acceptable at this rarity.
- **Toggle flipped off mid-cool**: checked at fire time; no notification.

## 6. Testing

Rust unit tests in `live_sessions.rs` (the transition logic is pure given injected `now`):
- ≥10 min live span + quiet 300 s → fires once; second prune pass → no refire.
- 8-min session → never fires.
- Write during Cooling → no fire, span continues accumulating.
- `notify_session_finished = false` → no fire.
- Body formatting: with cost / zero cost / >1 h duration.

TS: SettingsPanel renders and persists the new toggle (existing settings-panel test conventions).

## 7. File-level checklist

Modified: `src-tauri/src/live_sessions.rs` (notified-set, transition hook, body formatting), `src-tauri/src/lib.rs` (pass notification handle/settings into prune tick), `src-tauri/src/app_state.rs` (settings field + Default), `src-tauri/src/commands.rs` (no validation needed — bool), `src/lib/generated/bindings.ts` (Settings field), `src/settings/SettingsPanel.tsx` (+ test).
No new files. No DB change. No new IPC.
