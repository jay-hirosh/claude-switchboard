# Today tab — root-level daily dashboard

**Status:** Design ready for review
**Date:** 2026-08-18
**Tracking PR:** TBD

## 1. Problem

Every existing report tab either aggregates all-time (Repo), a rolling multi-day window (Cost/Sessions: 7d, Models/Cache: 30d, Heatmap: 180d), or lets the user pick a range (Trends). None of them answer "what did I do *today*, all in one place" without mentally subtracting older rows out of a 7-day list or clicking through several tabs. The one piece of true calendar-day infrastructure that exists (`get_today_pattern`, added for the Trends → Daily pattern "Today" lookback option) proves the concept works but only covers the hour-of-day chart.

## 2. Goals / non-goals

**Goals**
- A new "Today" tab, first in the tab bar and the tab shown when the popover/window opens, that answers "what happened today" at a glance: headline totals, hour-by-hour activity, today's sessions, today's spend by repo, by model, and cache efficiency — all scoped to the local calendar day (midnight to now), not a rolling 24h window.
- Numbers agree with each other across sections (today's total cost on the headline row equals the sum of the session rows below it equals the sum of the model rows) and agree with what the Cost/Repo/Cache tabs would show if you filtered them to today by hand.

**Non-goals**
- No drill-down/expand affordances that duplicate what the dedicated tabs already do (compaction markers, subagent rows, per-account cache split, project-level nesting under a repo). Today is a summary; clicking through to Repo/Sessions/Cache/Trends is how you get the full detail.
- No new "today" concept for Models/Heatmap/Limit hits — out of scope for this pass.
- No changes to how any *existing* tab computes its own numbers.

## 3. UX model

Single scrolling dashboard (no inner sub-tabs), sections top to bottom:

1. **Headline row** — 3 stat cards: cost today, tokens today, sessions today (distinct conversations, same "distinct conversation" semantics `SessionsTab` already uses for its count).
2. **Hourly activity** — tokens-by-hour and cost-by-hour bars for today, identical data/visual to what `DailyPatternPanel`'s "Today" lookback already renders.
3. **Sessions today** — one row per conversation active today (project, model badge, account badge(s), clock time, tokens, cost), sorted newest first. Not expandable — no per-turn breakdown, no subagent/compaction rows. Capped at 50 rows with a "showing latest 50" note (today realistically never approaches that, but the cap keeps the contract explicit rather than implicit).
4. **By repo today** — one row per git repo touched today (repo name, session count, account badges, proportional bar, tokens, cost), same visual as a collapsed `RepoTab` card. No project-level expand.
5. **By model today** — one row per model used today (badge, proportional bar sized by that model's share of today's tokens, tokens, cost), visually matching the per-model rows `UsageTrends`' day-breakdown panel already renders for a clicked day.
6. **Cache today** — compact version of `CacheTab`: hit-rate ring, reads, writes, estimated savings. No by-account split.

**Empty state:** when zero events fall in today's window, show one empty state for the whole tab ("No activity yet today — usage will appear here once you start a session"), not six empty sections.

**Header caption:** the tab bar's header currently shows `Stale/Live · last N days` from `TAB_WINDOW_DAYS[activeTab]`. Today has no "last N days" — it always shows `· Today` instead when `activeTab === 'today'`.

## 4. Architecture

### 4.1 Backend — two new commands, following the `get_today_pattern` precedent

Naming follows the existing "daily" family (`get_daily_trends`, `get_daily_pattern`, `get_daily_model_breakdown`, `get_daily_account_breakdown` — modifier prefixed, not suffixed) and its one "today" sibling (`get_today_pattern`). The two new commands are `get_today_repo_breakdown` and `get_today_cache_stats`.

**`get_today_cache_stats`** — trivial: identical body to `get_cache_stats(days, state)` except the event source is `state.db.events_between(local_midnight_utc(Utc::now()), Utc::now())` instead of `get_session_history(days, state).await?`. Returns the existing `CacheStats` struct — no new type.

**`get_today_repo_breakdown`** — same output shape as `get_repo_breakdown` (`Vec<RepoStats>`, reusing the existing `RepoStats`/`RepoProjectStats` structs — no new types), but the source data differs in a way that needs two small, additive changes:

- `get_repo_breakdown` sources from `list_resumable_sessions`, whose `SessionSummary.total_tokens`/`total_cost_usd` are *lifetime* totals (filled from `Db::session_totals()`, which sums `session_events` with no time bound). A "today" version needs *today's* per-conversation totals, which only `events_between(local_midnight, now)` can give — but those `StoredSessionEvent` rows carry `source_file`/`project`/tokens/cost/`account_uuid`, not `cwd`. `cwd` (needed for `resolve_repo_name`'s `.git` walk) only exists in `SessionSummary`, which is built by parsing the JSONL transcript itself.
- So: add `pub source_file: String` to `SessionSummary` (`src-tauri/src/sessions/recap.rs`), documented like its `total_tokens`/`account_uuids` siblings ("not parsed from the transcript — filled in by `list_resumable_sessions` from the same `rel_str` it already computes to look up totals"). Populate it in `list_resumable_sessions` (`commands.rs`) at the point that already computes `rel_str` for the `totals`/`account_uuids` lookups (`commands.rs:2303-2312`) — `s.source_file = rel_str.into_owned()`.
- `get_today_repo_breakdown` then:
  1. `let events = state.db.events_between(local_midnight_utc(Utc::now()), Utc::now())?;`
  2. Fold events by parent source file (subagent paths collapse onto their parent — same split-on-`/subagents/` rule `Db::session_totals` already applies), summing tokens/cost and unioning `account_uuid` into a `Vec<Option<String>>`, into `HashMap<String, TodayTotals>` where `TodayTotals { tokens, cost, account_uuids }`.
     - Extract the parent-key split (currently inlined in `queries.rs::session_totals`, `commands.rs::245-` frontend has its own copy too) into one shared `pub fn parent_source_file(source_file: &str) -> String` in `store/queries.rs`, used by both `session_totals` and this new fold — one definition instead of a third copy.
  3. `let sessions = list_resumable_sessions(state).await?;` → build `HashMap<&str, &SessionSummary>` keyed by `source_file`.
  4. For each `(source_file, totals)` in the today-fold: look up `cwd`/`project_name` via the map. **Fallback for a miss** (a file whose `SessionSummary` isn't in the (cached, 200-row-capped) list — in practice unreachable for *today's* files, since `discover_session_files` orders newest-mtime-first and today's files are by definition the most recent, but not structurally impossible): use the event's own `project` field as both `cwd` and `project_name`, so the row still appears (as its own single-project "repo") rather than silently dropping today's spend.
  5. Group into `RepoStats`/`RepoProjectStats` via the existing `resolve_repo_name` + grouping logic in `group_repo_stats`. That function currently takes `&[SessionSummary]` and reads `.cwd`/`.project_name`/`.total_tokens`/`.total_cost_usd`/`.account_uuids` — narrow its input to a small local struct (`RepoStatsInput { cwd, project_name, total_tokens, total_cost_usd, account_uuids }`, `impl From<&SessionSummary>`) so both `get_repo_breakdown` (all-time) and `get_today_repo_breakdown` (today) call the same grouping function instead of forking the by-repo/by-project `HashMap` logic a second time.

Both new commands registered in **both** `collect_commands!` blocks in `lib.rs` (release and debug — every existing command is listed in both; see e.g. `get_today_pattern` at `lib.rs:127`/`190`).

### 4.2 Frontend

**`src/lib/ipc.ts`** — add:
```ts
getTodayRepoBreakdown: () => commands.getTodayRepoBreakdown().then(unwrap),
getTodayCacheStats: () => commands.getTodayCacheStats().then(unwrap),
```
No `src/lib/types.ts` changes — both commands return existing re-exported types (`RepoStats`, `CacheStats`).

**`src/lib/dayKey.ts`** (new) — `localDayKey(iso: string): string` and `formatDayLabel(dayKey: string): string`, moved verbatim out of `SessionsTab.tsx` (currently private there, `SessionsTab.tsx:32-59`). `SessionsTab.tsx` imports them from the new module instead of defining them locally. This is the exact function the Cost tab already relies on to agree with the backend's `chrono::Local` day bucketing (per its own doc comment) — `TodayTab` needs the identical logic to decide which of the fetched events are "today," so it must be the same function, not a second implementation that could drift.

**`src/report/trends/HourlyStrip.tsx`** (new) — `HourlyStrip` extracted verbatim out of `DailyPatternPanel.tsx` (currently a private function there, `DailyPatternPanel.tsx:261-317`, taking `metric`/`totals`/`windows` props). `DailyPatternPanel.tsx` imports it instead of defining it locally. `TodayTab`'s hourly section renders two `<HourlyStrip metric="tokens" .../>` / `<HourlyStrip metric="cost" .../>`, windows=[] (no warm-up shading — that's a Trends-specific overlay, not part of what "today" needs to show).

**`src/report/TodayTab.tsx`** (new) — the tab component, structured as one exported `TodayTab()` plus local section components (`HeadlineRow`, `SessionsSection`, `RepoSection`, `ModelSection`, `CacheSection`), matching the existing convention of keeping a tab's helper components in the same file (`SessionsTab.tsx` does this with `BreakdownTable`/`CompactionRows`/`SubagentRow`).

Data flow:
- One `useTabData` call fetching, in parallel: `ipc.getSessionHistory(1)`, `ipc.getPricing()`, `ipc.getTodayPattern()`, `ipc.getTodayRepoBreakdown()`, `ipc.getTodayCacheStats()`.
  - `getSessionHistory(1)` is provably sufficient for "today" regardless of timezone: the query window is `[now − 24h, now]`, and local midnight-today is always ≤ 24h before `now` (elapsed local time since midnight is always < 24h) — so the window is always a superset of today's local calendar day. `days: 2` was considered and rejected as unnecessary defensive padding for a bound that already holds exactly.
  - Events are filtered to `localDayKey(e.ts) === localDayKey(new Date().toISOString())` before anything downstream touches them.
- **Headline + Sessions + Models** all derive from that one filtered event array plus `getPricing()`:
  - Sessions: `aggregateSessions(todaysEvents, pricing)` (already exported from `SessionsTab.tsx`) — every row it returns already has `day === today` by construction, since all input events are pre-filtered, so no further filtering is needed on the result.
  - Headline cost/tokens: sum of the aggregated rows' `total_cost_usd`/`headline_tokens`. Session count: `aggregateSessions` rows' distinct conversation count, same de-dup rule `SessionsTab` uses (`id.slice(0, id.lastIndexOf('#'))`) — collapses to "just count the rows" here since every row already belongs to the same single day.
  - Models: fold `todaysEvents` by `model`, summing `input_tokens + output_tokens` (for the bar) and `cost_usd`, using the same `costPerCategory`/`lookupPricing` helpers `SessionsTab` already imports from `../lib/pricing` — this is a **new, smaller** fold than the backend's `accumulate_model_stats` (no `by_account` split needed for a summary row), written directly in `TodayTab.tsx` since it's short and has no other consumer.
- **Hourly activity** renders directly from `getTodayPattern()`'s `hourly_totals` via `<HourlyStrip>`.
- **Repo section** renders directly from `getTodayRepoBreakdown()`'s `RepoStats[]`.
- **Cache section** renders directly from `getTodayCacheStats()`'s `CacheStats`.

**`src/report/ExpandedReport.tsx`**:
- `TAB_CONFIG`: prepend `{ id: 'today', label: 'Today' }`.
- `TAB_COMPONENTS`: add `today: TodayTab`.
- `TAB_WINDOW_DAYS`: add `today: undefined`.
- Header caption: change the window-label expression from `TAB_WINDOW_DAYS[activeTab] ? ... : ''` to also branch on `activeTab === 'today' ? ' · Today' : ''`.
- `useState<string>('browse')` → `useState<string>('today')` (both the `activeTab` initial value and `prevTabRef`'s initial value, so the very first tab-slide direction computation is correct on mount).

## 5. Edge cases

- **Nothing happened yet today.** All five fetches succeed and return empty/zero data (not an error — `events_between` on an empty range is a valid empty result, same as every other tab's empty-history case). Render one whole-tab `EmptyState`, checked via `todaysEvents.length === 0` (the same signal already used, e.g., `RepoTab`'s `data.length === 0`).
- **App opened right after local midnight.** `getSessionHistory(1)`'s window still contains all of today (proof in §4.2) even when today is only seconds old; `getTodayPattern`/`getTodayRepoBreakdown`/`getTodayCacheStats` independently recompute `local_midnight_utc(Utc::now())` at call time, so a tab left open across midnight shows stale "yesterday" data until the next `sessionDataVersion`-triggered refetch — same staleness behavior every other tab already has (none of them re-derive "now" without a refetch).
- **A source_file with today activity has no matching `SessionSummary`.** Handled by the fallback in §4.1 step 4 (use `project` as a stand-in `cwd`) rather than dropping the row — today's total must always account for 100% of today's fetched events, or the headline card would disagree with the repo section's sum.
- **Timezone / DST boundary.** `local_midnight_utc` already has a passing unit test (`local_midnight_utc_is_start_of_todays_local_calendar_day`, `pattern.rs:430`) exercised by `get_today_pattern`; the two new commands reuse the same function, not a re-derivation, so they inherit that correctness.

## 6. Trade-offs

**Extending `SessionSummary` with `source_file` vs. re-walking the JSONL files a second time in the new command.** Considered giving `get_today_repo_breakdown` its own independent `discover_session_files` + `recap::parse_session` pass to get `cwd` without touching `SessionSummary`. Rejected: `list_resumable_sessions` already does exactly that work and caches it (keyed on newest-mtime, invalidated automatically on any new/appended transcript); a second independent walk would parse every session file's JSONL twice on every Today-tab load for data the cache already has. Adding one field is the smaller, more consistent change, and it's additive — nothing that reads `SessionSummary` today has to change.

**Folding today's events by model client-side vs. reusing `get_model_breakdown(1)`.** `get_model_breakdown(days)` uses the same rolling `[now-24h, now]` window as `get_session_history`, not a calendar-day one — calling it with `days: 1` would silently disagree with the rest of the tab (which is calendar-day-scoped) whenever "now" isn't within 24h of local midnight in a way that happens to line up. Folding the already-fetched, already-filtered `todaysEvents` client-side guarantees the model section's totals sum to exactly the headline card's totals, by construction, with no second network round trip.

**Non-expandable session/repo rows vs. reusing the full expandable components.** `SessionsTab`'s row (compaction markers, subagent nesting, five-category cost breakdown table) and `RepoTab`'s card (project-level expand) are both stateful, tab-specific components built around their own full-history data shape. Forcing them into a shared component generic enough for both the all-time/multi-day tabs and a same-day summary would add an abstraction with two call sites and diverging needs (today never has subagent/compaction detail worth a click, since there's rarely more than a handful of rows) for no real duplication saved — YAGNI. `TodayTab` renders its own short, flat rows sharing only the small presentational primitives every tab already shares (`Card`, `AccountBadge`, `ModelBadge`, `formatCost`, `formatTokens`).

## 7. Testing

- **Rust:** unit tests for `get_today_repo_breakdown`'s fold-and-join logic and the extracted `parent_source_file` helper, following the existing style of `session_totals_fold_subagents_onto_their_parent` (`queries.rs:1127`) and `local_midnight_utc_is_start_of_todays_local_calendar_day` (`pattern.rs:430`). No test for `get_today_cache_stats` — its non-sibling `get_cache_stats` has none either (thin bucketing commands aren't unit-tested at that layer in this codebase, per the precedent noted in the trends-day-model-breakdown spec).
- **Frontend:** `src/report/TodayTab.test.tsx`, same render+mock pattern as `TrendsTab.test.tsx`/`AccountRow.test.tsx` (`vi.hoisted` IPC mock, `render` + `@testing-library/react`), covering:
  - Empty state when all fetches return empty data.
  - Headline totals equal the sum of the rendered session rows, and equal the sum of the rendered model rows (the internal-consistency guarantee this whole design is built around).
  - An event from yesterday (present in the `getSessionHistory(1)` mock response, since that window can include a slice of yesterday) is excluded from every section.
  - Repo/cache sections render straight from their mocked IPC responses (no client-side recomputation to test there).
- **`src/lib/dayKey.test.ts`** (new) — unit tests for `localDayKey`/`formatDayLabel` now that they're a standalone module, covering the "Today"/"Yesterday"/short-date label branches and local-midnight parsing (`SessionsTab.test.ts` today only exercises them indirectly through `aggregateSessions`, not in isolation).

## 8. Open questions

None blocking.

## 9. File-level checklist

New:
- `src/lib/dayKey.ts` — `localDayKey`/`formatDayLabel` extracted from `SessionsTab.tsx`
- `src/report/trends/HourlyStrip.tsx` — extracted from `DailyPatternPanel.tsx`
- `src/report/TodayTab.tsx` — the tab
- `src/report/TodayTab.test.tsx`

Modified:
- `src-tauri/src/sessions/recap.rs` — `SessionSummary.source_file: String`
- `src-tauri/src/store/queries.rs` — extract `pub fn parent_source_file(source_file: &str) -> String`; `session_totals` uses it
- `src-tauri/src/commands.rs` — populate `SessionSummary.source_file` in `list_resumable_sessions`; narrow `group_repo_stats` to a small `RepoStatsInput` so both repo commands share it; add `get_today_repo_breakdown` and `get_today_cache_stats`
- `src-tauri/src/lib.rs` — register the two new commands in both `collect_commands!` blocks
- `src/lib/ipc.ts` — `getTodayRepoBreakdown`, `getTodayCacheStats`
- `src/report/ExpandedReport.tsx` — `today` tab entry, default tab, header caption
- `src/report/SessionsTab.tsx` — import `localDayKey`/`formatDayLabel` from `../lib/dayKey` instead of defining them
- `src/report/trends/DailyPatternPanel.tsx` — import `HourlyStrip` from `./HourlyStrip` instead of defining it

Unchanged: every other report tab's own IPC calls and rendering; `src-tauri/src/store/schema.sql` (no schema change — `source_file` is derived at read time from the same relative-path computation `list_resumable_sessions` already does, not persisted).
