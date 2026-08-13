import type { DailyBucket } from './types';

export interface AnomalyRatios {
  costRatio: number | null;
  tokensRatio: number | null;
}

export interface AnomalyDetectionOptions {
  thresholdRatio?: number;
  baselineDays?: number;
  minBaselineDays?: number;
}

/**
 * Flags days whose cost or tokens ran well above their trailing baseline —
 * a quiet signal for a runaway session or accidental loop, not a
 * notification. The baseline is averaged over the prior N *active* bucket
 * entries (days with usage), not N calendar days — a quiet day shouldn't
 * dilute what "normal" looks like.
 */
export function detectAnomalies(
  buckets: DailyBucket[],
  options: AnomalyDetectionOptions = {},
): Map<string, AnomalyRatios> {
  const { thresholdRatio = 2, baselineDays = 14, minBaselineDays = 7 } = options;
  const sorted = [...buckets].sort((a, b) => a.date.localeCompare(b.date));
  const result = new Map<string, AnomalyRatios>();

  for (let i = 0; i < sorted.length; i++) {
    const day = sorted[i];
    const priorWindow = sorted.slice(Math.max(0, i - baselineDays), i);
    if (priorWindow.length < minBaselineDays) continue;

    const baselineCost = priorWindow.reduce((s, b) => s + b.cost_usd, 0) / priorWindow.length;
    const baselineTokens =
      priorWindow.reduce((s, b) => s + b.input_tokens + b.output_tokens, 0) / priorWindow.length;
    const dayTokens = day.input_tokens + day.output_tokens;

    const costRatio = baselineCost > 0 ? day.cost_usd / baselineCost : null;
    const tokensRatio = baselineTokens > 0 ? dayTokens / baselineTokens : null;

    const isAnomaly =
      (costRatio !== null && costRatio >= thresholdRatio) ||
      (tokensRatio !== null && tokensRatio >= thresholdRatio);
    if (isAnomaly) result.set(day.date, { costRatio, tokensRatio });
  }

  return result;
}
