# Per-account session attribution

## Problem

Claude Switchboard manages multiple accounts by swapping which one's credentials are live in Claude Code. Only the **Limit hits** tab currently shows genuinely per-account data (it's built from live API polling of every managed account, which always knows account identity). Every other data tab — Cost, Models, Trends, Repo, Heatmap, Cache — is built from `session_events`, a local cache of parsed `~/.claude/projects/*.jsonl` transcripts. Those transcripts carry no account identity at all, so when a user has multiple accounts, every one of those six tabs silently mixes all accounts' activity together with no way to tell them apart.

Goal: attribute session data to the account that was active when it happened, and surface that attribution as close to per-row as each tab's data shape allows.

## Non-goals

- **Retroactive attribution of history.** No account-swap history has ever been recorded, so sessions logged before this ships have no way to know which account was live. They are permanently labeled "Unknown account." This is a hard constraint, not a phased rollout choice.
- **Sub-60-second precision for swaps made outside Switchboard.** An in-app swap (`swap_to_account`) is recorded immediately and exactly. A swap made another way (manual `claude login`, another tool, another machine sharing the same account) is only detected the next time the poll loop reconciles the live account, which runs at least every 60 seconds. Session events that land in that window are attributed to whichever account the reconciliation lands on — there's no way to do better without changing what Claude Code itself logs.
- **A global account filter.** Considered and rejected in favor of in-place attribution (see brainstorm history) — every tab keeps showing all accounts' data at once, tagged, rather than requiring the user to switch a filter to compare.

## Architecture

### 1. `account_intervals` table

New table, migration `0012_account_intervals.sql` (schema version 11 → 12, following the existing pattern in `store/mod.rs::migrate()` — one `if current < 12` block executing the migration file, plus the matching `CREATE TABLE IF NOT EXISTS` added to `schema.sql`):

```sql
CREATE TABLE IF NOT EXISTS account_intervals (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    account_uuid TEXT NOT NULL,
    started_at   INTEGER NOT NULL,
    ended_at     INTEGER,               -- NULL = still the active account
    FOREIGN KEY (account_uuid) REFERENCES accounts(id)
);
CREATE INDEX IF NOT EXISTS idx_account_intervals_span
    ON account_intervals(started_at, ended_at);
```

At most one row has `ended_at IS NULL` at a time. An event's account is whichever interval satisfies `started_at <= ts AND (ended_at IS NULL OR ts < ended_at)`; an event whose `ts` falls in no interval (before Switchboard ever recorded one, or a gap where no managed account was live) has no attribution.

### 2. Recording transitions

A single `Db` method, `record_account_transition(new_active: Option<&str>, at: DateTime<Utc>)`, does the work transactionally: close any currently-open interval (`ended_at = at`) if its account differs from `new_active`, then open a new interval for `new_active` if it's `Some` and different from what was already open. No-ops if `new_active` matches the currently-open interval's account, so both call sites below can call it freely without producing spurious zero-length intervals.

Two call sites, both already computing "the active account changed":

- **`commands.rs::swap_to_account`**, right where it currently sets `*state.active_since.write() = Some(Utc::now())` (~line 1065) — call `state.db.record_account_transition(Some(&target_uuid), Utc::now())` there too. This is the precise, immediate path for in-app swaps.
- **`poll_loop.rs::poll_all`**, in the existing `if prev_active_slot != active_slot` block (~line 436-441) that already fires `active_since` and the `accounts_changed` event — add the same call, resolving `active_slot`'s `account_uuid` from `accounts`. This is what catches everything that doesn't go through Switchboard's own swap command, at the same cadence (≤60s) the app already uses to detect those changes today.

Both sites converge on the same `record_account_transition`, so an in-app swap that's also observed a moment later by the poll loop is a no-op the second time.

### 3. Query-layer join

`StoredSessionEvent` (`store/queries.rs`) gains `pub account_uuid: Option<String>`. `events_between` and `session_totals` (`store/queries.rs`) add a `LEFT JOIN account_intervals ON ts >= started_at AND (ended_at IS NULL OR ts < ended_at)` (SQLite allows a non-equality join condition here; both queries are already bounded by a `ts` range so this stays cheap). Every report command downstream of `events_between` — `get_session_history`, `get_daily_trends`, `get_daily_model_breakdown`, `get_project_breakdown`, `get_cache_stats` — inherits the field for free since they all aggregate in Rust from `events_between`'s output; only the ones whose aggregation is per-model or per-day-only need to additionally group by `account_uuid` (below).

The frontend already loads the full account list (`ipc.listAccounts`) wherever the report UI lives, so `account_uuid → email` resolution and a stable per-account color are derived client-side rather than duplicating email strings into every event. Color mapping: a fixed palette indexed by each account's existing `slot` number (`palette[slot % palette.length]`), so colors stay stable as accounts are added/removed. `account_uuid: null` always renders as "Unknown account" in a fixed muted gray, sorted last in any legend.

### New/extended backend surface

| Command | Change |
|---|---|
| `get_session_history` | `StoredSessionEvent` gains `account_uuid: Option<String>` — no signature change. |
| `get_model_breakdown` | Each entry gains `by_account: Vec<{ account_uuid: Option<String>, tokens: u64, cost_usd: f64 }>`. |
| `get_daily_account_breakdown(days)` | **New command**, structurally mirrors the existing `get_daily_model_breakdown` but groups by `account_uuid` instead of `model` per day. Backs both the Trends "color by account" toggle and the Heatmap dominant-account indicator (called with 180 days for the latter). |
| `get_cache_stats_by_account(days)` | **New command**, same aggregation as `get_cache_stats` (hit ratio, savings) but grouped by account — returns `Vec<AccountCacheStats>`, one entry per account that had cache activity in the window, shaped like `LimitHitHistory`'s existing per-account array. |
| `get_repo_breakdown` | Each `RepoStats`/`ProjectStats` entry gains `account_uuids: Vec<String>` — the distinct accounts whose events fall inside that repo/project's constituent sessions, via a new query resolving distinct `account_uuid`s per `source_file` through the same interval join. |

## Per-tab UI treatment

Reference: "account badge" = a small colored dot + short label (account's display name or email local-part), same visual weight as the existing `ModelBadge`. "Unknown account" always renders muted/gray.

- **Cost tab** (`SessionsTab.tsx`): each row is already one conversation-day (`AggregatedSession`). Add an account badge next to the existing model badge. `aggregateSessions` collects the distinct `account_uuid`s seen among a row's events; the overwhelmingly common case is one — if a conversation happens to straddle a swap mid-day, show all badges present rather than picking one.
- **Repo tab** (`RepoTab.tsx`): each repo card gets badge(s) for every account in its new `account_uuids` field, next to the existing session count.
- **Models tab** (`ModelsTab.tsx`): rows are per-model sums with no session left to tag. Add a thin per-account split bar under each model row, using `by_account`, in the same visual language as the donut segments above it.
- **Trends tab** (`TrendsTab.tsx`): the day bars are already stacked and colored by model tier with a toggleable legend. Add a second toggle, "Color by: Model / Account," which re-renders the same stacking mechanism from `get_daily_account_breakdown` instead of `get_daily_model_breakdown`.
- **Heatmap tab** (`HeatmapTab.tsx`): cells stay one aggregate intensity value, but each cell gets a small ring in its dominant account's color (from `get_daily_account_breakdown(180)`), and the hover tooltip lists the full split, e.g. "work 80% · personal 20%."
- **Cache tab** (`CacheTab.tsx`): restructures from one aggregate ring into small per-account cards (email + its own hit-rate ring and savings figure), sourced from `get_cache_stats_by_account` — effectively a compact version of how Limit Hits already presents per-account data. The existing aggregate ring stays as a "Total" summary card above the per-account list.
- **Limit hits tab**: already per-account; unchanged.
- **Providers / Sessions (browse) tabs**: out of scope — neither is a usage-analytics view keyed by historical session data.

## Testing

- Backend: unit tests for `record_account_transition` covering close-then-open, no-op on same account, and the "no prior interval" cold-start case; `events_between`/`session_totals` join tests asserting correct attribution across an interval boundary and `NULL` attribution for timestamps outside any interval; one test per new/extended command exercising the account grouping.
- Frontend: extend `aggregateSessions`'s existing test coverage (`report/SessionsTab.test.ts`) with a case spanning two accounts in one day; add coverage for the new per-tab account-derived views (`ModelsTab.test.tsx`, `TrendsTab.test.tsx`) following each file's existing pattern.
- Manual: exercise with two managed accounts, swap between them mid-session, and confirm attribution lands correctly across all six tabs plus the ~60s external-swap detection path (swap via `claude login` outside Switchboard rather than the in-app swap button).

## Rollout

Purely additive: new table, new columns/fields, new commands. No existing data is modified or re-ingested. On upgrade, `account_intervals` starts empty, so every tab's account attribution reads "Unknown account" for all history until the next swap (in-app or detected) opens the first real interval.
