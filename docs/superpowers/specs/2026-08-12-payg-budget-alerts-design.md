# PAYG budget alerts — $ burn-rate forecast + independent threshold

**Status:** Design approved (user selected: both halves; single threshold slider, default 85%)
**Date:** 2026-08-12
**Roadmap:** backlog item alongside Phase 2 in `docs/superpowers/roadmap-2026-08-12.md`

## 1. Problem

Pay-as-you-go credits are the only real-money bucket in the app, yet they get the least insight: the popover's PAYG row shows a bare utilization %, the `monthly_limit_cents` / `used_credits_cents` fields Anthropic already returns are rendered nowhere, there is no forecast ("am I on pace to blow the budget?"), and PAYG alerts share the 5H/7D threshold sliders instead of having their own.

## 2. Goals / non-goals

**Goals**
- Show real dollars on the PAYG row: `$31.20 of $100.00`.
- Extend the existing burn-rate machinery to PAYG: `→ ~$52 by reset` when a reset date is known.
- Give PAYG its own single notification threshold (default 85%), separate from the shared 5H/7D sliders.

**Non-goals**
- No multi-threshold PAYG (user chose one slider).
- No 7-day burn-rate forecast (separate feature if ever).
- No change to when notifications are evaluated (still active-slot-only in the poll loop — PAYG is account-scoped like every other bucket).
- No historical spend charting.

## 3. Architecture

### 3.1 Backend — $ burn rate (mirrors the existing 5H projection)

The 5H projection works off an in-memory per-slot ring buffer of `(timestamp, utilization)` samples filled by the poll loop (`poll_loop.rs:642-673`, buffer created at `:117`), needing ≥2 samples ≥2 min apart; it is deliberately empty right after launch (`hydrated_caches` sets `burn_rate: None`). PAYG follows the identical pattern with dollars:

```rust
// app_state.rs, next to BurnRateProjection
/// Linear projection of pay-as-you-go spend. Same sampling rules as
/// BurnRateProjection: >=2 samples spanning >=2 minutes, in-memory only.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ExtraBurnRate {
    /// Spend velocity in cents per minute (can be 0.0 when idle).
    pub cents_per_min: f64,
    /// Projected used_credits_cents at resets_at. None when the account
    /// reports no reset date (PAYG frequently has resets_at = null).
    pub projected_cents_at_reset: Option<f64>,
}
```

- New `extra_burn_buffers: HashMap<u32, VecDeque<(DateTime<Utc>, f64)>>` alongside `burn_buffers` in the poll loop, sampling `used_credits_cents as f64` whenever `extra_usage.is_enabled`. Samples older than 24h are evicted (PAYG's window is ~30 days; a day of samples gives a stable slope without unbounded growth).
- `CachedUsage` gains `#[serde(default)] pub extra_burn_rate: Option<ExtraBurnRate>` — additive, hydration-safe (hydrated caches get `None`, same as `burn_rate`).
- Slope from first→last sample; `projected_cents_at_reset = used + slope * minutes_until_reset` only when `resets_at` is `Some`.

### 3.2 Backend — independent threshold

- `Settings` gains `pub payg_threshold: u8`, default `85` (added to the custom `Default` impl at `app_state.rs:46-59`; the struct-level `#[serde(default)]` makes old stored blobs deserialize fine — no migration, same as `stagger_gap_secs`).
- `update_settings` validation: `payg_threshold <= 100`.
- `notifier::evaluate()` signature gains `payg_threshold: u8`. The `Bucket::ExtraUsage` arm stops iterating the shared `thresholds` slice and checks only `payg_threshold`. Dedup semantics unchanged (once per reset cycle when `resets_at` known, 24h cooldown otherwise — `rules.rs:100-112` already handles both).
- Notification body upgraded to dollars when the fields are present: `"Used $42.50 of $50.00"` (falls back to the current wording when `monthly_limit_cents == 0`).

### 3.3 Frontend

- `src/lib/format.ts`: new `formatCents(cents: number): string` → `"$31.20"` (two decimals, no thousands separator needed at these magnitudes).
- `ExtraRow` in `src/components/UsageSummary.tsx` (currently renders only `pct` + `resetsAt` via `InstrumentRow`) gains:
  - a secondary line `$31.20 of $100.00` when `monthly_limit_cents > 0`;
  - a forecast caption `→ ~$52 by reset` when `extra_burn_rate.projected_cents_at_reset` is present, following `BurnRateCaption`'s conventions (`UsageSummary.tsx:164-195`), including its hide-when-flat rule (hide when `cents_per_min` rounds to $0.00/day).
- `SettingsPanel.tsx` Notifications card: one new `Slider` labeled "Pay-as-you-go threshold", min 25 / max 95 / step 5 (same rails as the existing threshold sliders), bound to `payg_threshold`, with a caption noting it replaces the shared thresholds for the credits bucket.
- `src/lib/generated/bindings.ts`: `ExtraBurnRate` type, `CachedUsage.extra_burn_rate`, `Settings.payg_threshold` — hand-added in tauri-specta's exact output style (the file is generated but committed; verify with `pnpm lint` and regenerate on next debug run).

## 4. Edge cases

- **`resets_at` absent** (common for PAYG): forecast caption hidden entirely — a projection without an endpoint is noise. The $ spent/limit line still shows.
- **Credits topped up / limit raised mid-window**: `used_credits_cents` can drop or `monthly_limit_cents` change between samples; a negative slope simply projects lower (fine). Samples spanning a top-up produce a briefly-wrong slope that self-corrects as old samples age out — same tolerance the 5H projection already accepts.
- **PAYG disabled** (`is_enabled == false`): no sampling, no forecast, row hidden (existing behavior), threshold check skipped (utilization is `None`, existing `evaluate` guard).
- **Old settings blob without `payg_threshold`**: deserializes via `Default` → 85.

## 5. Testing

- Rust (`notifier/rules.rs` tests): PAYG fires at its own threshold and NOT at the shared thresholds; shared buckets ignore `payg_threshold`; dollar body formatting with/without limit.
- Rust (poll_loop or a small unit around the slope fn): projection math — two samples → cents/min; `None` reset → `projected_cents_at_reset: None`.
- TS: `formatCents` unit cases (0, 3120, 10000); `UsageSummary.test.tsx` — $ line renders when limit present, forecast renders when projection present, both absent otherwise.

## 6. Open questions

None blocking.

## 7. File-level checklist

Modified (backend): `src-tauri/src/app_state.rs` (ExtraBurnRate, CachedUsage field, Settings field + Default), `src-tauri/src/poll_loop.rs` (extra buffer + sampling + projection), `src-tauri/src/commands.rs` (validation), `src-tauri/src/notifier/rules.rs` (signature + ExtraUsage arm + body).
Modified (frontend): `src/lib/format.ts`, `src/lib/generated/bindings.ts`, `src/components/UsageSummary.tsx`, `src/settings/SettingsPanel.tsx`, plus their tests.
No new files. No DB migration. No new IPC commands.
