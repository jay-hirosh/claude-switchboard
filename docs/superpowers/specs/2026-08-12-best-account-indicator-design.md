# Best-account indicator

**Status:** Design ready for review
**Date:** 2026-08-12
**Tracking PR:** TBD

## 1. Problem

When managing multiple Claude accounts, deciding which one to switch to means manually scanning each account row's 5H/7D percentage bars and comparing them across rows. There's no at-a-glance signal for "which account currently has the most headroom."

This descoped from a broader "smart account switching" idea that originally paired this ranking with a global keyboard shortcut to auto-swap. The hotkey half was cut — this spec covers only the passive indicator.

## 2. Goals / non-goals

**Goals**
- Highlight, on the account list (Accounts panel and the expanded report's Accounts sidebar), which managed account currently has the most runway.
- Zero new backend/IPC — computed entirely from `AccountListEntry.cached_usage`, which is already polled and present client-side for every row.

**Non-goals**
- No keyboard shortcut or automatic swap.
- No change to the compact popover — it only ever reflects the active account, and cross-account ranking doesn't belong in a "two bars, two timers" glance view.
- No user-configurable ranking rules (margin, which buckets count) — fixed constants, to keep this a lightweight indicator rather than a settings surface.

## 3. Ranking logic

Pure function, no React/IPC dependency:

```ts
// src/accounts/bestAccount.ts
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
  const activeEligible = active && eligible.find((a) => a.account_uuid === active.account_uuid);
  if (activeEligible && constraint(activeEligible) - constraint(best) < marginPct) return null;

  return best.account_uuid;
}
```

- **Constraint = `max(5H%, 7D%)`**, not 5H alone — whichever bucket is closer to blocking you is the one that actually matters for "can I use this account right now."
- **Eligibility excludes errored/unpolled accounts** — an account with `last_error` set is excluded even if it still shows usable last-known-good numbers (per `AccountRow`'s own stale-data fallback): a currently-failing poll (rate-limited, network error, expired auth) is exactly the kind of account this feature shouldn't steer someone toward. An account with no `cached_usage` yet (never successfully polled) is excluded for the same reason — no reliable current read.
- **Suppressed when the active account is already best**, or within `marginPct` (3 points) of best — avoids nagging over noise-level differences.
- **Margin check is skipped if the active account itself is ineligible** (errored) — there's no reliable baseline to compare against, so the best available account is flagged regardless of how close the numbers are.

## 4. Architecture

- **New file `src/accounts/bestAccount.ts`** — the function above. Pure, independently testable.
- **`useAccountManagement.ts`** — add `const bestAccountUuid = useMemo(() => computeBestAccountUuid(accounts), [accounts]);`, returned alongside the existing `accounts`, `currentActive`, etc.
- **`AccountRow.tsx`** — new optional prop `isBest?: boolean`. When true, render a small `BestBadge` pill next to the existing `PlanBadge`, styled with `--color-teal` / `--color-teal-dim` — deliberately not `--color-accent` (terracotta), which already means "MAX tier" and "active row" in this same row. Teal is otherwise only used for warm-up affordances elsewhere in the row, and the two never render in the same visual position, so there's no clash.
- **`AccountsPanel.tsx` and `AccountsSidebar.tsx`** — both already destructure `useAccountManagement()` and both already map `accounts` into `<AccountRow entry={a} ... />`. Add `isBest={a.account_uuid === bestAccountUuid}` to both call sites. The ranking logic itself lives in one place; both consumers just wire the boolean through.

No backend changes. No new IPC command. No DB migration.

## 5. Edge cases

- **Exact tie** — `constraint(active) - constraint(best) === 0`, which is `< marginPct`, so suppressed. Matches "not worth flagging."
- **Active account errored** — active is not `eligible`, so the margin comparison is skipped and the best eligible account is flagged even for a small lead, since there's no baseline to weigh it against.
- **All accounts errored** — `eligible.length === 0` → `null`, nothing rendered.
- **Only one managed account** — `accounts.length < 2` → `null`, no comparison possible.

## 6. Testing

- `src/accounts/bestAccount.test.ts` (plain `.ts`, matching `SessionsTab.test.ts`'s convention for pure-function tests, not the `__tests__/` subfolder used for component tests in this directory): tie/margin suppression, errored-active-account override, single-account, all-errored, basic happy path.
- `src/accounts/__tests__/AccountRow.test.tsx` — one additional case: `isBest` renders the pill; omitted/`false` renders nothing.

## 7. Open questions

None blocking.

## 8. File-level checklist

New:
- `src/accounts/bestAccount.ts`
- `src/accounts/bestAccount.test.ts`

Modified:
- `src/accounts/useAccountManagement.ts` — compute and return `bestAccountUuid`
- `src/accounts/AccountRow.tsx` — `isBest` prop + `BestBadge` pill
- `src/accounts/AccountsPanel.tsx` — pass `isBest` to `AccountRow`
- `src/accounts/AccountsSidebar.tsx` — pass `isBest` to `AccountRow`
- `src/accounts/__tests__/AccountRow.test.tsx` — new case for `isBest`

Unchanged: all `src-tauri/` backend files, `src/lib/ipc.ts`, the compact popover, all other report tabs.
