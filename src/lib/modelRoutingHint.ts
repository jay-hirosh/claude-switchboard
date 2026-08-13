import type { UsageSnapshot } from './generated/bindings';

export interface ModelRoutingHint {
  busier: 'opus' | 'sonnet';
  busierPct: number;
  quieterPct: number;
  resetsAt: string | null;
}

/**
 * Flags when a Max-plan account's per-model 7D buckets have diverged enough
 * that switching models would meaningfully extend runway.
 *
 * Suppressed (returns null) when: neither bucket carries data (no per-model
 * split on this plan); the busier bucket hasn't reached `thresholds[0]`
 * (warn) yet; or the gap between the two is under `gapThresholdPct`.
 */
export function computeModelRoutingHint(
  snap: UsageSnapshot,
  thresholds: [number, number],
  gapThresholdPct = 25,
): ModelRoutingHint | null {
  if (snap.seven_day_opus == null && snap.seven_day_sonnet == null) return null;

  const opusPct = snap.seven_day_opus?.utilization ?? 0;
  const sonnetPct = snap.seven_day_sonnet?.utilization ?? 0;
  const busier = opusPct >= sonnetPct ? 'opus' : 'sonnet';
  const busierPct = Math.max(opusPct, sonnetPct);
  const quieterPct = Math.min(opusPct, sonnetPct);

  const [warn] = thresholds;
  if (busierPct < warn) return null;
  if (busierPct - quieterPct < gapThresholdPct) return null;

  const resetsAt =
    (busier === 'opus' ? snap.seven_day_opus?.resets_at : snap.seven_day_sonnet?.resets_at) ??
    null;

  return { busier, busierPct, quieterPct, resetsAt };
}
