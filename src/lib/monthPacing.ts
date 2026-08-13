import type { DailyBucket } from './types';
import { toDateKey, startOfMonth } from './periodComparison';

export interface MonthPacing {
  spentSoFar: number;
  projectedTotal: number;
  daysElapsed: number;
  daysInMonth: number;
}

/**
 * Projects this calendar month's total spend by extrapolating the daily
 * rate seen so far — a simple linear pace, not a live-sampled burn rate
 * (unlike PAYG's), since this works off historical daily buckets already
 * in hand rather than a live poll ring buffer.
 */
export function computeMonthPacing(buckets: DailyBucket[], now: Date): MonthPacing {
  const startKey = toDateKey(startOfMonth(now));
  const endKey = toDateKey(now);

  const spentSoFar = buckets
    .filter((b) => b.date >= startKey && b.date <= endKey)
    .reduce((sum, b) => sum + b.cost_usd, 0);

  const daysElapsed = now.getDate();
  const daysInMonth = new Date(now.getFullYear(), now.getMonth() + 1, 0).getDate();
  const projectedTotal = (spentSoFar / daysElapsed) * daysInMonth;

  return { spentSoFar, projectedTotal, daysElapsed, daysInMonth };
}
