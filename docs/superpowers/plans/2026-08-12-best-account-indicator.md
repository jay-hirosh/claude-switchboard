# Best-Account Indicator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Flag the managed account with the most remaining rate-limit headroom with a small "Best available" pill on its row in the Accounts panel and the expanded report's sidebar.

**Architecture:** A pure ranking function (`computeBestAccountUuid`) picks the account whose worst bucket (`max(5H%, 7D%)`) is lowest, with suppression rules so the pill only appears when switching would meaningfully help. `useAccountManagement` memoizes it; both existing account-list surfaces pass a boolean down to `AccountRow`, which renders the pill. Zero backend/IPC changes — everything computes from `cached_usage` already present on each `AccountListEntry`.

**Tech Stack:** React 19 + TypeScript, Vitest + @testing-library/react, Tailwind v4 design tokens.

**Spec:** `docs/superpowers/specs/2026-08-12-best-account-indicator-design.md`

## Global Constraints

- No hard-coded colors/spacing — design tokens only. The pill uses `--color-teal` / `--color-teal-dim` (NOT `--color-accent`, which already means "MAX tier" / "active row").
- Pill copy is exactly `Best available`.
- Margin constant is 3 percentage points, not user-configurable.
- No `src-tauri/` changes, no new IPC commands, no changes to the compact popover.
- Package manager is `pnpm`. Typecheck: `pnpm lint`. Tests: `pnpm test` (or `pnpm exec vitest run <file>` for one file).

---

### Task 1: Ranking function `computeBestAccountUuid`

**Files:**
- Create: `src/accounts/bestAccount.ts`
- Test: `src/accounts/bestAccount.test.ts` (co-located plain `.ts`, matching `src/report/SessionsTab.test.ts`'s convention for pure-function tests — NOT the `__tests__/` folder, which this directory reserves for component tests)

**Interfaces:**
- Consumes: `AccountListEntry` from `src/lib/generated/bindings` (fields used: `account_uuid: string`, `is_active: boolean`, `last_error: string | null`, `cached_usage: CachedUsage | null` with `cached_usage.snapshot.five_hour` / `.seven_day` of type `{ utilization: number; resets_at?: string | null } | null`).
- Produces: `computeBestAccountUuid(accounts: AccountListEntry[], marginPct = 3): string | null` — returns the `account_uuid` of the account to flag, or `null` when nothing should be flagged. Task 3 calls this.

- [ ] **Step 1: Write the failing tests**

Create `src/accounts/bestAccount.test.ts`:

```ts
import { describe, it, expect } from 'vitest';
import type { AccountListEntry, CachedUsage } from '../lib/generated/bindings';
import { computeBestAccountUuid } from './bestAccount';

function cached(fiveHour: number | null, sevenDay: number | null): CachedUsage {
  return {
    snapshot: {
      five_hour: fiveHour === null ? null : { utilization: fiveHour, resets_at: null },
      seven_day: sevenDay === null ? null : { utilization: sevenDay, resets_at: null },
      seven_day_sonnet: null,
      seven_day_opus: null,
      extra_usage: null,
    },
    account_id: 'x',
    account_email: 'x@x.com',
    last_error: null,
    burn_rate: null,
    auth_source: 'OAuth',
  } as CachedUsage;
}

function acct(
  uuid: string,
  opts: {
    active?: boolean;
    fiveHour?: number;
    sevenDay?: number;
    lastError?: string;
    unpolled?: boolean;
  } = {},
): AccountListEntry {
  return {
    slot: 1,
    email: `${uuid}@x.com`,
    account_uuid: uuid,
    org_name: null,
    org_uuid: null,
    subscription_type: 'pro',
    source: 'OAuth',
    is_active: opts.active ?? false,
    cached_usage: opts.unpolled
      ? null
      : cached(opts.fiveHour ?? 0, opts.sevenDay ?? 0),
    last_error: opts.lastError ?? null,
  } as AccountListEntry;
}

describe('computeBestAccountUuid', () => {
  it('flags the account whose WORST bucket is lowest, not the lowest 5H', () => {
    // A (active): constraint 70. B: 5h is 20 but 7d 55 governs → 55.
    // C: lowest 5h overall (10) but 7d 65 governs → 65. Best is B.
    const accounts = [
      acct('A', { active: true, fiveHour: 70, sevenDay: 40 }),
      acct('B', { fiveHour: 20, sevenDay: 55 }),
      acct('C', { fiveHour: 10, sevenDay: 65 }),
    ];
    expect(computeBestAccountUuid(accounts)).toBe('B');
  });

  it('returns null when the active account is already best', () => {
    const accounts = [
      acct('A', { active: true, fiveHour: 10, sevenDay: 10 }),
      acct('B', { fiveHour: 50, sevenDay: 50 }),
    ];
    expect(computeBestAccountUuid(accounts)).toBeNull();
  });

  it('suppresses leads smaller than the margin, including exact ties', () => {
    const twoPointLead = [
      acct('A', { active: true, fiveHour: 50, sevenDay: 50 }),
      acct('B', { fiveHour: 48, sevenDay: 48 }),
    ];
    expect(computeBestAccountUuid(twoPointLead)).toBeNull();

    const tie = [
      acct('A', { active: true, fiveHour: 50, sevenDay: 50 }),
      acct('B', { fiveHour: 50, sevenDay: 50 }),
    ];
    expect(computeBestAccountUuid(tie)).toBeNull();
  });

  it('flags leads at or above the margin', () => {
    const accounts = [
      acct('A', { active: true, fiveHour: 50, sevenDay: 50 }),
      acct('B', { fiveHour: 47, sevenDay: 47 }),
    ];
    expect(computeBestAccountUuid(accounts)).toBe('B');
  });

  it('excludes errored accounts even when their numbers look best', () => {
    const accounts = [
      acct('A', { active: true, fiveHour: 60, sevenDay: 60 }),
      acct('B', { fiveHour: 5, sevenDay: 5, lastError: 'rate-limited (429)' }),
      acct('C', { fiveHour: 30, sevenDay: 30 }),
    ];
    expect(computeBestAccountUuid(accounts)).toBe('C');
  });

  it('excludes never-polled accounts (no cached_usage)', () => {
    const accounts = [
      acct('A', { active: true, fiveHour: 60, sevenDay: 60 }),
      acct('B', { unpolled: true }),
      acct('C', { fiveHour: 30, sevenDay: 30 }),
    ];
    expect(computeBestAccountUuid(accounts)).toBe('C');
  });

  it('skips the margin check when the active account itself is errored', () => {
    // Active is ineligible → no reliable baseline → best eligible wins
    // regardless of how its numbers compare to active's stale ones.
    const accounts = [
      acct('A', { active: true, fiveHour: 10, sevenDay: 10, lastError: 'auth_required' }),
      acct('B', { fiveHour: 90, sevenDay: 90 }),
    ];
    expect(computeBestAccountUuid(accounts)).toBe('B');
  });

  it('returns null with fewer than two managed accounts', () => {
    expect(computeBestAccountUuid([acct('A', { active: true })])).toBeNull();
    expect(computeBestAccountUuid([])).toBeNull();
  });

  it('returns null when every account is ineligible', () => {
    const accounts = [
      acct('A', { active: true, lastError: 'auth_required' }),
      acct('B', { unpolled: true }),
    ];
    expect(computeBestAccountUuid(accounts)).toBeNull();
  });

  it('treats a missing bucket as 0 for that bucket', () => {
    // B reports only 7d (5h null → 0); constraint is 20.
    const accounts = [
      acct('A', { active: true, fiveHour: 60, sevenDay: 60 }),
      { ...acct('B'), cached_usage: cached(null, 20) },
    ];
    expect(computeBestAccountUuid(accounts)).toBe('B');
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm exec vitest run src/accounts/bestAccount.test.ts`
Expected: FAIL — cannot resolve `./bestAccount` (module does not exist yet).

- [ ] **Step 3: Write the implementation**

Create `src/accounts/bestAccount.ts`:

```ts
import type { AccountListEntry } from '../lib/generated/bindings';

/**
 * Picks the managed account with the most remaining headroom, or null when
 * no account is worth flagging.
 *
 * "Headroom" is judged by the WORST of the two buckets
 * (max of 5H% and 7D%) — whichever bucket is closer to blocking you is the
 * one that matters for "can I use this account right now."
 *
 * Suppression rules (all return null):
 *  - fewer than two managed accounts;
 *  - no eligible account (eligible = polled at least once and not errored);
 *  - the best account is already the active one;
 *  - the best account's lead over the active one is under `marginPct`
 *    points — unless the active account is itself ineligible (errored),
 *    in which case there is no trustworthy baseline and the best eligible
 *    account is flagged regardless of margin.
 */
export function computeBestAccountUuid(
  accounts: AccountListEntry[],
  marginPct = 3,
): string | null {
  if (accounts.length < 2) return null;

  const constraint = (a: AccountListEntry) => {
    const s = a.cached_usage?.snapshot;
    return Math.max(s?.five_hour?.utilization ?? 0, s?.seven_day?.utilization ?? 0);
  };
  const eligible = accounts.filter((a) => !a.last_error && a.cached_usage);
  if (eligible.length === 0) return null;

  const best = eligible.reduce((a, b) => (constraint(a) <= constraint(b) ? a : b));
  if (best.is_active) return null;

  const active = accounts.find((a) => a.is_active);
  const activeEligible =
    active && eligible.find((a) => a.account_uuid === active.account_uuid);
  if (activeEligible && constraint(activeEligible) - constraint(best) < marginPct) {
    return null;
  }

  return best.account_uuid;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm exec vitest run src/accounts/bestAccount.test.ts`
Expected: PASS (10 tests).

- [ ] **Step 5: Commit**

```bash
git add src/accounts/bestAccount.ts src/accounts/bestAccount.test.ts
git commit -m "feat(accounts): add best-account ranking function"
```

---

### Task 2: `AccountRow` renders the pill

**Files:**
- Modify: `src/accounts/AccountRow.tsx` (Props interface ~line 13-23; badge cluster in the header row ~line 296-320)
- Test: `src/accounts/__tests__/AccountRow.test.tsx` (append a new `describe` block; the file's existing `entry()` factory at ~line 42 is reused as-is)

**Interfaces:**
- Consumes: nothing from Task 1 — the row is presentation-only.
- Produces: `AccountRow` accepts a new optional prop `isBest?: boolean` (default `false`). Task 3 passes it from both call sites.

- [ ] **Step 1: Write the failing tests**

Append to `src/accounts/__tests__/AccountRow.test.tsx` (top-level, after the existing `describe` blocks):

```tsx
describe('AccountRow best-available badge', () => {
  it('renders the pill when isBest is set', () => {
    render(
      <AccountRow
        entry={entry({ is_active: false })}
        thresholds={[75, 90]}
        isBest
      />,
    );
    expect(screen.getByText('Best available')).toBeTruthy();
  });

  it('renders no pill when isBest is omitted', () => {
    render(<AccountRow entry={entry({ is_active: false })} thresholds={[75, 90]} />);
    expect(screen.queryByText('Best available')).toBeNull();
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm exec vitest run src/accounts/__tests__/AccountRow.test.tsx`
Expected: the two new tests FAIL ("Unable to find an element with the text: Best available" for the first; the second may pass trivially — that's fine, the first must fail). Pre-existing tests still PASS.

- [ ] **Step 3: Implement the prop and badge**

In `src/accounts/AccountRow.tsx`:

3a. Add to the `Props` interface, directly after `shareHint?: string | null;`:

```ts
  /** Flags this row as the best account to switch to (most headroom). */
  isBest?: boolean;
```

3b. Add a `BestBadge` component directly after the existing `PlanBadge` function (~line 66). Teal, not accent — accent already means "MAX tier" and "active row" in this same row:

```tsx
function BestBadge() {
  return (
    <span
      className="
        shrink-0 inline-flex items-center rounded-[var(--radius-pill)]
        bg-[var(--color-teal-dim)] px-[var(--space-xs)] py-[1px]
        text-[length:var(--text-micro)] font-[var(--weight-semibold)]
        uppercase tracking-[var(--tracking-label)] text-[color:var(--color-teal)]
      "
    >
      Best available
    </span>
  );
}
```

3c. Destructure the prop in the `AccountRow` signature, after `shareHint`:

```ts
  isBest = false,
```

3d. Render it in the header row, immediately BEFORE `<PlanBadge plan={entry.subscription_type ?? null} />` (~line 307), so the order reads email · Best available · plan · Active/Switch:

```tsx
        {isBest && <BestBadge />}
```

(No `!entry.is_active` guard — the ranking function never flags the active account; the row stays presentation-only.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm exec vitest run src/accounts/__tests__/AccountRow.test.tsx`
Expected: PASS, including both new tests.

- [ ] **Step 5: Commit**

```bash
git add src/accounts/AccountRow.tsx src/accounts/__tests__/AccountRow.test.tsx
git commit -m "feat(accounts): AccountRow renders a Best available pill via isBest prop"
```

---

### Task 3: Wire ranking into both account-list surfaces

**Files:**
- Modify: `src/accounts/useAccountManagement.ts` (imports ~line 1-9; a `useMemo` after `currentActive` ~line 66; the return object ~line 151-170)
- Modify: `src/accounts/AccountsPanel.tsx` (destructure ~line 21-39; `<AccountRow …>` call site ~line 84-95)
- Modify: `src/accounts/AccountsSidebar.tsx` (destructure near top; `<AccountRow …>` call site ~line 74-85)

**Interfaces:**
- Consumes: `computeBestAccountUuid(accounts)` from Task 1; `isBest` prop on `AccountRow` from Task 2.
- Produces: `useAccountManagement()` return object gains `bestAccountUuid: string | null`. Both panels pass `isBest={a.account_uuid === bestAccountUuid}`.

- [ ] **Step 1: Compute in the hook**

In `src/accounts/useAccountManagement.ts`:

1a. Add the import after the existing `ipc` import:

```ts
import { computeBestAccountUuid } from './bestAccount';
```

1b. After the `currentActive` memo, add (`useMemo` is already imported):

```ts
  const bestAccountUuid = useMemo(() => computeBestAccountUuid(accounts), [accounts]);
```

1c. Add `bestAccountUuid,` to the returned object, after `currentActive,`.

- [ ] **Step 2: Pass it in `AccountsPanel.tsx`**

2a. Add `bestAccountUuid,` to the destructuring of `useAccountManagement()` (after `currentActive,`).

2b. In the `accounts.map` render, add to the `<AccountRow>` props, after `shareHint={shareHint}`:

```tsx
              isBest={a.account_uuid === bestAccountUuid}
```

- [ ] **Step 3: Pass it in `AccountsSidebar.tsx`**

Same two changes: add `bestAccountUuid,` to the hook destructuring, and add `isBest={a.account_uuid === bestAccountUuid}` to its `<AccountRow>` call site after `shareHint={shareHint}`.

- [ ] **Step 4: Typecheck and run the full suite**

Run: `pnpm lint`
Expected: clean (no TypeScript errors).

Run: `pnpm test`
Expected: all tests PASS (241 pre-existing + 12 new from Tasks 1–2).

- [ ] **Step 5: Commit**

```bash
git add src/accounts/useAccountManagement.ts src/accounts/AccountsPanel.tsx src/accounts/AccountsSidebar.tsx
git commit -m "feat(accounts): surface Best available pill in accounts panel and sidebar"
```

---

## Verification checklist (after all tasks)

- `pnpm lint` clean, `pnpm test` fully green.
- Manual: with 2+ accounts at meaningfully different utilizations, the non-active account with the most headroom shows a teal "Best available" pill in BOTH the popover's Accounts panel and the expanded report's sidebar; the pill never appears on the active row, on errored rows, or when only one account is managed.
- Compact popover unchanged. No `src-tauri/` diffs.
