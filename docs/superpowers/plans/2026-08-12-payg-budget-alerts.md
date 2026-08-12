# PAYG Budget Alerts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Real dollars on the pay-as-you-go row (`$31.20 of $100.00`), a `→ ~$52 by reset` burn-rate forecast, and an independent PAYG notification threshold (single slider, default 85%).

**Architecture:** Mirror the existing 5-hour burn-rate machinery for PAYG dollars (in-memory per-slot sample buffer in the poll loop → `ExtraBurnRate` on `CachedUsage`), split the `ExtraUsage` bucket out of the shared threshold loop in the notifier onto its own `Settings.payg_threshold`, and surface both in `ExtraRow`/`SettingsPanel`.

**Tech Stack:** Rust (Tauri v2 backend), React 19 + TypeScript, Vitest, cargo test.

**Spec:** `docs/superpowers/specs/2026-08-12-payg-budget-alerts-design.md`

## Global Constraints

- `payg_threshold` default is **85**; validation `<= 100`; Settings slider rails min 25 / max 95 / step 5.
- Forecast caption text format: `→ ~$52 by reset` (whole dollars, `~` prefix). Spent line format: `$31.20 of $100.00` (two decimals).
- New notification body when limit known: `Used $42.50 of $50.00`; fallback body unchanged (`Pay-as-you-go credits running low`).
- No DB migration (settings serialize as one JSON blob; struct-level `#[serde(default)]` + custom `Default` handles old blobs). No new IPC commands.
- `src/lib/generated/bindings.ts` is generated-but-committed: hand-edit in the file's exact existing style; `pnpm lint` must pass.
- Design tokens only in UI. Package manager pnpm. Rust tests: `cd src-tauri && cargo test`. TS: `pnpm lint`, `pnpm test`.
- Known pre-existing baseline failure (NOT yours): `src/lib/__tests__/theme.test.ts` fails at collection (localStorage error). Bar = no new failures.

---

### Task 1: Independent PAYG threshold (backend, end-to-end)

**Files:**
- Modify: `src-tauri/src/app_state.rs` (Settings struct ~line 28-44, Default impl ~line 46-59)
- Modify: `src-tauri/src/commands.rs` (`update_settings` validation ~line 513-531)
- Modify: `src-tauri/src/notifier/rules.rs` (`evaluate` ~line 83-136, body match ~line 120-125, tests module)
- Modify: `src-tauri/src/poll_loop.rs` (evaluate call site ~line 276-283)

**Interfaces:**
- Consumes: existing `Settings`, `evaluate(db, account_id, snapshot, thresholds, now)`.
- Produces: `Settings.payg_threshold: u8` (default 85); new signature `evaluate(db, account_id, snapshot, thresholds: &[u8], payg_threshold: u8, now)`. Task 4's slider binds to the settings field.

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/notifier/rules.rs`, add to the existing `mod tests` (reuse the existing `fresh()` helper; model fixtures on the existing `snap_five_hour`):

```rust
    fn snap_extra(util: f64, used_cents: u64, limit_cents: u64) -> UsageSnapshot {
        UsageSnapshot {
            five_hour: None,
            seven_day: None,
            seven_day_sonnet: None,
            seven_day_opus: None,
            extra_usage: Some(ExtraUsage {
                is_enabled: true,
                monthly_limit_cents: limit_cents,
                used_credits_cents: used_cents,
                utilization: Some(util),
                resets_at: None,
            }),
            fetched_at: Utc::now(),
            unknown: Default::default(),
        }
    }

    #[test]
    fn payg_uses_its_own_threshold_not_the_shared_ones() {
        let (_d, db) = fresh();
        // Shared thresholds would fire at 75; payg_threshold of 85 must not.
        let s = snap_extra(80.0, 4000, 5000);
        let fired = evaluate(&db, "a", &s, &[75, 90], 85, Utc::now()).unwrap();
        assert!(fired.is_empty(), "80% is below the 85% PAYG threshold");
        // At 86% it fires exactly once, on the PAYG threshold.
        let s2 = snap_extra(86.0, 4300, 5000);
        let fired2 = evaluate(&db, "a", &s2, &[75, 90], 85, Utc::now()).unwrap();
        assert_eq!(fired2.len(), 1);
        assert_eq!(fired2[0].threshold, 85);
        assert!(matches!(fired2[0].bucket, Bucket::ExtraUsage));
    }

    #[test]
    fn shared_buckets_ignore_the_payg_threshold() {
        let (_d, db) = fresh();
        // 5h at 87%: fires for shared 75 (and not 90). payg_threshold=50
        // must not add an extra firing on the five-hour bucket.
        let s = snap_five_hour(87.0, 3);
        let fired = evaluate(&db, "a", &s, &[75, 90], 50, Utc::now()).unwrap();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].threshold, 75);
    }

    #[test]
    fn payg_body_shows_dollars_when_limit_known() {
        let (_d, db) = fresh();
        let s = snap_extra(90.0, 4250, 5000);
        let fired = evaluate(&db, "a", &s, &[], 85, Utc::now()).unwrap();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].body, "Used $42.50 of $50.00");
    }

    #[test]
    fn payg_body_falls_back_without_limit() {
        let (_d, db) = fresh();
        let s = snap_extra(90.0, 0, 0);
        let fired = evaluate(&db, "a", &s, &[], 85, Utc::now()).unwrap();
        assert_eq!(fired.len(), 1);
        assert!(fired[0].body.contains("credits"));
    }
```

Also update every existing `evaluate(...)` call in the tests module to pass a `payg_threshold` argument of `75` before `now` (the existing `extra_usage_without_reset_uses_24h_cooldown` test relies on PAYG firing at 75 — keep its behavior identical by passing 75 as the PAYG threshold; check whether its fixture sets a nonzero `monthly_limit_cents` — if it does, its `body.contains("credits")` assertion must change to match the new dollar body).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test notifier`
Expected: compile error — `evaluate` has no `payg_threshold` parameter yet.

- [ ] **Step 3: Implement**

3a. `src-tauri/src/app_state.rs` — add to `Settings` after `thresholds`:

```rust
    /// Utilization % at which the pay-as-you-go bucket notifies. PAYG has a
    /// single threshold, separate from the shared `thresholds` used by the
    /// rate-limit buckets. `#[serde(default)]` at struct level + the custom
    /// Default keep old settings blobs readable.
    #[serde(default = "default_payg_threshold")]
    pub payg_threshold: u8,
```

Add beside the struct (module scope): `fn default_payg_threshold() -> u8 { 85 }` — a field-level default is required here because the struct-level `#[serde(default)]` only applies when the *whole* blob is absent a field-by-field; verify by reading how `terminal` handles it (field-level `#[serde(default)]`), and set `payg_threshold: 85,` in the custom `Default` impl.

3b. `src-tauri/src/commands.rs` — in `update_settings`, after the thresholds check:

```rust
    if s.payg_threshold > 100 {
        return Err("payg_threshold must be between 0 and 100".to_string());
    }
```

3c. `src-tauri/src/notifier/rules.rs` — change `evaluate` signature:

```rust
pub fn evaluate(
    db: &Db,
    account_id: &str,
    snapshot: &UsageSnapshot,
    thresholds: &[u8],
    payg_threshold: u8,
    now: DateTime<Utc>,
) -> Result<Vec<Fired>> {
```

Inside the bucket loop, select the threshold set per bucket (replacing the direct `for &threshold in thresholds`):

```rust
        let bucket_thresholds: &[u8] = match bucket {
            Bucket::ExtraUsage => std::slice::from_ref(&payg_threshold),
            _ => thresholds,
        };
        for &threshold in bucket_thresholds {
```

And upgrade the `(Bucket::ExtraUsage, None)` body arm — dollars when the snapshot carries a limit:

```rust
            let body = match (bucket, resets_at) {
                (Bucket::ExtraUsage, _) => match snapshot.extra_usage.as_ref() {
                    Some(e) if e.monthly_limit_cents > 0 => format!(
                        "Used ${:.2} of ${:.2}",
                        e.used_credits_cents as f64 / 100.0,
                        e.monthly_limit_cents as f64 / 100.0
                    ),
                    _ => "Pay-as-you-go credits running low".to_string(),
                },
                (_, Some(reset)) => format!("Resets in {}", humanize_duration(reset - now)),
                (_, None) => "Window reset time unknown".to_string(),
            };
```

3d. `src-tauri/src/poll_loop.rs` — at the call site (~line 276), read and pass the new field:

```rust
                let (thresholds, payg_threshold) = {
                    let s = state.settings.read();
                    (s.thresholds.clone(), s.payg_threshold)
                };
                if let Ok(fired) = notifier::evaluate(
                    &state.db,
                    &cached.account_id,
                    &snapshot,
                    &thresholds,
                    payg_threshold,
                    Utc::now(),
                ) {
```

- [ ] **Step 4: Run the backend suite**

Run: `cd src-tauri && cargo test`
Expected: all tests pass, including the 4 new ones. Fix any remaining `evaluate` call sites the compiler reports.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/app_state.rs src-tauri/src/commands.rs src-tauri/src/notifier/rules.rs src-tauri/src/poll_loop.rs
git commit -m "feat(notifier): give pay-as-you-go its own alert threshold"
```

---

### Task 2: ExtraBurnRate projection (backend)

**Files:**
- Modify: `src-tauri/src/app_state.rs` (new struct next to `BurnRateProjection` ~line 92; `CachedUsage` ~line 109-118)
- Modify: `src-tauri/src/poll_loop.rs` (buffer struct at `spawn` ~line 115-117, threading through `poll_all`→`fetch_and_apply_one`→`apply_fetch_outcome`, new fn next to `update_burn_rate` ~line 642, `hydrated_caches` ~line 603-640, `placeholder_cached`)

**Interfaces:**
- Consumes: `ExtraUsage` fields (`is_enabled`, `used_credits_cents`, `resets_at`), existing `update_burn_rate` conventions.
- Produces: `pub struct ExtraBurnRate { pub cents_per_min: f64, pub projected_cents_at_reset: Option<f64> }`; `CachedUsage.extra_burn_rate: Option<ExtraBurnRate>` (serde-default). Task 3 renders both.

- [ ] **Step 1: Write the failing tests**

In `src-tauri/src/poll_loop.rs`'s existing `#[cfg(test)]` module (it follows `update_burn_rate`; match its fixture style — read the module first and reuse its snapshot builder if one exists, otherwise construct `UsageSnapshot` inline as below):

```rust
    fn snap_with_extra(used_cents: u64, reset_in_hours: Option<i64>) -> UsageSnapshot {
        UsageSnapshot {
            five_hour: None,
            seven_day: None,
            seven_day_sonnet: None,
            seven_day_opus: None,
            extra_usage: Some(crate::usage_api::ExtraUsage {
                is_enabled: true,
                monthly_limit_cents: 10_000,
                used_credits_cents: used_cents,
                utilization: Some(used_cents as f64 / 100.0),
                resets_at: reset_in_hours.map(|h| Utc::now() + chrono::Duration::hours(h)),
            }),
            fetched_at: Utc::now(),
            unknown: Default::default(),
        }
    }

    #[test]
    fn extra_burn_rate_projects_dollars_at_reset() {
        let mut buf = VecDeque::new();
        let t0 = Utc::now();
        assert!(update_extra_burn_rate(&mut buf, &snap_with_extra(1000, Some(24)), t0).is_none());
        // 10 minutes later, 100 more cents spent → 10 cents/min.
        let r = update_extra_burn_rate(
            &mut buf,
            &snap_with_extra(1100, Some(24)),
            t0 + chrono::Duration::minutes(10),
        )
        .expect("two samples 10min apart project");
        assert!((r.cents_per_min - 10.0).abs() < 0.01);
        let projected = r.projected_cents_at_reset.expect("reset known");
        // 1100 + 10c/min * 24h(1440min) = 15500
        assert!((projected - 15_500.0).abs() < 60.0, "got {projected}");
    }

    #[test]
    fn extra_burn_rate_none_without_reset_date_still_reports_slope() {
        let mut buf = VecDeque::new();
        let t0 = Utc::now();
        update_extra_burn_rate(&mut buf, &snap_with_extra(1000, None), t0);
        let r = update_extra_burn_rate(
            &mut buf,
            &snap_with_extra(1100, None),
            t0 + chrono::Duration::minutes(10),
        )
        .expect("slope computable without reset");
        assert!(r.projected_cents_at_reset.is_none());
        assert!((r.cents_per_min - 10.0).abs() < 0.01);
    }

    #[test]
    fn extra_burn_rate_needs_two_minutes_of_span() {
        let mut buf = VecDeque::new();
        let t0 = Utc::now();
        update_extra_burn_rate(&mut buf, &snap_with_extra(1000, Some(24)), t0);
        assert!(update_extra_burn_rate(
            &mut buf,
            &snap_with_extra(1010, Some(24)),
            t0 + chrono::Duration::seconds(60),
        )
        .is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test extra_burn`
Expected: compile error — `update_extra_burn_rate` does not exist.

- [ ] **Step 3: Implement**

3a. `src-tauri/src/app_state.rs` — next to `BurnRateProjection`:

```rust
/// Linear projection of pay-as-you-go spend, sibling to BurnRateProjection:
/// in-memory samples only (empty right after launch), >=2 samples spanning
/// >=2 minutes. Dollars are tracked as cents to match the API payload.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ExtraBurnRate {
    /// Spend velocity in cents per minute.
    pub cents_per_min: f64,
    /// Projected used_credits_cents at extra_usage.resets_at. None when the
    /// account reports no reset date (common for PAYG).
    pub projected_cents_at_reset: Option<f64>,
}
```

And on `CachedUsage`, after `burn_rate`:

```rust
    #[serde(default)]
    pub extra_burn_rate: Option<ExtraBurnRate>,
```

3b. `src-tauri/src/poll_loop.rs`:

Replace the per-slot buffer type with a two-buffer struct (single param, no signature explosion):

```rust
#[derive(Default)]
pub struct SlotBurnBuffers {
    five_hour: VecDeque<(DateTime<Utc>, f64)>,
    extra: VecDeque<(DateTime<Utc>, f64)>,
}
```

- `spawn` (~line 117): `let mut burn_buffers: HashMap<u32, SlotBurnBuffers> = HashMap::new();`
- Thread the changed type through `poll_all` → `fetch_and_apply_one` → `apply_fetch_outcome` (mechanical: the compiler lists every site).
- In `apply_fetch_outcome`'s `Ok(snapshot)` arm:

```rust
            let bufs = burn_buffers.entry(slot).or_default();
            let burn_rate = update_burn_rate(&mut bufs.five_hour, &snapshot, Utc::now());
            let extra_burn_rate = update_extra_burn_rate(&mut bufs.extra, &snapshot, Utc::now());
```

and add `extra_burn_rate,` to the `CachedUsage` literal.

New function next to `update_burn_rate`:

```rust
fn update_extra_burn_rate(
    buf: &mut VecDeque<(DateTime<Utc>, f64)>,
    snapshot: &UsageSnapshot,
    now: DateTime<Utc>,
) -> Option<ExtraBurnRate> {
    let extra = snapshot.extra_usage.as_ref()?;
    if !extra.is_enabled {
        return None;
    }
    // PAYG windows run ~30 days; a rolling day of samples gives a stable
    // slope without unbounded growth.
    let cutoff = now - ChronoDuration::hours(24);
    while let Some(&(ts, _)) = buf.front() {
        if ts < cutoff {
            buf.pop_front();
        } else {
            break;
        }
    }
    let used = extra.used_credits_cents as f64;
    buf.push_back((now, used));
    if buf.len() < 2 {
        return None;
    }
    let &(t0, u0) = buf.front()?;
    let &(t1, u1) = buf.back()?;
    let span_minutes = (t1 - t0).num_seconds() as f64 / 60.0;
    if span_minutes < 2.0 {
        return None;
    }
    let cents_per_min = (u1 - u0) / span_minutes;
    let projected_cents_at_reset = extra.resets_at.map(|reset| {
        let mins_until_reset = ((reset - now).num_seconds() as f64 / 60.0).max(0.0);
        u1 + cents_per_min * mins_until_reset
    });
    Some(ExtraBurnRate { cents_per_min, projected_cents_at_reset })
}
```

3c. Every other `CachedUsage { ... }` literal (`hydrated_caches`, `placeholder_cached`, and any in `commands.rs` — the compiler finds them all): add `extra_burn_rate: None,`.

- [ ] **Step 4: Run the backend suite**

Run: `cd src-tauri && cargo test`
Expected: all pass, including the 3 new tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/app_state.rs src-tauri/src/poll_loop.rs src-tauri/src/commands.rs
git commit -m "feat(poll): project pay-as-you-go spend as a dollar burn rate"
```

---

### Task 3: Dollars + forecast on the PAYG row (frontend)

**Files:**
- Modify: `src/lib/generated/bindings.ts` (add `ExtraBurnRate` type next to `BurnRateProjection` ~line 465; add `extra_burn_rate?: ExtraBurnRate | null` to `CachedUsage` ~line 479)
- Modify: `src/lib/format.ts` (new `formatCents`)
- Create: `src/lib/format.test.ts`
- Modify: `src/components/UsageSummary.tsx` (`ExtraRow` ~line 201-225 and its call site ~line 138-151)
- Modify: `src/components/UsageSummary.test.tsx` (new describe block)

**Interfaces:**
- Consumes: `CachedUsage.extra_burn_rate` and `ExtraUsage.{used_credits_cents,monthly_limit_cents}` from Task 2's shape.
- Produces: `formatCents(cents: number): string` → `"$31.20"`. Task 4 does not depend on this task.

- [ ] **Step 1: Write the failing tests**

`src/lib/format.test.ts` (new file, co-located pure-function test):

```ts
import { describe, it, expect } from 'vitest';
import { formatCents } from './format';

describe('formatCents', () => {
  it('formats cents as dollars with two decimals', () => {
    expect(formatCents(3120)).toBe('$31.20');
    expect(formatCents(10000)).toBe('$100.00');
    expect(formatCents(0)).toBe('$0.00');
    expect(formatCents(5)).toBe('$0.05');
  });
});
```

Append to `src/components/UsageSummary.test.tsx` (reuse its `usage()` fixture):

```tsx
function paygUsage(): CachedUsage {
  const u = usage();
  u.snapshot.extra_usage = {
    is_enabled: true,
    monthly_limit_cents: 10000,
    used_credits_cents: 3120,
    utilization: 31.2,
    resets_at: new Date(Date.now() + 10 * 86400_000).toISOString(),
  };
  (u as CachedUsage).extra_burn_rate = {
    cents_per_min: 1.5,
    projected_cents_at_reset: 5200,
  };
  return u;
}

describe('pay-as-you-go dollars and forecast', () => {
  it('shows spent-of-limit dollars and the projected spend', () => {
    render(
      <UsageSummary usage={paygUsage()} thresholds={[75, 90]} />,
    );
    expect(screen.getByText(/\$31\.20 of \$100\.00/)).toBeTruthy();
    expect(screen.getByText(/→ ~\$52 by reset/)).toBeTruthy();
  });

  it('hides dollars when no limit and forecast when no projection', () => {
    const u = paygUsage();
    u.snapshot.extra_usage!.monthly_limit_cents = 0;
    u.extra_burn_rate = null;
    render(<UsageSummary usage={u} thresholds={[75, 90]} />);
    expect(screen.queryByText(/ of \$/)).toBeNull();
    expect(screen.queryByText(/by reset/)).toBeNull();
  });
});
```

(If `UsageSummary` in the non-collapsible form hides the extra row, pass `collapsible detailsOpen` as the existing tests do — mirror whichever prop shape the file's existing `showExtra` tests use.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm exec vitest run src/lib/format.test.ts src/components/UsageSummary.test.tsx`
Expected: `formatCents` unresolved; PAYG describe fails.

- [ ] **Step 3: Implement**

3a. `src/lib/format.ts`:

```ts
/** "$31.20" from an integer cents amount (PAYG credit fields). */
export function formatCents(cents: number): string {
  return `$${(cents / 100).toFixed(2)}`;
}
```

3b. `src/lib/generated/bindings.ts` — insert after the `BurnRateProjection` type, matching the file's comment style:

```ts
/**
 * Linear projection of pay-as-you-go spend, sibling to BurnRateProjection:
 * in-memory samples only (empty right after launch), >=2 samples spanning
 * >=2 minutes. Dollars are tracked as cents to match the API payload.
 */
export type ExtraBurnRate = { 
/**
 * Spend velocity in cents per minute.
 */
cents_per_min: number; 
/**
 * Projected used_credits_cents at extra_usage.resets_at. None when the
 * account reports no reset date (common for PAYG).
 */
projected_cents_at_reset: number | null }
```

And on the `CachedUsage` line add `extra_burn_rate?: ExtraBurnRate | null; ` after `burn_rate?: BurnRateProjection | null; `.

3c. `src/components/UsageSummary.tsx`:

- Call site: pass the extra data through —

```tsx
            <ExtraRow
              pct={extra.utilization ?? 0}
              resetsAt={extra.resets_at ?? null}
              usedCents={extra.used_credits_cents ?? 0}
              limitCents={extra.monthly_limit_cents ?? 0}
              burn={usage.extra_burn_rate ?? null}
              warnAt={warn}
              dangerAt={danger}
            />
```

- `ExtraRow`: extend props and render a sub-line under the `InstrumentRow`:

```tsx
function ExtraRow({
  pct,
  resetsAt,
  usedCents,
  limitCents,
  burn,
  warnAt,
  dangerAt,
}: {
  pct: number;
  resetsAt: string | null;
  usedCents: number;
  limitCents: number;
  burn: ExtraBurnRate | null;
  warnAt: number;
  dangerAt: number;
}) {
  const data: Utilization | null = resetsAt
    ? { utilization: pct, resets_at: resetsAt }
    : null;
  // Hide a flat forecast: under one cent a day extrapolates to nothing.
  const showForecast =
    burn != null &&
    burn.projected_cents_at_reset != null &&
    Math.abs(burn.cents_per_min) * 1440 >= 1;
  return (
    <div className="flex flex-col gap-[2px]">
      <InstrumentRow
        label="Pay-as-you-go"
        caption={resetsAt ? undefined : 'no reset window'}
        value={pct}
        data={data}
        warnAt={warnAt}
        dangerAt={dangerAt}
      />
      {(limitCents > 0 || showForecast) && (
        <div className="flex items-baseline justify-between gap-[var(--space-xs)]">
          {limitCents > 0 ? (
            <span className="mono text-[length:var(--text-micro)] tabular-nums text-[color:var(--color-text-secondary)]">
              {formatCents(usedCents)} of {formatCents(limitCents)}
            </span>
          ) : (
            <span />
          )}
          {showForecast && (
            <span
              className="text-[length:var(--text-micro)] tabular-nums text-[color:var(--color-text-muted)]"
              title={`${burn.cents_per_min >= 0 ? '+' : ''}${(burn.cents_per_min * 14.4).toFixed(2)} $/day`}
            >
              → ~${Math.round((burn.projected_cents_at_reset as number) / 100)} by reset
            </span>
          )}
        </div>
      )}
    </div>
  );
}
```

Add imports: `formatCents` from `../lib/format`, `ExtraBurnRate` from the bindings/types re-export used by the file's existing imports.

- [ ] **Step 4: Run tests to verify they pass**

Run: `pnpm exec vitest run src/lib/format.test.ts src/components/UsageSummary.test.tsx`
Expected: PASS. Then `pnpm lint` → clean.

- [ ] **Step 5: Commit**

```bash
git add src/lib/format.ts src/lib/format.test.ts src/lib/generated/bindings.ts src/components/UsageSummary.tsx src/components/UsageSummary.test.tsx
git commit -m "feat(popover): show pay-as-you-go dollars and projected spend"
```

---

### Task 4: PAYG threshold slider (frontend settings)

**Files:**
- Modify: `src/lib/generated/bindings.ts` (`Settings` type ~line 632: add `payg_threshold: number; ` after the `thresholds` field)
- Modify: `src/settings/SettingsPanel.tsx` (Notifications card ~line 222-251)
- Modify: the existing SettingsPanel test file in `src/settings/__tests__/` (follow its established mock/render pattern; if none covers the panel, add the assertion to whichever settings test exists)

**Interfaces:**
- Consumes: `Settings.payg_threshold` from Task 1 (via bindings edit here).
- Produces: user-editable slider persisted through the existing `update('payg_threshold', v)` → `save()` flow.

- [ ] **Step 1: Write the failing test**

In the SettingsPanel test file (match its existing setup exactly — it already mocks `ipc`/store; add):

```tsx
  it('renders and updates the pay-as-you-go threshold slider', () => {
    // Render the panel with settings whose payg_threshold is 85, assert a
    // slider labeled /pay-as-you-go/i exists showing 85%, change it to 90,
    // and assert the local draft reflects 90% — following this file's
    // existing slider-interaction pattern for thresholds.
  });
```

Write the real body by copying the file's existing threshold-slider test steps (fireEvent.change on the range input, assert formatted value). The assertion targets are: label text matching `/pay-as-you-go threshold/i`, initial `85%`, post-change `90%`.

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm exec vitest run src/settings/__tests__`
Expected: new test fails (no such slider).

- [ ] **Step 3: Implement**

3a. Bindings: add `payg_threshold: number; ` to the `Settings` type after `thresholds: number[]; `.

3b. `SettingsPanel.tsx`, inside the Notifications `<Card>` after the existing `local.thresholds.map(...)` sliders:

```tsx
          <Slider
            label="Pay-as-you-go threshold"
            min={25}
            max={95}
            step={5}
            value={local.payg_threshold}
            onChange={(e) => update('payg_threshold', Number(e.target.value))}
            formatValue={(v) => `${v}%`}
          />
          <p className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] px-[var(--space-2xs)]">
            Credits alert fires at this level — separate from the rate-limit thresholds above.
          </p>
```

- [ ] **Step 4: Run the full frontend gate**

Run: `pnpm lint` → clean. Run: `pnpm test` → no new failures (baseline theme.test.ts collection error only).

- [ ] **Step 5: Commit**

```bash
git add src/lib/generated/bindings.ts src/settings/SettingsPanel.tsx src/settings/__tests__
git commit -m "feat(settings): pay-as-you-go threshold slider"
```

---

## Verification checklist (after all tasks)

- `cd src-tauri && cargo test` fully green; `pnpm lint` clean; `pnpm test` no new failures.
- Manual: with PAYG enabled, popover Details shows `$X of $Y`; after ≥2 polls ≥2 min apart with spend movement and a known reset date, the `→ ~$N by reset` caption appears; Settings shows the PAYG slider at 85%.
- Old settings blobs load with `payg_threshold = 85` (deserialization default).
