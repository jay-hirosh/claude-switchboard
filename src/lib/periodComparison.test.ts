import { describe, it, expect } from 'vitest';
import { computePeriodComparison } from './periodComparison';
import type { DailyBucket } from './types';

function bucket(date: string, input: number, output: number, cost: number, requests: number): DailyBucket {
  return { date, input_tokens: input, output_tokens: output, cost_usd: cost, request_count: requests };
}

describe('computePeriodComparison — week granularity', () => {
  it('sums the trailing 7 days as current and the 7 before that as prior', () => {
    // "now" = 2026-08-14 (Fri). Current week: Aug 8–14. Prior week: Aug 1–7.
    const now = new Date(2026, 7, 14);
    const buckets: DailyBucket[] = [
      bucket('2026-08-01', 1000, 0, 1, 1), // prior week
      bucket('2026-08-07', 1000, 0, 1, 1), // prior week
      bucket('2026-08-08', 2000, 0, 2, 1), // current week
      bucket('2026-08-14', 2000, 0, 2, 1), // current week
      bucket('2026-07-31', 9999, 0, 9, 9), // outside both — must not count
    ];

    const rows = computePeriodComparison(buckets, 'week', now);
    const cost = rows.find((r) => r.metric === 'cost')!;
    expect(cost.current).toBeCloseTo(4); // 2 + 2
    expect(cost.prior).toBeCloseTo(2); // 1 + 1
    expect(cost.deltaPct).toBeCloseTo(100); // doubled
  });

  it('returns a null deltaPct when the prior period has no data, instead of NaN or Infinity', () => {
    const now = new Date(2026, 7, 14);
    const buckets: DailyBucket[] = [bucket('2026-08-10', 1000, 0, 5, 3)];

    const rows = computePeriodComparison(buckets, 'week', now);
    const cost = rows.find((r) => r.metric === 'cost')!;
    expect(cost.current).toBe(5);
    expect(cost.prior).toBe(0);
    expect(cost.deltaPct).toBeNull();
  });
});

describe('computePeriodComparison — month granularity', () => {
  it('compares month-to-date against the full prior calendar month', () => {
    // "now" = 2026-03-15. Current: Mar 1–15. Prior: all of Feb (28 days, 2026 not a leap year).
    const now = new Date(2026, 2, 15);
    const buckets: DailyBucket[] = [
      bucket('2026-02-01', 0, 0, 10, 5),
      bucket('2026-02-28', 0, 0, 10, 5),
      bucket('2026-03-01', 0, 0, 6, 2),
      bucket('2026-03-15', 0, 0, 6, 2),
      bucket('2026-01-31', 0, 0, 999, 999), // outside both — must not count
    ];

    const rows = computePeriodComparison(buckets, 'month', now);
    const cost = rows.find((r) => r.metric === 'cost')!;
    expect(cost.current).toBeCloseTo(12); // 6 + 6
    expect(cost.prior).toBeCloseTo(20); // 10 + 10
  });

  it('handles the first day of a month, where the prior month has no overlap risk', () => {
    const now = new Date(2026, 2, 1); // March 1
    const buckets: DailyBucket[] = [
      bucket('2026-02-28', 0, 0, 4, 1),
      bucket('2026-03-01', 0, 0, 3, 1),
    ];

    const rows = computePeriodComparison(buckets, 'month', now);
    const cost = rows.find((r) => r.metric === 'cost')!;
    expect(cost.current).toBeCloseTo(3);
    expect(cost.prior).toBeCloseTo(4);
  });
});

describe('computePeriodComparison — sparkline series', () => {
  it('includes the current period\'s daily values in chronological order, one point per metric', () => {
    const now = new Date(2026, 7, 14);
    const buckets: DailyBucket[] = [
      bucket('2026-08-13', 0, 0, 1, 1),
      bucket('2026-08-08', 0, 0, 2, 1),
      bucket('2026-08-14', 0, 0, 3, 1),
    ];

    const rows = computePeriodComparison(buckets, 'week', now);
    const cost = rows.find((r) => r.metric === 'cost')!;
    expect(cost.series).toEqual([2, 1, 3]); // Aug 8, 13, 14 — chronological, missing days omitted
  });
});

describe('computePeriodComparison — metrics', () => {
  it('reports cost, tokens (input+output), and requests as separate rows', () => {
    const now = new Date(2026, 7, 14);
    const buckets: DailyBucket[] = [bucket('2026-08-14', 100, 50, 2.5, 4)];

    const rows = computePeriodComparison(buckets, 'week', now);
    expect(rows.map((r) => r.metric).sort()).toEqual(['cost', 'requests', 'tokens']);
    expect(rows.find((r) => r.metric === 'tokens')!.current).toBe(150);
    expect(rows.find((r) => r.metric === 'requests')!.current).toBe(4);
  });
});
