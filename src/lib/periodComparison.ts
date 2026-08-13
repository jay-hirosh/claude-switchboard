import type { DailyBucket } from './types';

export type PeriodMetric = 'cost' | 'tokens' | 'requests';
export type PeriodGranularity = 'week' | 'month';

export interface PeriodComparisonRow {
  metric: PeriodMetric;
  current: number;
  prior: number;
  /** null when the prior period has no data — an undefined % change, not 0 or Infinity. */
  deltaPct: number | null;
  /** Current period's daily values, chronological, missing days omitted — for the sparkline. */
  series: number[];
}

export function toDateKey(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

function addDays(d: Date, delta: number): Date {
  const copy = new Date(d);
  copy.setDate(copy.getDate() + delta);
  return copy;
}

export function startOfMonth(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), 1);
}

function inRange(buckets: DailyBucket[], startKey: string, endKey: string): DailyBucket[] {
  return buckets
    .filter((b) => b.date >= startKey && b.date <= endKey)
    .sort((a, b) => a.date.localeCompare(b.date));
}

function sumRange(buckets: DailyBucket[], startKey: string, endKey: string) {
  let cost = 0;
  let tokens = 0;
  let requests = 0;
  for (const b of inRange(buckets, startKey, endKey)) {
    cost += b.cost_usd;
    tokens += b.input_tokens + b.output_tokens;
    requests += b.request_count;
  }
  return { cost, tokens, requests };
}

/**
 * Sums `buckets` into a current-vs-prior comparison for cost, tokens, and
 * requests. "week" compares the trailing 7 days against the 7 before that;
 * "month" compares month-to-date against the full prior calendar month —
 * uneven month lengths mean the two spans aren't the same number of days,
 * which is expected for a calendar-aligned comparison.
 */
export function computePeriodComparison(
  buckets: DailyBucket[],
  granularity: PeriodGranularity,
  now: Date,
): PeriodComparisonRow[] {
  let currentStart: Date;
  let currentEnd: Date;
  let priorStart: Date;
  let priorEnd: Date;

  if (granularity === 'week') {
    currentEnd = now;
    currentStart = addDays(now, -6);
    priorEnd = addDays(now, -7);
    priorStart = addDays(now, -13);
  } else {
    currentStart = startOfMonth(now);
    currentEnd = now;
    priorEnd = addDays(currentStart, -1);
    priorStart = startOfMonth(priorEnd);
  }

  const currentDays = inRange(buckets, toDateKey(currentStart), toDateKey(currentEnd));
  const current = sumRange(buckets, toDateKey(currentStart), toDateKey(currentEnd));
  const prior = sumRange(buckets, toDateKey(priorStart), toDateKey(priorEnd));

  const seriesFor: Record<PeriodMetric, (b: DailyBucket) => number> = {
    cost: (b) => b.cost_usd,
    tokens: (b) => b.input_tokens + b.output_tokens,
    requests: (b) => b.request_count,
  };

  return (['cost', 'tokens', 'requests'] as const).map((metric) => {
    const c = current[metric];
    const p = prior[metric];
    return {
      metric,
      current: c,
      prior: p,
      deltaPct: p > 0 ? ((c - p) / p) * 100 : null,
      series: currentDays.map(seriesFor[metric]),
    };
  });
}
