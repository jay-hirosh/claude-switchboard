import { describe, it, expect } from 'vitest';
import { computeMonthPacing } from './monthPacing';
import type { DailyBucket } from './types';

function bucket(date: string, cost: number): DailyBucket {
  return { date, input_tokens: 0, output_tokens: 0, cost_usd: cost, request_count: 1 };
}

describe('computeMonthPacing', () => {
  it('projects month-end spend by extrapolating the daily rate so far', () => {
    // now = Aug 10 (10th day of a 31-day month). Spent $50 across the first 10 days.
    const now = new Date(2026, 7, 10);
    const buckets: DailyBucket[] = [
      bucket('2026-08-01', 20),
      bucket('2026-08-10', 30),
      bucket('2026-07-31', 999), // prior month — must not count
    ];

    const pacing = computeMonthPacing(buckets, now);
    expect(pacing.spentSoFar).toBeCloseTo(50);
    expect(pacing.daysElapsed).toBe(10);
    expect(pacing.daysInMonth).toBe(31);
    // rate = 50/10 = 5/day; projected = 5 * 31 = 155
    expect(pacing.projectedTotal).toBeCloseTo(155);
  });

  it('handles the first day of the month without dividing by zero', () => {
    const now = new Date(2026, 7, 1);
    const buckets: DailyBucket[] = [bucket('2026-08-01', 12)];

    const pacing = computeMonthPacing(buckets, now);
    expect(pacing.daysElapsed).toBe(1);
    // rate = 12/1 = 12/day; projected = 12 * 31 = 372
    expect(pacing.projectedTotal).toBeCloseTo(372);
  });

  it('projects zero when there is no spend yet this month', () => {
    const now = new Date(2026, 7, 5);
    const pacing = computeMonthPacing([], now);
    expect(pacing.spentSoFar).toBe(0);
    expect(pacing.projectedTotal).toBe(0);
  });

  it('accounts for a shorter month (February, non-leap year)', () => {
    const now = new Date(2026, 1, 14); // Feb 14, 2026 — not a leap year
    const buckets: DailyBucket[] = [bucket('2026-02-14', 28)];

    const pacing = computeMonthPacing(buckets, now);
    expect(pacing.daysInMonth).toBe(28);
  });
});
