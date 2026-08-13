import { describe, it, expect } from 'vitest';
import { detectAnomalies } from './anomalyDetection';
import type { DailyBucket } from './types';

function bucket(date: string, cost: number, tokens: number): DailyBucket {
  return { date, input_tokens: tokens, output_tokens: 0, cost_usd: cost, request_count: 1 };
}

// 7 prior active days at a steady baseline of $1/day, 100 tokens/day.
function baselineDays(): DailyBucket[] {
  return Array.from({ length: 7 }, (_, i) => bucket(`2026-08-0${i + 1}`, 1, 100));
}

describe('detectAnomalies', () => {
  it('flags a day whose cost is at least 2x the trailing baseline average', () => {
    const buckets = [...baselineDays(), bucket('2026-08-08', 3, 100)]; // 3x baseline cost
    const anomalies = detectAnomalies(buckets);
    expect(anomalies.has('2026-08-08')).toBe(true);
    expect(anomalies.get('2026-08-08')!.costRatio).toBeCloseTo(3);
  });

  it('flags a day whose tokens are at least 2x the trailing baseline, even if cost is normal', () => {
    const buckets = [...baselineDays(), bucket('2026-08-08', 1, 250)]; // 2.5x baseline tokens
    const anomalies = detectAnomalies(buckets);
    expect(anomalies.has('2026-08-08')).toBe(true);
    expect(anomalies.get('2026-08-08')!.tokensRatio).toBeCloseTo(2.5);
  });

  it('does not flag a day within normal range of the baseline', () => {
    const buckets = [...baselineDays(), bucket('2026-08-08', 1.5, 120)]; // 1.5x, 1.2x — under threshold
    const anomalies = detectAnomalies(buckets);
    expect(anomalies.has('2026-08-08')).toBe(false);
  });

  it('does not flag a day with fewer than the minimum prior active days of history', () => {
    const buckets = [bucket('2026-08-01', 1, 100), bucket('2026-08-02', 1, 100), bucket('2026-08-03', 100, 100)];
    const anomalies = detectAnomalies(buckets);
    expect(anomalies.has('2026-08-03')).toBe(false);
  });

  it('does not flag when the baseline is zero, to avoid a division artifact', () => {
    const zeroBaseline = Array.from({ length: 7 }, (_, i) => bucket(`2026-08-0${i + 1}`, 0, 0));
    const buckets = [...zeroBaseline, bucket('2026-08-08', 5, 500)];
    const anomalies = detectAnomalies(buckets);
    expect(anomalies.has('2026-08-08')).toBe(false);
  });

  it('respects a custom threshold ratio', () => {
    const buckets = [...baselineDays(), bucket('2026-08-08', 1.5, 100)]; // 1.5x
    expect(detectAnomalies(buckets, { thresholdRatio: 1.5 }).has('2026-08-08')).toBe(true);
    expect(detectAnomalies(buckets, { thresholdRatio: 2 }).has('2026-08-08')).toBe(false);
  });
});
