# Trends tab — per-model breakdown on day click

**Status:** Design ready for review
**Date:** 2026-07-26
**Tracking PR:** TBD

## 1. Problem

The Trends tab chart's hover tooltip shows a day's total tokens and cost, but not which models contributed. The Models tab has per-model breakdown, but only as a single 30-day aggregate — there's no way to see "which models ran on July 24th, and how much did each cost."

## 2. Goals / non-goals

**Goals**
- Clicking a day's bar reveals that day's per-model breakdown (tokens, cost, cache read/creation) inline, directly below the chart.
- Instant reveal — no loading spinner on click.

**Non-goals**
- No per-click IPC round trip.
- No changes to the Models tab itself (its 30-day aggregate is unaffected).
- No multi-day selection/comparison — one selected day at a time.

## 3. UX model

- Click a bar → toggles `selectedDate`. Click the same bar again → collapses. Click a different bar → switches directly to the new day.
- Inline panel appears below the chart (above the existing avg/total summary row) when a date is selected:
  - Header: date, day total tokens, day total cost (same values already shown in the hover tooltip).
  - One row per model, sorted by cost descending: `Badge` (short model name, reusing `ModelsTab`'s `shortName`/`modelKey`/`MODEL_VARIANT` helpers) + a proportional bar (fill % = that model's tokens ÷ **the selected day's** total tokens — not the 30-day total `ModelsTab` uses) + tokens + cost. This much — badge, bar, pct, cost — mirrors `ModelsTab.tsx:138-170`'s row shell directly.
  - Below each row, a cache read/created sub-line. This is **new markup**, not a reuse: `ModelsTab` has no per-model cache figures in its row list (`ModelsTab.tsx:138-170` is badge/bar/pct/cost only) — cache numbers there only exist in the separate aggregate "Cache efficiency" card (`ModelsTab.tsx:174-192`). The per-model-per-day sub-line is this feature's own addition, built from `ModelStats.cache_read_tokens` / `cache_creation_tokens`, which the struct already carries.
- Every rendered bar already corresponds to a day with ≥1 event (`get_daily_trends` only emits days that have events), so every clickable day is guaranteed a non-empty breakdown.

## 4. Architecture

### 4.1 Backend — new command

`src-tauri/src/commands.rs`:

```rust
#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct DailyModelBucket {
    pub date: String,
    pub models: Vec<ModelStats>,
}

#[command]
#[specta::specta]
pub async fn get_daily_model_breakdown(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<DailyModelBucket>, String> {
    let events = get_session_history(days, state).await?;
    // bucket by (date, model) into BTreeMap<String, HashMap<String, ModelStats>>,
    // then flatten: for each date, collect its models and
    // .sort_by(|a, b| b.cost_usd.total_cmp(&a.cost_usd)) before pushing into
    // the Vec<DailyModelBucket> (outer order is date-ascending, free from the
    // BTreeMap key).
}
```

One pass over `get_session_history(days)` — the same fetch `get_daily_trends` and `get_model_breakdown` already do — bucketed by the composite key `(date, model)` instead of by date alone. Sibling function to the two existing ones; same style. Note on error handling: like its siblings, this function has no fallible operation of its own after `get_session_history(...).await?` — the `Result<_, String>` it can return is entirely inherited from that call (whose own `String` errors come from `err_to_string` inside `events_between`), not produced here.

**Sort is mandatory, not incidental.** `get_daily_trends` gets date-ascending order for free from its `BTreeMap<String, DailyBucket>` key. `get_model_breakdown` does not sort — it returns `HashMap<String, ModelStats>::into_values().collect()`, and Rust's `HashMap` iteration order is randomized per process. The new command's per-day model list would inherit that same unordered-ness unless the flatten step explicitly sorts each day's `Vec<ModelStats>` by `cost_usd` descending, per the code comment above.

Registered in **both** `collect_commands!` blocks in `lib.rs` (this project runs two `tauri-specta` builders — one exports the TS bindings, one is the real invoke handler — every existing command is already listed in both).

### 4.2 Frontend

`src/lib/ipc.ts` — add `getDailyModelBreakdown: (days: number) => commands.getDailyModelBreakdown(days).then(unwrap)`.

`src/lib/types.ts` — re-export `DailyModelBucket` alongside the existing `DailyBucket`/`ModelStats` re-exports.

`src/report/TrendsTab.tsx`:
- Fetch `getDailyModelBreakdown(30)` in parallel with `getDailyTrends(30)` via `Promise.all` inside the existing `useTabData` call (same pattern `ModelsTab` uses for `getModelBreakdown` + `getCacheStats`). Both are loaded before the tab renders, so breakdown data is already in memory by the time any bar is clickable.
- Add `const [selectedDate, setSelectedDate] = useState<string | null>(null)`.
- Bar `onClick`: `setSelectedDate(d => d === day.date ? null : day.date)`.
- Look up `breakdown.find(b => b.date === selectedDate)` to render the panel — pure local array find, no fetch.
- Extract `shortName` / `modelKey` / `MODEL_VARIANT` from `ModelsTab.tsx` into a shared module (e.g. `src/report/modelDisplay.ts`) so both tabs use the same model-label logic instead of duplicating it.

## 5. Edge cases

- **Selected day drops out of view.** Switching the 7d/30d range toggle, or a `sessionDataVersion` bump reshaping `visibleData`, can leave `selectedDate` pointing at a day no longer rendered. The panel keys off `breakdown.find(...)`, so if the date isn't found it simply renders nothing — but to avoid an invisible "stuck" selection, clear `selectedDate` in a `useEffect` when `range` changes.
- **New data arrives while a day is selected** (`sessionDataVersion` bump mid-view). Both queries re-run via `useTabData`'s existing dependency array; the panel re-renders with fresh numbers for the same `selectedDate` if it's still present, or clears per the rule above if not.

## 6. Trade-offs

**Single upfront command vs. lazy per-click fetch.** Considered fetching a day's breakdown on-demand per click (`get_model_breakdown_for_date(date)`), mirroring `get_model_breakdown`'s signature more closely. Rejected: `get_daily_trends` already re-reads the full `days`-window event history from SQLite once when the tab loads; doing that same full read again on every single click (to filter down to one day) is repeated work for data that's already available, and it introduces a per-click loading flicker. The single-command approach costs one extra (cheap) query at tab-load time and makes every subsequent click free.

## 7. Testing

- No new Rust unit test. Following existing convention: neither `get_daily_trends` nor `get_model_breakdown` (the two commands this most closely mirrors) has unit test coverage anywhere in `src-tauri/tests/` or inline — these thin bucketing commands aren't tested at that layer in this codebase, so `get_daily_model_breakdown` follows suit.
- `src/report/TrendsTab.test.tsx` (note the `.tsx` — this is a render test, not a pure-function test). `SessionsTab.test.ts` is the wrong precedent: it only imports and asserts on pure helpers (`aggregateSessions`, `modelLabel`), no `render`, no IPC mock. The actual precedent is `src/accounts/__tests__/AccountRow.test.tsx`'s pattern — `vi.hoisted` IPC mock, `vi.mock('../../lib/ipc', () => ({ ipc: ipcMock }))`, `render` + `fireEvent` from `@testing-library/react`. `TrendsTab.test.tsx` mocks `ipc.getDailyTrends` / `ipc.getDailyModelBreakdown` the same way and exercises:
  - Clicking a bar reveals the panel with the correct per-model rows for that date, sorted by cost descending.
  - Clicking the same bar again collapses the panel.
  - Clicking a different bar switches the panel to the new date.
  - Switching the 7d/30d range clears the selection.

## 8. Open questions

None blocking.

## 9. File-level checklist

New:
- `src/report/modelDisplay.ts` — `shortName` / `modelKey` / `MODEL_VARIANT` extracted from `ModelsTab.tsx`
- `src/report/TrendsTab.test.tsx` (co-located with `TrendsTab.tsx`, matching `SessionsTab.test.ts`'s co-location within `src/report/` rather than the `__tests__/` subfolder convention used in `src/accounts/` — the file's *content* follows `AccountRow.test.tsx`'s render+mock pattern, per §7; its *location* follows this directory's existing sibling)

Modified:
- `src-tauri/src/commands.rs` — `DailyModelBucket` struct + `get_daily_model_breakdown` command
- `src-tauri/src/lib.rs` — register new command in both `collect_commands!` blocks
- `src/lib/ipc.ts` — `getDailyModelBreakdown` wrapper
- `src/lib/types.ts` — re-export `DailyModelBucket`
- `src/report/TrendsTab.tsx` — fetch, selection state, click handler, inline panel
- `src/report/ModelsTab.tsx` — import `shortName`/`modelKey`/`MODEL_VARIANT` from the extracted module instead of defining them locally

Unchanged: `src-tauri/src/store/mod.rs`, all other report tabs, all IPC commands not listed above.
