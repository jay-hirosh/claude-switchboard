# Trends Tab Day-Model Breakdown Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Clicking a day's bar on the Trends tab reveals that day's per-model token/cost/cache breakdown in an inline panel below the chart, with zero per-click IPC latency.

**Architecture:** A new Tauri command `get_daily_model_breakdown` does a single pass over the same event history `get_daily_trends`/`get_model_breakdown` already fetch, bucketed by `(date, model)`. The frontend loads it once alongside `getDailyTrends` on tab mount; clicking a bar is a pure local array lookup, no new network/IPC call per click.

**Tech Stack:** Rust (Tauri commands, `tauri-specta` for TS binding generation), React 19 + TypeScript, Vitest + Testing Library for frontend tests.

**Spec:** `docs/superpowers/specs/2026-07-26-trends-day-model-breakdown-design.md`

## Global Constraints

- No per-click IPC round trip — all breakdown data for the window is fetched once, upfront, alongside the existing daily-trends fetch (spec §2, §6).
- No changes to the Models tab's own 30-day aggregate output or its `get_model_breakdown`/`get_cache_stats` commands (spec §2).
- Per-day model rows must be sorted by `cost_usd` descending — `HashMap` iteration order is not stable in Rust, so this requires an explicit sort, not an assumption (spec §4.1).
- No new Rust unit test for the bucketing command — matches the existing convention that neither `get_daily_trends` nor `get_model_breakdown` has test coverage anywhere in `src-tauri/tests/` or inline (spec §7). Frontend behavior is covered by a render test instead.
- One selected day at a time — no multi-day comparison (spec §2).

---

### Task 1: Backend — `get_daily_model_breakdown` command

**Files:**
- Modify: `src-tauri/src/commands.rs` (add struct + command after `get_model_breakdown`, currently ending at line 142)
- Modify: `src-tauri/src/lib.rs` (register in both `collect_commands!` blocks, currently at lines ~165 and ~201, immediately after `commands::get_model_breakdown,`)

**Interfaces:**
- Consumes: `get_session_history(days: u32, state) -> Result<Vec<StoredSessionEvent>, String>` (existing, `commands.rs:76`), `ModelStats` struct (existing, `commands.rs:32-39`).
- Produces: `DailyModelBucket { date: String, models: Vec<ModelStats> }` and `get_daily_model_breakdown(days: u32, state) -> Result<Vec<DailyModelBucket>, String>`, consumed by Task 3.

- [ ] **Step 1: Add the `DailyModelBucket` struct**

In `src-tauri/src/commands.rs`, immediately after the `ModelStats` struct (after line 39, before the `ProjectStats` struct):

```rust
#[derive(Debug, Serialize, Deserialize, specta::Type)]
pub struct DailyModelBucket {
    pub date: String,
    pub models: Vec<ModelStats>,
}
```

- [ ] **Step 2: Add the `get_daily_model_breakdown` command**

Immediately after the `get_model_breakdown` function (after line 142, before `get_project_breakdown`):

```rust
#[command]
#[specta::specta]
pub async fn get_daily_model_breakdown(
    days: u32,
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<DailyModelBucket>, String> {
    let events = get_session_history(days, state).await?;
    use std::collections::{BTreeMap, HashMap};
    let mut by_day: BTreeMap<String, HashMap<String, ModelStats>> = BTreeMap::new();
    for e in events {
        let date = e
            .ts
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string();
        let by_model = by_day.entry(date).or_insert_with(HashMap::new);
        let entry = by_model
            .entry(e.model.clone())
            .or_insert_with(|| ModelStats {
                model: e.model.clone(),
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                cost_usd: 0.0,
            });
        entry.input_tokens += e.input_tokens;
        entry.output_tokens += e.output_tokens;
        entry.cache_read_tokens += e.cache_read_tokens;
        entry.cache_creation_tokens += e.cache_creation_5m_tokens + e.cache_creation_1h_tokens;
        entry.cost_usd += e.cost_usd;
    }
    Ok(by_day
        .into_iter()
        .map(|(date, models)| {
            let mut models: Vec<ModelStats> = models.into_values().collect();
            models.sort_by(|a, b| b.cost_usd.total_cmp(&a.cost_usd));
            DailyModelBucket { date, models }
        })
        .collect())
}
```

This mirrors `get_daily_trends` (date bucketing via `BTreeMap`, giving date-ascending order for free) and `get_model_breakdown` (per-model aggregation fields) simultaneously — the only genuinely new logic is the composite `(date, model)` key and the explicit cost-descending sort per day, which neither sibling function needs to do (see Global Constraints).

- [ ] **Step 3: Register the command in both `collect_commands!` blocks**

In `src-tauri/src/lib.rs`, there are two `#[cfg(...)]`-gated `collect_commands!` blocks (one for release builds, one for debug builds — `tauri-specta`'s `Builder::commands` replaces the previous list rather than appending, so both must be kept in sync manually, same as every other command in this file). In **both** blocks, add the new command directly after the existing `commands::get_model_breakdown,` line:

```rust
            commands::get_model_breakdown,
            commands::get_daily_model_breakdown,
```

- [ ] **Step 4: Verify it compiles**

Run: `cd src-tauri && cargo check`
Expected: compiles cleanly, no errors or warnings about the new code.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs
git commit -m "feat(trends): add get_daily_model_breakdown command"
```

---

### Task 2: Frontend — extract shared model-display helpers

`ModelsTab.tsx` currently defines `MODEL_VARIANT`, `modelKey`, and `shortName` as unexported, closure-free local functions (`ModelsTab.tsx:12-29`). Task 3's day-breakdown panel needs the exact same label logic. Extracting them into their own module avoids duplicating them, and — as a side effect — makes them independently testable for the first time (they currently have no test coverage).

**Files:**
- Create: `src/report/modelDisplay.ts`
- Create: `src/report/modelDisplay.test.ts`
- Modify: `src/report/ModelsTab.tsx:1-29`

**Interfaces:**
- Produces: `MODEL_VARIANT: Record<string, 'opus' | 'sonnet' | 'haiku' | 'default'>`, `modelKey(name: string): string`, `shortName(model: string): string` — all consumed by Task 3.

- [ ] **Step 1: Write the failing test**

Create `src/report/modelDisplay.test.ts`:

```ts
import { describe, it, expect } from 'vitest';
import { modelKey, shortName, MODEL_VARIANT } from './modelDisplay';

describe('modelKey', () => {
  it('classifies opus/sonnet/haiku regardless of case or version suffix', () => {
    expect(modelKey('claude-opus-4-7')).toBe('opus');
    expect(modelKey('Claude-Sonnet-4-6')).toBe('sonnet');
    expect(modelKey('claude-haiku-4-5')).toBe('haiku');
  });

  it('falls back to default for unrecognized models', () => {
    expect(modelKey('glm-5.1')).toBe('default');
  });
});

describe('shortName', () => {
  it('extracts a compact "tier version" label', () => {
    expect(shortName('claude-opus-4-7')).toBe('opus 4-7');
    expect(shortName('claude-sonnet-4-6')).toBe('sonnet 4-6');
  });

  it('returns the raw name when no tier pattern matches', () => {
    expect(shortName('glm-5.1')).toBe('glm-5.1');
  });
});

describe('MODEL_VARIANT', () => {
  it('maps known tiers to their badge variant', () => {
    expect(MODEL_VARIANT.opus).toBe('opus');
    expect(MODEL_VARIANT.sonnet).toBe('sonnet');
    expect(MODEL_VARIANT.haiku).toBe('haiku');
  });
});
```

- [ ] **Step 2: Run it to verify it fails**

Run: `npx vitest run src/report/modelDisplay.test.ts`
Expected: FAIL — `Cannot find module './modelDisplay'` (the file doesn't exist yet).

- [ ] **Step 3: Create `modelDisplay.ts`**

```ts
export const MODEL_VARIANT: Record<string, 'opus' | 'sonnet' | 'haiku' | 'default'> = {
  opus: 'opus',
  sonnet: 'sonnet',
  haiku: 'haiku',
};

export function modelKey(name: string): string {
  const lower = name.toLowerCase();
  if (lower.includes('opus')) return 'opus';
  if (lower.includes('sonnet')) return 'sonnet';
  if (lower.includes('haiku')) return 'haiku';
  return 'default';
}

export function shortName(model: string): string {
  const m = model.match(/(opus|sonnet|haiku)-(\d+(?:-\d+)?)/i);
  return m ? `${m[1]} ${m[2]}` : model;
}
```

This is the exact logic currently at `ModelsTab.tsx:12-29`, moved verbatim and exported.

- [ ] **Step 4: Run the test again to verify it passes**

Run: `npx vitest run src/report/modelDisplay.test.ts`
Expected: PASS (5 tests).

- [ ] **Step 5: Update `ModelsTab.tsx` to import instead of defining locally**

In `src/report/ModelsTab.tsx`, replace lines 12-29 (the `MODEL_VARIANT` const and the `modelKey`/`shortName` functions) with:

```ts
import { MODEL_VARIANT, modelKey, shortName } from './modelDisplay';
```

Place this import alongside the existing imports at the top of the file (after the `useAppStore` import on line 10). Delete the now-duplicate local definitions entirely — do not keep both.

- [ ] **Step 6: Verify nothing broke**

Run: `npx tsc -b`
Expected: no type errors.

Run: `npx vitest run`
Expected: all existing tests still pass (this is a pure refactor — no behavior change).

- [ ] **Step 7: Commit**

```bash
git add src/report/modelDisplay.ts src/report/modelDisplay.test.ts src/report/ModelsTab.tsx
git commit -m "refactor(report): extract model-display helpers for reuse in Trends tab"
```

---

### Task 3: Frontend — bindings, IPC wrapper, and TrendsTab click-to-reveal panel

**Files:**
- Modify: `src/lib/generated/bindings.ts` (hand-edit — see note below)
- Modify: `src/lib/ipc.ts`
- Modify: `src/lib/types.ts`
- Modify: `src/report/TrendsTab.tsx` (full rewrite of the existing 153-line file)
- Create: `src/report/TrendsTab.test.tsx`

**Interfaces:**
- Consumes: `get_daily_model_breakdown` (Task 1), `MODEL_VARIANT`/`modelKey`/`shortName` (Task 2), existing `ipc.getDailyTrends`, `useTabData`, `Card`, `Badge`, `formatTokens`, `formatCost`.

**Note on `bindings.ts`:** This file carries a header "Do not edit this file manually" — normally `tauri-specta` regenerates it automatically the next time the app runs in debug mode (`pnpm tauri dev`), because the export call in `lib.rs` (`specta_builder.export(...)`, gated `#[cfg(debug_assertions)]`) only executes as part of the app's actual runtime startup, not during `cargo check`/`cargo build` alone — there's no headless/scriptable export path in this codebase. Step 1 below hand-writes the exact deterministic output `tauri-specta` would produce (verified against the existing `getDailyTrends`/`getModelBreakdown`/`ModelStats` entries already in the file). If you run `pnpm tauri dev` at any point after Task 1 lands, it will regenerate this file — the result should be byte-identical to this hand-edit; if it differs, trust the regenerated version and diff to see what was missed here.

- [ ] **Step 1: Hand-edit `bindings.ts`**

In `src/lib/generated/bindings.ts`, add the command wrapper immediately after `getModelBreakdown` (after line 47, before `getProjectBreakdown`):

```ts
async getDailyModelBreakdown(days: number) : Promise<Result<DailyModelBucket[], string>> {
    try {
    return { status: "ok", data: await TAURI_INVOKE("get_daily_model_breakdown", { days }) };
} catch (e) {
    if(e instanceof Error) throw e;
    else return { status: "error", error: e  as any };
}
},
```

Add the type immediately after the `ModelStats` type export (line 343):

```ts
export type DailyModelBucket = { date: string; models: ModelStats[] }
```

- [ ] **Step 2: Add the IPC wrapper**

In `src/lib/ipc.ts`, add immediately after the `getModelBreakdown` line:

```ts
  getDailyModelBreakdown: (days: number) => commands.getDailyModelBreakdown(days).then(unwrap),
```

- [ ] **Step 3: Re-export the type**

In `src/lib/types.ts`, add `DailyModelBucket` to the re-export list, immediately after `DailyBucket`:

```ts
  DailyBucket,
  DailyModelBucket,
```

- [ ] **Step 4: Verify the bindings compile**

Run: `npx tsc -b`
Expected: no type errors (confirms the hand-written `bindings.ts`/`ipc.ts`/`types.ts` additions are internally consistent before writing any component code against them).

- [ ] **Step 5: Write the failing test**

Create `src/report/TrendsTab.test.tsx`:

```tsx
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { DailyBucket, DailyModelBucket } from '../lib/types';

const TRENDS: DailyBucket[] = [
  { date: '2026-07-24', input_tokens: 8_000_000, output_tokens: 4_400_000, cost_usd: 4.82 },
  { date: '2026-07-25', input_tokens: 2_000_000, output_tokens: 1_000_000, cost_usd: 1.1 },
];

const BREAKDOWN: DailyModelBucket[] = [
  {
    date: '2026-07-24',
    models: [
      { model: 'claude-sonnet-4-6', input_tokens: 6_000_000, output_tokens: 2_100_000, cache_read_tokens: 6_200_000, cache_creation_tokens: 300_000, cost_usd: 2.94 },
      { model: 'claude-opus-4-7', input_tokens: 1_700_000, output_tokens: 1_700_000, cache_read_tokens: 2_100_000, cache_creation_tokens: 100_000, cost_usd: 1.61 },
      { model: 'claude-haiku-4-5', input_tokens: 300_000, output_tokens: 600_000, cache_read_tokens: 50_000, cache_creation_tokens: 10_000, cost_usd: 0.27 },
    ],
  },
  {
    date: '2026-07-25',
    models: [
      { model: 'claude-sonnet-4-6', input_tokens: 2_000_000, output_tokens: 1_000_000, cache_read_tokens: 500_000, cache_creation_tokens: 20_000, cost_usd: 1.1 },
    ],
  },
];

const ipcMock = vi.hoisted(() => ({
  getDailyTrends: vi.fn(),
  getDailyModelBreakdown: vi.fn(),
}));

vi.mock('../lib/ipc', () => ({ ipc: ipcMock }));

vi.mock('../lib/store', async () => {
  const actual = await vi.importActual<typeof import('../lib/store')>('../lib/store');
  const state = { sessionDataVersion: 0 };
  const useAppStore: any = (sel: any) => sel(state);
  useAppStore.getState = () => state;
  return { ...actual, useAppStore };
});

import { TrendsTab } from './TrendsTab';

describe('TrendsTab — day breakdown panel', () => {
  beforeEach(() => {
    ipcMock.getDailyTrends.mockResolvedValue(TRENDS);
    ipcMock.getDailyModelBreakdown.mockResolvedValue(BREAKDOWN);
  });

  it('reveals the per-model breakdown for a day on click, sorted by cost descending', async () => {
    render(<TrendsTab />);
    const bar = await screen.findByRole('button', { name: /Jul 24/i });
    fireEvent.click(bar);

    const panel = await screen.findByTestId('day-breakdown-panel');
    const badges = within(panel).getAllByText(/^(sonnet|opus|haiku) \d/);
    expect(badges.map((b) => b.textContent)).toEqual(['sonnet 4-6', 'opus 4-7', 'haiku 4-5']);
  });

  it('collapses the panel when the same bar is clicked again', async () => {
    render(<TrendsTab />);
    const bar = await screen.findByRole('button', { name: /Jul 24/i });
    fireEvent.click(bar);
    await screen.findByTestId('day-breakdown-panel');

    fireEvent.click(bar);
    await waitFor(() => expect(screen.queryByTestId('day-breakdown-panel')).not.toBeInTheDocument());
  });

  it('switches the panel to a different day when a different bar is clicked', async () => {
    render(<TrendsTab />);
    const bar24 = await screen.findByRole('button', { name: /Jul 24/i });
    fireEvent.click(bar24);
    let panel = await screen.findByTestId('day-breakdown-panel');
    expect(within(panel).getByText('opus 4-7')).toBeInTheDocument();

    const bar25 = screen.getByRole('button', { name: /Jul 25/i });
    fireEvent.click(bar25);
    await waitFor(() => {
      panel = screen.getByTestId('day-breakdown-panel');
      expect(within(panel).queryByText('opus 4-7')).not.toBeInTheDocument();
    });
    expect(within(panel).getByText('sonnet 4-6')).toBeInTheDocument();
  });

  it('clears the selection when the range toggle changes', async () => {
    render(<TrendsTab />);
    const bar = await screen.findByRole('button', { name: /Jul 24/i });
    fireEvent.click(bar);
    await screen.findByTestId('day-breakdown-panel');

    fireEvent.click(screen.getByText('7d'));
    await waitFor(() => expect(screen.queryByTestId('day-breakdown-panel')).not.toBeInTheDocument());
  });
});
```

- [ ] **Step 6: Run it to verify it fails**

Run: `npx vitest run src/report/TrendsTab.test.tsx`
Expected: FAIL — the current `TrendsTab` renders bars as plain non-interactive `<div>`s with no accessible name and no breakdown panel, so `findByRole('button', { name: /Jul 24/i })` will not find anything.

- [ ] **Step 7: Rewrite `TrendsTab.tsx`**

Replace the full contents of `src/report/TrendsTab.tsx` with:

```tsx
import { useEffect, useMemo, useState } from 'react';
import { Card } from '../components/ui/Card';
import { Badge } from '../components/ui/Badge';
import { Button } from '../components/ui/Button';
import { EmptyState } from '../components/ui/EmptyState';
import { formatTokens, formatCost } from '../lib/format';
import { IconTrends } from '../lib/icons';
import { ipc } from '../lib/ipc';
import { useTabData } from '../lib/useTabData';
import { useAppStore } from '../lib/store';
import { MODEL_VARIANT, modelKey, shortName } from './modelDisplay';

export function TrendsTab() {
  const version = useAppStore((s) => s.sessionDataVersion);
  const { data, error, loading, reload } = useTabData(
    () =>
      Promise.all([ipc.getDailyTrends(30), ipc.getDailyModelBreakdown(30)]).then(
        ([trends, breakdown]) => ({ trends, breakdown }),
      ),
    [version],
  );
  const [range, setRange] = useState<'7d' | '30d'>('30d');
  const [selectedDate, setSelectedDate] = useState<string | null>(null);

  useEffect(() => {
    setSelectedDate(null);
  }, [range]);

  const trends = data?.trends ?? null;
  const breakdown = data?.breakdown ?? null;

  const visibleData = useMemo(() => {
    if (!trends) return [];
    const days = range === '7d' ? 7 : 30;
    return trends.slice(-days);
  }, [trends, range]);

  const selectedDay = useMemo(() => {
    if (!selectedDate) return null;
    return visibleData.find((d) => d.date === selectedDate) ?? null;
  }, [visibleData, selectedDate]);

  const selectedBreakdown = useMemo(() => {
    if (!breakdown || !selectedDate) return null;
    return breakdown.find((b) => b.date === selectedDate) ?? null;
  }, [breakdown, selectedDate]);

  if (error) {
    return (
      <EmptyState
        icon={<IconTrends size={32} />}
        title="Couldn't load trends"
        description={error}
        action={<Button variant="ghost" size="sm" onClick={reload}>Retry</Button>}
      />
    );
  }
  if (loading || !trends || !breakdown) {
    return <p className="text-[color:var(--color-text-muted)]">Loading…</p>;
  }

  if (trends.length === 0) {
    return (
      <EmptyState
        icon={<IconTrends size={32} />}
        title="No trend data"
        description="Trends will appear after a few days of usage."
      />
    );
  }

  const maxValue = Math.max(
    ...visibleData.map((d) => d.input_tokens + d.output_tokens),
    1,
  );
  const chartHeight = 160;

  return (
    <div className="flex flex-col gap-[var(--space-md)]">
      {/* Range selector */}
      <div className="flex gap-[var(--space-2xs)] bg-[var(--color-track)] rounded-[var(--radius-sm)] p-[2px] w-fit">
        {(['7d', '30d'] as const).map((r) => (
          <button
            key={r}
            type="button"
            onClick={() => setRange(r)}
            className={[
              'px-[var(--space-sm)] py-[var(--space-2xs)]',
              'text-[length:var(--text-label)] font-[var(--weight-medium)]',
              'rounded-[var(--radius-sm)]',
              'transition-[background,color] duration-[var(--duration-fast)]',
              range === r
                ? 'bg-[var(--color-bg-card)] text-[color:var(--color-text)]'
                : 'text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text-secondary)]',
            ].join(' ')}
          >
            {r}
          </button>
        ))}
      </div>

      {/* Chart */}
      <Card className="p-[var(--space-md)]">
        <div className="flex items-end gap-[2px]" style={{ height: chartHeight }}>
          {visibleData.map((day) => {
            const total = day.input_tokens + day.output_tokens;
            const heightPct = (total / maxValue) * 100;
            const isDanger = day.cost_usd >= 3;
            const isWarn = day.cost_usd >= 1.5 && !isDanger;
            const isSelected = day.date === selectedDate;
            const label = `${new Date(day.date).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}: ${formatTokens(total)} tokens, ${formatCost(day.cost_usd)}`;

            return (
              <div
                key={day.date}
                className="flex-1 flex flex-col justify-end group relative"
                style={{ height: '100%' }}
              >
                <button
                  type="button"
                  aria-label={label}
                  aria-pressed={isSelected}
                  onClick={() => setSelectedDate((d) => (d === day.date ? null : day.date))}
                  className={[
                    'w-full rounded-t-[2px] transition-[height,background-color] duration-[var(--duration-normal)]',
                    isDanger
                      ? 'bg-[var(--color-danger)]'
                      : isWarn
                        ? 'bg-[var(--color-warn)]'
                        : 'bg-[var(--color-accent)]',
                    isSelected
                      ? 'opacity-100 ring-2 ring-[var(--color-border-focus)]'
                      : 'opacity-80 group-hover:opacity-100',
                  ].join(' ')}
                  style={{ height: `${heightPct}%` }}
                />
                {/* Tooltip */}
                <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-[var(--space-xs)] hidden group-hover:block z-10">
                  <div className="bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded-[var(--radius-sm)] px-[var(--space-sm)] py-[var(--space-xs)] whitespace-nowrap">
                    <div className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                      {new Date(day.date).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}
                    </div>
                    <div className="mono text-[length:var(--text-label)] text-[color:var(--color-text)]">
                      {formatTokens(total)}
                    </div>
                    <div className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                      ${day.cost_usd.toFixed(2)}
                    </div>
                  </div>
                </div>
              </div>
            );
          })}
        </div>

        {/* X-axis labels */}
        <div className="flex mt-[var(--space-xs)]">
          {visibleData.map((day, i) => (
            <span
              key={day.date}
              className="flex-1 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] mono"
            >
              {i % (range === '7d' ? 1 : 5) === 0
                ? new Date(day.date).toLocaleDateString('en-US', { day: 'numeric' })
                : null}
            </span>
          ))}
        </div>
      </Card>

      {/* Day breakdown panel */}
      {selectedDay && selectedBreakdown && (
        <Card data-testid="day-breakdown-panel" className="p-[var(--space-md)] flex flex-col gap-[var(--space-sm)]">
          <div className="flex items-center justify-between">
            <span className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text)]">
              {new Date(selectedDay.date).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}
            </span>
            <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)]">
              {formatTokens(selectedDay.input_tokens + selectedDay.output_tokens)} · {formatCost(selectedDay.cost_usd)}
            </span>
          </div>
          <div className="flex flex-col gap-[var(--space-xs)]">
            {selectedBreakdown.models.map((m) => {
              const dayTotal = selectedDay.input_tokens + selectedDay.output_tokens;
              const modelTotal = m.input_tokens + m.output_tokens;
              const pct = dayTotal > 0 ? (modelTotal / dayTotal) * 100 : 0;
              const key = modelKey(m.model);

              return (
                <div key={m.model} className="flex flex-col gap-[var(--space-2xs)]">
                  <div className="flex items-center gap-[var(--space-sm)]">
                    <Badge variant={MODEL_VARIANT[key] ?? 'default'}>{shortName(m.model)}</Badge>
                    <div className="flex-1">
                      <div className="w-full h-[6px] rounded-[var(--radius-pill)] bg-[var(--color-track)] overflow-hidden">
                        <div
                          className="h-full rounded-[var(--radius-pill)] transition-[width] duration-[var(--duration-bar)] ease-[var(--ease-spring)]"
                          style={{
                            width: `${pct}%`,
                            background:
                              key === 'opus'
                                ? 'var(--color-model-opus)'
                                : key === 'sonnet'
                                  ? 'var(--color-model-sonnet)'
                                  : 'var(--color-model-haiku)',
                          }}
                        />
                      </div>
                    </div>
                    <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-muted)] tabular-nums min-w-[48px] text-right">
                      {formatTokens(modelTotal)}
                    </span>
                    <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)] tabular-nums min-w-[48px] text-right">
                      {formatCost(m.cost_usd)}
                    </span>
                  </div>
                  <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] pl-[calc(var(--space-sm)+7px)]">
                    cache: {formatTokens(m.cache_read_tokens)} read · {formatTokens(m.cache_creation_tokens)} created
                  </span>
                </div>
              );
            })}
          </div>
        </Card>
      )}

      {/* Summary */}
      <div className="flex items-center gap-[var(--space-md)] px-[var(--space-2xs)]">
        <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)]">
          Avg {formatTokens(visibleData.reduce((s, d) => s + d.input_tokens + d.output_tokens, 0) / visibleData.length)}
        </span>
        <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">·</span>
        <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)]">
          ${visibleData.reduce((s, d) => s + d.cost_usd, 0).toFixed(2)} total
        </span>
      </div>
    </div>
  );
}
```

Key changes from the original: the bar element is now a real `<button>` (matching the range-selector buttons already in this file) with an `aria-label` carrying the same info the hover tooltip shows, so it's both clickable and accessible; `selectedDate` state and the `useEffect` that clears it on range change; the day-breakdown `Card` rendered conditionally between the chart and the summary row.

- [ ] **Step 8: Run the test again to verify it passes**

Run: `npx vitest run src/report/TrendsTab.test.tsx`
Expected: PASS (4 tests).

- [ ] **Step 9: Verify nothing else broke**

Run: `npx tsc -b`
Expected: no type errors.

Run: `npx vitest run`
Expected: all tests pass, including the untouched `SessionsTab.test.ts` and `modelDisplay.test.ts` from Task 2.

- [ ] **Step 10: Commit**

```bash
git add src/lib/generated/bindings.ts src/lib/ipc.ts src/lib/types.ts src/report/TrendsTab.tsx src/report/TrendsTab.test.tsx
git commit -m "feat(trends): click a day to reveal its per-model breakdown"
```

---

### Task 4: Full-suite verification and manual check

**Files:** none (verification only).

- [ ] **Step 1: Run the full Rust test suite**

Run: `cd src-tauri && cargo test`
Expected: all tests pass (181, unchanged from before this work — no new Rust tests added per the Global Constraints note).

- [ ] **Step 2: Run the full frontend test suite**

Run: `npx vitest run`
Expected: all tests pass, including the new `modelDisplay.test.ts` (5 tests) and `TrendsTab.test.tsx` (4 tests).

- [ ] **Step 3: Full typecheck**

Run: `npx tsc -b`
Expected: no errors.

- [ ] **Step 4: Manual check (not scriptable — do this yourself before shipping)**

Run `pnpm tauri dev`, open the Trends tab, click a day's bar, confirm the breakdown panel appears with sensible numbers, click it again to confirm it collapses, and click a different day to confirm it switches. This also regenerates `src/lib/generated/bindings.ts` for real — diff it against the hand-edit from Task 3 Step 1; they should match exactly. Type-checking and the test suite verify code correctness, not that the feature actually looks and feels right — this step is the only one that does.

- [ ] **Step 5: Commit (if the manual check surfaced any fixes)**

If Step 4 required changes, commit them separately with a message describing what was off. If nothing needed fixing, there's nothing to commit here — Task 3's commit already covers the shipped behavior.
