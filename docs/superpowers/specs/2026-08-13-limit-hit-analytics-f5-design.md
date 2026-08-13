# F5 — Limit-hit analytics

**Status:** Design approved (user selected: new dedicated `window_peaks` table, new dedicated report tab, all managed accounts)
**Date:** 2026-08-13
**Depends on:** none (independent of F1–F4's live-session registry; reads `session_events` and a new table).

## 1. Problem

The roadmap (`docs/superpowers/roadmap-2026-08-12.md`, F5) asks "when do I actually run out?" — a report answering how often each rate-limit bucket gets hit, what time of day it happens, and which projects drove the consumption beforehand.

**Correction to the roadmap's premise:** F5 was scoped assuming `api_snapshots` holds "full usage-snapshot history per account" that could be mined directly with no schema change. It doesn't — `schema.sql` (`api_snapshots`) is pruned to the newest 50 rows per account (`lib.rs::prune_snapshots(50)`), and at the default 5-minute poll interval that's ~4 hours of retained data. Reading its only write site (`poll_loop.rs`), the table exists purely for cold-start rehydration, not analytics. A schema addition is required regardless of lookback window.

## 2. Goals / non-goals

**Goals**
- Track, going forward from ship date, the peak utilization reached in every finished 5H and 7D window, per managed account.
- A new report tab: per-account hit counts against the danger threshold, an hour-of-day distribution of when those peaks occurred, and the top projects that consumed the window beforehand.
- Covers every managed account, not just the active one.

**Non-goals**
- No retroactive history — `api_snapshots`' 50-row cap means there's nothing meaningful to backfill from; the report starts empty and builds up.
- No per-model (Opus/Sonnet) peak tracking — scoped to the two headline buckets (`five_hour`, `seven_day`). F4 already covers model-level divergence.
- No hardcoded 95% threshold — reuses the app's existing configurable danger threshold (`thresholds[1]`, default 90), the same "close to blocked" signal used everywhere else (F3, the meters, `bestAccount.ts`).
- No change to `api_snapshots`' retention or purpose — it stays a rehydration-only cache.
- Project attribution is not scoped to the specific account that hit its limit — `session_events` has no per-account dimension (JSONL transcripts don't record which account was authenticated when a session ran), so `top_projects` reflects local activity during the hit window(s) across all accounts, not this account specifically. Fixing this would require adding account attribution to the JSONL ingest pipeline — out of scope for this feature.

## 3. Behavior

### Schema — `window_peaks`

```sql
CREATE TABLE window_peaks (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id   TEXT NOT NULL,
    bucket       TEXT NOT NULL,       -- 'five_hour' | 'seven_day'
    resets_at    INTEGER NOT NULL,    -- the window's resets_at; its identity key
    window_start INTEGER NOT NULL,    -- earliest poll observed for this window
    peak_pct     REAL NOT NULL,
    peak_at      INTEGER NOT NULL,    -- when peak_pct was observed
    FOREIGN KEY (account_id) REFERENCES accounts(id)
);
CREATE UNIQUE INDEX idx_window_peaks_identity
    ON window_peaks(account_id, bucket, resets_at);
```

`(account_id, bucket, resets_at)` is the window's identity — the same trick F4's dismissal storage uses. A window rollover is *detected implicitly*: when a poll reports a new `resets_at` for a bucket, the UPSERT's unique-index miss creates a fresh row rather than updating the old one. No explicit "did the window just end" bookkeeping in application code.

### Poll-loop write path

In the same handler that already does `db.insert_snapshot(...)` (`poll_loop.rs`, `FetchOutcome::Ok` branch), for each of `five_hour` and `seven_day` present in the snapshot with a non-null `resets_at`:

```sql
INSERT INTO window_peaks (account_id, bucket, resets_at, window_start, peak_pct, peak_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?4)
ON CONFLICT(account_id, bucket, resets_at) DO UPDATE SET
    window_start = MIN(window_start, excluded.window_start),
    peak_pct = MAX(peak_pct, excluded.peak_pct),
    peak_at = CASE WHEN excluded.peak_pct > peak_pct THEN excluded.peak_at ELSE peak_at END
```

Best-effort, same as `insert_snapshot`: a failure logs a `tracing::warn!` and never interrupts polling.

### IPC command

`get_limit_hit_history(days: u32) -> LimitHitReport`, read-only, server-aggregated (matching `get_daily_trends`'s existing pattern rather than shipping raw rows to the frontend):

```rust
struct LimitHitReport {
    accounts: Vec<AccountLimitHits>,
}
struct AccountLimitHits {
    account_id: String,
    email: String,
    five_hour_hits: u32,       // peak_pct >= danger threshold, bucket = five_hour
    seven_day_hits: u32,       // same, bucket = seven_day
    hourly_distribution: [u32; 24],  // count of peak_at (local hour) across both buckets, hits only
    top_projects: Vec<ProjectAttribution>,  // session_events joined on window_start..peak_at, summed cost, top 5
}
struct ProjectAttribution {
    project: String,
    cost_usd: f64,
}
```

The danger threshold is read from `Settings` (already loaded server-side for the notifier), not passed by the caller.

### Frontend — new tab

New `LimitHitsTab.tsx`, wired into `ExpandedReport.tsx`'s existing `TAB_CONFIG` / `TAB_COMPONENTS` / `TAB_WINDOW_DAYS` maps (same pattern as the other 8 tabs), using `useTabData`. Content: a stat row per account (5H/7D hit counts), an hour-of-day bar chart, and a top-projects list. `EmptyState` (existing component) for accounts with zero rows — expected for a while after this ships, given goal #1's "no retroactive history."

## 4. Edge cases

- **App closed when a window starts:** `window_start` is the first poll observed *after* relaunch, not the window's true start — same class of approximation already accepted for the live-session liveness signal (roadmap's Phase 1 note). Attribution undercounts a project's early contribution in that case; peak tracking itself is unaffected (peaks are still caught as long as polling resumes before the window's true peak passes).
- **`resets_at` absent** (bucket temporarily null on an errored/unauthenticated poll): skip the upsert for that bucket on that poll; the existing row for the last-known `resets_at` is untouched.
- **First-ever poll for an account:** no prior row exists, so the UPSERT's insert branch runs — no special-cased "first poll" code path needed.
- **Danger threshold changed mid-history:** hit counts reflect the *current* setting at query time (computed from `peak_pct` at read time, not filtered at write time) — changing the threshold in Settings retroactively reclassifies existing history, which is the expected, simplest behavior (no need to store a second threshold-relative flag per row).

## 5. Testing

Backend (`queries.rs` style):
- Upsert: peak tracked correctly across multiple polls within one window; a new `resets_at` creates a new row rather than overwriting; `window_start` takes the MIN across polls; `peak_at` only advances when `peak_pct` strictly increases.
- Aggregation: hit-count threshold boundary; hour-of-day bucketing; project attribution join picks events strictly within `[window_start, peak_at]` and sums correctly; empty-history account returns zeroed struct, not an error.

Frontend (RTL, `TrendsTab.test.tsx` style):
- Empty state renders when an account has no `window_peaks` rows.
- Populated state renders hit counts, the hourly chart, and the top-projects list from a seeded `LimitHitReport`.

## 6. File-level checklist

New: `src-tauri/src/store/migrations/0010_window_peaks.sql` (next after `0009_session_compactions.sql`), `src/report/LimitHitsTab.tsx` (+ test).
Modified: `src-tauri/src/store/schema.sql` (table def, for fresh installs), `src-tauri/src/store/queries.rs` (upsert + aggregation query, + tests), `src-tauri/src/poll_loop.rs` (upsert call in the `FetchOutcome::Ok` branch), `src-tauri/src/commands.rs` (`get_limit_hit_history` command), `src/lib/generated/bindings.ts` (regenerated via specta), `src/lib/ipc.ts` (wrapper), `src/report/ExpandedReport.tsx` (tab wiring).
No change to `api_snapshots`, its prune policy, or the rehydration path.
