# Local-vs-synced data toggle

**Status:** Design ready for review
**Date:** 2026-08-31
**Tracking PR:** TBD

## 1. Problem

`docs/superpowers/specs/2026-08-21-cross-machine-archive-sync-design.md` (phase 2, "cross-machine archive sync") deliberately deferred device-level UI: it lands every device's raw JSONL lines into the local `transcript_lines`/`file_snapshots` archive tables, but explicitly scoped out "a device filter/badge in the report UI" as future work.

That gap turns out to be more than a missing filter: the table every dashboard/report tab actually reads (`session_events`, plus `session_compactions` for compaction markers) is populated **only** by the local JSONL file watcher (`jsonl_parser/walker.rs`). The sync engine pulls peer rows into the raw archive tables but never derives `session_events`/`session_compactions` from them. So today, synced data from other machines is invisible everywhere in the UI — every dashboard row is inherently local regardless of sync state.

This spec adds a Settings toggle to show/hide synced data in dashboards, which requires first wiring pulled archive rows through the same event-derivation pipeline the local watcher already uses.

## 2. Goals / non-goals

**Goals**
- A toggle in Settings: **off** (default) shows only this machine's data everywhere in the report UI; **on** shows this machine's data plus everything synced from other devices under the same account.
- Data pulled from other devices is derived into `session_events`/`session_compactions` (token/cost/compaction rows), not just archived as raw JSONL lines, so it can actually appear in dashboards.
- The toggle is a client-only display preference — it does not change what gets pushed, pulled, or archived, only what the report UI queries.

**Non-goals (this phase)**
- No per-device breakdown or badge ("came from MacBook Pro" vs "came from desktop") — peer-derived rows are tagged with the same collapsed `"synced-from-peer"` sentinel `transcript_lines`/`file_snapshots` already use, matching that existing convention. A real per-device identity would require a backend wire-format change (the pull response doesn't currently carry the origin device's id) and is out of scope here.
- No historical backfill of already-pulled `transcript_lines` rows from before this ships — derivation happens going forward, on each pull cycle's newly-received page. (Existing peer rows already in `transcript_lines` predate this feature in most users' databases and are typically small/recent given sync is newly launched; a one-time backfill can be a fast follow if needed.)
- No change to push behavior, backend schema, or wire format.

## 3. Architecture

```
Device B's JSONL file  ──(local watcher)──▶  session_events (device_id = B's own id)
                                          ╲
                                           ╲─(also archived)──▶ transcript_lines (device_id = B)
                                                                        │
                                                                (sync push/pull)
                                                                        │
                                                                        ▼
Device A's local store.db:              transcript_lines (device_id = "synced-from-peer")
                                                │
                                    (NEW: same parse_event_line/
                                     parse_compaction_line logic
                                     the local watcher uses)
                                                ▼
                                    session_events / session_compactions
                                    (device_id = "synced-from-peer")
                                                │
                     Settings toggle ──▶  events_between(from, to, only_device_id)
                                                │
                                                ▼
                                          Report UI tabs
```

The key move: reuse the existing local-ingestion parser (`jsonl_parser::record::{parse_event_line, parse_compaction_line}`) against pulled raw lines, instead of building a second parsing path. Both the local watcher and the sync-pull path end up calling the same functions and writing to the same tables — only the source of the raw line and the `device_id` stamp differ.

## 4. Data model

**Local (client) — schema v15 migration**, additive:
```sql
ALTER TABLE session_events      ADD COLUMN device_id TEXT NOT NULL DEFAULT '';
ALTER TABLE session_compactions ADD COLUMN device_id TEXT NOT NULL DEFAULT '';

-- Backfill: every row that exists today was necessarily produced by the
-- local JSONL watcher (the sync-derivation path this spec adds doesn't
-- exist yet), so all of them belong to this device.
UPDATE session_events      SET device_id = '<this install''s device_id>' WHERE device_id = '';
UPDATE session_compactions SET device_id = '<this install''s device_id>' WHERE device_id = '';
```
`<this install's device_id>` is read via the existing `Db::device_id()` (the `settings.sync_device_id` row already used for `transcript_lines`/`file_snapshots`) — no new identity concept.

Going forward:
- Local watcher (`jsonl_parser/walker.rs::ingest_file`) stamps new `session_events`/`session_compactions` rows with `Db::device_id()`, same as it already does for `transcript_lines`.
- Sync engine (`sync/engine.rs::run_sync_cycle`) stamps derived rows with the existing `SYNCED_FROM_PEER_DEVICE_ID = "synced-from-peer"` sentinel, same convention as `insert_pulled_transcript_lines`.

No change to `event_id`'s existing `UNIQUE` constraint — it already dedupes correctly across sources (same `requestId:message.id` wins regardless of which device produced the row), so a user who both syncs and later re-ingests the same file locally can't double-count.

## 5. Sync pipeline changes (Rust)

`run_sync_cycle` (`src-tauri/src/sync/engine.rs`) gains a `&PricingTable` parameter (already loaded once at startup for the watcher — passed through, not rebuilt). After each pulled page's `transcript_lines` are inserted via `insert_pulled_transcript_lines`, the same page's raw lines are run through `parse_event_line(raw_line, &project_slug)` / `parse_compaction_line(raw_line)`, priced via the pricing table, and inserted into `session_events`/`session_compactions` tagged with `SYNCED_FROM_PEER_DEVICE_ID`. `project_slug` (already present on `SyncTranscriptLine`) is passed as the fallback project, matching how the local walker passes its directory-derived project name — in practice `parse_event_line` prefers the JSONL's own `cwd` field when present, so this fallback rarely matters.

Insert failures here are logged and skipped, not propagated — matching the walker's existing "archive/derivation failures must never abort ingestion" posture (see its `insert_transcript_lines` error handling). A failure to derive one page's events must not stop the push/pull cycle or block later pages.

## 6. Query & command layer (Rust)

`Db::events_between` gains `only_device_id: Option<&str>`:
- `Some(id)` → `AND e.device_id = ?` (local-only)
- `None` → no filter (all devices)

All Tauri commands currently calling `events_between` gain a `local_only: bool` parameter and resolve it via `local_only.then(|| db.device_id()).transpose()?.as_deref()`: `get_session_history`, `get_today_pattern`, `get_yesterday_pattern`, `get_week_pattern`, `get_today_repo_breakdown`, `get_yesterday_repo_breakdown`, `get_week_repo_breakdown`, `get_cache_stats`, `get_today_cache_stats`, `get_yesterday_cache_stats`, `get_week_cache_stats`. Compaction-reading queries used for session-view compaction markers get the same `only_device_id` treatment.

A new `get_device_id` command exposes `Db::device_id()` to the frontend (not required for filtering itself, since that's a plain boolean, but useful for any future display/debug need).

## 7. Frontend (TypeScript)

New store `src/lib/dataScope.ts`, mirroring the existing `density`/`reduceMotion` pattern in `src/lib/appearance.ts`: Zustand + `localStorage` persistence, applied instantly, no Save button.
```ts
showAllDevices: boolean  // default false
```

UI: a `Toggle` (`src/components/ui/Toggle.tsx`, the existing convention used throughout Settings) added to the Sync section (`SyncSettings.tsx`, rendered from `SettingsPanel.tsx:511-531`). Label: "Show data synced from other devices." Description: "When off, dashboards show only this machine's activity."

Every report tab that calls `events_between`-backed ipc functions (`DashboardTab`, `SessionsTab`, `TrendsTab`/`UsageTrends`, `HeatmapTab`, cost views) reads `showAllDevices` from the store and passes `local_only: !showAllDevices` into its existing ipc calls, refetching on toggle change via the same `useEffect`-dependency pattern those tabs already use for other filters.

## 8. Testing

**Rust:**
- Migration backfill: pre-existing `session_events`/`session_compactions` rows get this device's real id.
- `run_sync_cycle` derives `session_events`/`session_compactions` from a pulled page, tagged with the peer sentinel (extends the existing mocked push/pull test suite in `sync/engine.rs`).
- `events_between`'s new filter: local-only excludes peer-tagged rows; `None` includes both (extends existing `queries.rs` test patterns, e.g. `events_between_attributes_account_via_interval_join`).

**Frontend:**
- `dataScope` store: persists to `localStorage`, defaults to `false`.
- A report tab passes the correct `local_only` value into its ipc call for both toggle states.

## 9. Migration / rollout notes

Existing users upgrading see no behavior change on first launch (toggle defaults off, matching current — pre-feature — behavior exactly). Users with sync already configured will see synced data appear in dashboards only for pull cycles that happen *after* upgrading (per the non-backfill decision in §2); the raw `transcript_lines` archive itself is untouched and already has everything, so a future backfill pass (if added) has full data to work from.
