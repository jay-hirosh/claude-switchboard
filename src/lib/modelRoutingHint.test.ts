import { describe, it, expect } from 'vitest';
import type { UsageSnapshot, Utilization } from './generated/bindings';
import { computeModelRoutingHint } from './modelRoutingHint';

function snap(
  opus: number | null,
  sonnet: number | null,
  opusResetsAt: string | null = null,
  sonnetResetsAt: string | null = null,
): UsageSnapshot {
  const bucket = (pct: number | null, resetsAt: string | null): Utilization | null =>
    pct === null ? null : { utilization: pct, resets_at: resetsAt };
  return {
    five_hour: null,
    seven_day: null,
    seven_day_opus: bucket(opus, opusResetsAt),
    seven_day_sonnet: bucket(sonnet, sonnetResetsAt),
    extra_usage: null,
  };
}

describe('computeModelRoutingHint', () => {
  it('flags the busier model when it clears the warn floor and the gap threshold', () => {
    const hint = computeModelRoutingHint(snap(82, 31), [75, 90]);
    expect(hint).toEqual({ busier: 'opus', busierPct: 82, quieterPct: 31, resetsAt: null });
  });

  it('returns null when the busier bucket is under the warn threshold', () => {
    // Gap is 50pp, well over the 25pp default, but neither bucket is close to the limit.
    expect(computeModelRoutingHint(snap(60, 10), [75, 90])).toBeNull();
  });

  it('returns null when the gap is under the threshold', () => {
    expect(computeModelRoutingHint(snap(90, 70), [75, 90])).toBeNull();
  });

  it('flags gaps and warn floors at exactly the boundary', () => {
    const hint = computeModelRoutingHint(snap(75, 50), [75, 90]);
    expect(hint).toEqual({ busier: 'opus', busierPct: 75, quieterPct: 50, resetsAt: null });
  });

  it('returns null when neither model reports a bucket (no per-model split)', () => {
    expect(computeModelRoutingHint(snap(null, null), [75, 90])).toBeNull();
  });

  it('treats a missing single bucket as 0 usage', () => {
    const hint = computeModelRoutingHint(snap(null, 80), [75, 90]);
    expect(hint).toEqual({ busier: 'sonnet', busierPct: 80, quieterPct: 0, resetsAt: null });
  });

  it('returns the resets_at of the busier bucket', () => {
    const hint = computeModelRoutingHint(snap(82, 31, '2026-08-20T00:00:00Z', '2026-08-19T00:00:00Z'), [75, 90]);
    expect(hint?.resetsAt).toBe('2026-08-20T00:00:00Z');
  });

  it('respects a custom gap threshold', () => {
    expect(computeModelRoutingHint(snap(90, 80), [75, 90])).toBeNull();
    const hint = computeModelRoutingHint(snap(90, 80), [75, 90], 5);
    expect(hint).toEqual({ busier: 'opus', busierPct: 90, quieterPct: 80, resetsAt: null });
  });
});
