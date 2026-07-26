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
  - One row per model, sorted by cost descending: `Badge` (short model name, reusing `ModelsTab`'s `shortName`/`modelKey`/`MODEL_VARIANT` helpers) + a proportional bar + tokens + cost, with a cache read/created sub-line — same visual language as `ModelsTab`'s model list, scoped to one day instead of 30.
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
    // bucket by (date, model), then flatten to Vec<DailyModelBucket> sorted by date
}
```

One pass over `get_session_history(days)` — the same fetch `get_daily_trends` and `get_model_breakdown` already do — bucketed by the composite key `(date, model)` instead of by date alone. Sibling function to the two existing ones; same style, same error handling (`err_to_string`).

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
- `src/report/TrendsTab.test.ts` (vitest, following `SessionsTab.test.ts`'s precedent), mocking `ipc.getDailyTrends` / `ipc.getDailyModelBreakdown`:
  - Clicking a bar reveals the panel with the correct per-model rows for that date, sorted by cost descending.
  - Clicking the same bar again collapses the panel.
  - Clicking a different bar switches the panel to the new date.
  - Switching the 7d/30d range clears the selection.

## 8. Open questions

None blocking.

## 9. File-level checklist

New:
- `src/report/modelDisplay.ts` — `shortName` / `modelKey` / `MODEL_VARIANT` extracted from `ModelsTab.tsx`
- `src/report/TrendsTab.test.ts`

Modified:
- `src-tauri/src/commands.rs` — `DailyModelBucket` struct + `get_daily_model_breakdown` command
- `src-tauri/src/lib.rs` — register new command in both `collect_commands!` blocks
- `src/lib/ipc.ts` — `getDailyModelBreakdown` wrapper
- `src/lib/types.ts` — re-export `DailyModelBucket`
- `src/report/TrendsTab.tsx` — fetch, selection state, click handler, inline panel
- `src/report/ModelsTab.tsx` — import `shortName`/`modelKey`/`MODEL_VARIANT` from the extracted module instead of defining them locally

Unchanged: `src-tauri/src/store/mod.rs`, all other report tabs, all IPC commands not listed above.
