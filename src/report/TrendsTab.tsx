import { useEffect, useMemo, useState } from 'react';
import { save as saveDialog } from '@tauri-apps/plugin-dialog';
import { Card } from '../components/ui/Card';
import { Badge } from '../components/ui/Badge';
import { Button } from '../components/ui/Button';
import { IconButton } from '../components/ui/IconButton';
import { EmptyState } from '../components/ui/EmptyState';
import { AccountBadge } from '../components/ui/AccountBadge';
import { formatTokens, formatCost } from '../lib/format';
import { IconTrends, IconExport } from '../lib/icons';
import { ipc } from '../lib/ipc';
import { useTabData } from '../lib/useTabData';
import { useAppStore } from '../lib/store';
import { MODEL_VARIANT, modelKey, shortName } from './modelDisplay';
import { colorForAccount } from './accountDisplay';
import { computePeriodComparison, type PeriodGranularity, type PeriodMetric } from '../lib/periodComparison';
import { computeMonthPacing } from '../lib/monthPacing';
import { detectAnomalies } from '../lib/anomalyDetection';

const PERIOD_METRIC_LABELS: Record<PeriodMetric, string> = {
  cost: 'Cost',
  tokens: 'Tokens',
  requests: 'Requests',
};

function formatDeltaPct(pct: number | null): string {
  if (pct === null) return 'new';
  const rounded = Math.round(pct);
  if (rounded === 0) return '±0%';
  return rounded > 0 ? `▲ +${rounded}%` : `▼ ${rounded}%`;
}

/** Points attribute for a 60×20 sparkline, scaled to the series' own max. */
function sparklinePoints(series: number[]): string {
  if (series.length === 0) return '';
  const max = Math.max(...series, 0.0001);
  const stepX = series.length > 1 ? 60 / (series.length - 1) : 60;
  return series
    .map((v, i) => {
      const x = series.length > 1 ? i * stepX : 30;
      const y = 19 - (v / max) * 18;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(' ');
}

/** Fixed stacking order for the bar chart and its legend, and the color each
 *  tier renders as — shared with the day-breakdown panel's model-fill bars
 *  so a color always means the same tier everywhere in this tab. */
const MODEL_ORDER = ['opus', 'sonnet', 'haiku', 'default'] as const;
const MODEL_COLORS: Record<(typeof MODEL_ORDER)[number], string> = {
  opus: 'var(--color-model-opus)',
  sonnet: 'var(--color-model-sonnet)',
  haiku: 'var(--color-model-haiku)',
  default: 'var(--color-text-muted)',
};
const MODEL_LABELS: Record<(typeof MODEL_ORDER)[number], string> = {
  opus: 'Opus',
  sonnet: 'Sonnet',
  haiku: 'Haiku',
  default: 'Other',
};

// Covers two full calendar months worst-case (two 31-day months back to
// back), so the period-comparison strip always has a complete prior-month
// range to compare against. Also the range the CSV export covers.
const TRENDS_FETCH_DAYS = 62;

export function TrendsTab() {
  const version = useAppStore((s) => s.sessionDataVersion);
  const accounts = useAppStore((s) => s.accounts);
  const { data, error, loading, reload } = useTabData(
    () =>
      Promise.all([
        ipc.getDailyTrends(TRENDS_FETCH_DAYS),
        ipc.getDailyModelBreakdown(30),
        ipc.getDailyAccountBreakdown(30),
      ]).then(([trends, breakdown, accountBreakdown]) => ({ trends, breakdown, accountBreakdown })),
    [version],
  );
  const [range, setRange] = useState<'7d' | '30d'>('30d');
  const [metric, setMetric] = useState<'tokens' | 'cost'>('tokens');
  const [colorBy, setColorBy] = useState<'model' | 'account'>('model');
  const [selectedDate, setSelectedDate] = useState<string | null>(null);
  const [periodGranularity, setPeriodGranularity] = useState<PeriodGranularity>('week');

  useEffect(() => {
    setSelectedDate(null);
  }, [range]);

  const trends = data?.trends ?? null;
  const breakdown = data?.breakdown ?? null;
  const accountBreakdown = data?.accountBreakdown ?? null;

  const visibleData = useMemo(() => {
    if (!trends) return [];
    const days = range === '7d' ? 7 : 30;
    return trends.slice(-days);
  }, [trends, range]);

  const selectedDay = useMemo(() => {
    if (!selectedDate) return null;
    return visibleData.find((d) => d.date === selectedDate) ?? null;
  }, [visibleData, selectedDate]);

  const selectedBreakdown = useMemo(() => {
    if (!breakdown || !selectedDate) return null;
    return breakdown.find((b) => b.date === selectedDate) ?? null;
  }, [breakdown, selectedDate]);

  const breakdownByDate = useMemo(
    () => new Map((breakdown ?? []).map((b) => [b.date, b])),
    [breakdown],
  );

  const accountBreakdownByDate = useMemo(
    () => new Map((accountBreakdown ?? []).map((b) => [b.date, b])),
    [accountBreakdown],
  );

  // Which tiers actually show up in the visible range — an all-Sonnet month
  // shouldn't carry a legend entry for Opus.
  const legendKeys = useMemo(() => {
    const present = new Set<string>();
    for (const day of visibleData) {
      for (const m of breakdownByDate.get(day.date)?.models ?? []) {
        present.add(modelKey(m.model));
      }
    }
    return MODEL_ORDER.filter((k) => present.has(k));
  }, [visibleData, breakdownByDate]);

  const periodRows = useMemo(
    () => (trends ? computePeriodComparison(trends, periodGranularity, new Date()) : []),
    [trends, periodGranularity],
  );

  const monthPacing = useMemo(
    () => (trends ? computeMonthPacing(trends, new Date()) : null),
    [trends],
  );

  // Computed over the full fetched history (not just visibleData) so a
  // baseline can reach back before the currently-visible range.
  const anomalies = useMemo(() => (trends ? detectAnomalies(trends) : new Map()), [trends]);

  if (error) {
    return (
      <EmptyState
        icon={<IconTrends size={32} />}
        title="Couldn't load trends"
        description={error}
        action={<Button variant="ghost" size="sm" onClick={reload}>Retry</Button>}
      />
    );
  }
  if (loading || !trends || !breakdown) {
    return <p className="text-[color:var(--color-text-muted)]">Loading…</p>;
  }

  if (trends.length === 0) {
    return (
      <EmptyState
        icon={<IconTrends size={32} />}
        title="No trend data"
        description="Trends will appear after a few days of usage."
      />
    );
  }

  const maxValue = Math.max(
    ...visibleData.map((d) => (metric === 'tokens' ? d.input_tokens + d.output_tokens : d.cost_usd)),
    1,
  );
  const chartHeight = 160;

  async function handleExport() {
    const path = await saveDialog({
      defaultPath: `switchboard-trends-${new Date().toISOString().slice(0, 10)}.csv`,
      filters: [{ name: 'CSV', extensions: ['csv'] }],
    });
    if (typeof path !== 'string') return; // user cancelled
    await ipc.exportTrendsCsv(path, TRENDS_FETCH_DAYS);
  }

  return (
    <div className="flex flex-col gap-[var(--space-md)]">
      {/* Period comparison strip */}
      <div className="flex flex-col gap-[var(--space-sm)]">
        <div className="flex items-center justify-between">
          <span className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text-muted)]">
            vs. previous {periodGranularity}
          </span>
          <div className="flex gap-[var(--space-2xs)] bg-[var(--color-track)] rounded-[var(--radius-sm)] p-[2px] w-fit">
            {(['week', 'month'] as const).map((g) => (
              <button
                key={g}
                type="button"
                onClick={() => setPeriodGranularity(g)}
                className={[
                  'px-[var(--space-sm)] py-[var(--space-2xs)]',
                  'text-[length:var(--text-label)] font-[var(--weight-medium)]',
                  'rounded-[var(--radius-sm)]',
                  'transition-[background,color] duration-[var(--duration-fast)]',
                  periodGranularity === g
                    ? 'bg-[var(--color-bg-card)] text-[color:var(--color-text)]'
                    : 'text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text-secondary)]',
                ].join(' ')}
              >
                {g === 'week' ? 'Week' : 'Month'}
              </button>
            ))}
          </div>
        </div>
        <Card className="p-[var(--space-sm)] flex flex-col gap-[var(--space-xs)]">
          {periodRows.map((row) => (
            <div key={row.metric} className="flex items-center gap-[var(--space-sm)]">
              <span className="text-[length:var(--text-label)] text-[color:var(--color-text-secondary)] w-[64px] shrink-0">
                {PERIOD_METRIC_LABELS[row.metric]}
              </span>
              <svg viewBox="0 0 60 20" preserveAspectRatio="none" className="flex-1 h-[20px]">
                <polyline
                  points={sparklinePoints(row.series)}
                  fill="none"
                  stroke="var(--color-accent)"
                  strokeWidth="2"
                />
              </svg>
              <span
                className={[
                  'mono text-[length:var(--text-label)] tabular-nums min-w-[64px] text-right shrink-0',
                  row.deltaPct === null || row.deltaPct === 0
                    ? 'text-[color:var(--color-text-muted)]'
                    : row.deltaPct > 0
                      ? 'text-[color:var(--color-danger)]'
                      : 'text-[color:var(--color-safe)]',
                ].join(' ')}
              >
                {formatDeltaPct(row.deltaPct)}
              </span>
            </div>
          ))}
        </Card>
      </div>

      {/* Monthly cost pacing */}
      {monthPacing && (
        <Card className="p-[var(--space-sm)] flex flex-col gap-[var(--space-xs)]">
          <div className="w-full h-[10px] rounded-[var(--radius-pill)] bg-[var(--color-track)] overflow-hidden">
            <div
              className="h-full rounded-[var(--radius-pill)] transition-[width] duration-[var(--duration-bar)] ease-[var(--ease-spring)]"
              style={{
                width: `${monthPacing.projectedTotal > 0 ? Math.min(100, (monthPacing.spentSoFar / monthPacing.projectedTotal) * 100) : 0}%`,
                background: 'var(--color-accent)',
              }}
            />
          </div>
          <div className="flex items-center justify-between">
            <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)]">
              {formatCost(monthPacing.spentSoFar)} this month
            </span>
            <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
              → ~${Math.round(monthPacing.projectedTotal)} this month
            </span>
          </div>
        </Card>
      )}

      {/* Range + metric selectors */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-[var(--space-sm)]">
          <div className="flex gap-[var(--space-2xs)] bg-[var(--color-track)] rounded-[var(--radius-sm)] p-[2px] w-fit">
            {(['7d', '30d'] as const).map((r) => (
              <button
                key={r}
                type="button"
                onClick={() => setRange(r)}
                className={[
                  'px-[var(--space-sm)] py-[var(--space-2xs)]',
                  'text-[length:var(--text-label)] font-[var(--weight-medium)]',
                  'rounded-[var(--radius-sm)]',
                  'transition-[background,color] duration-[var(--duration-fast)]',
                  range === r
                    ? 'bg-[var(--color-bg-card)] text-[color:var(--color-text)]'
                    : 'text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text-secondary)]',
                ].join(' ')}
              >
                {r}
              </button>
            ))}
          </div>
          <div className="flex gap-[var(--space-2xs)] bg-[var(--color-track)] rounded-[var(--radius-sm)] p-[2px] w-fit">
            {(['model', 'account'] as const).map((c) => (
              <button
                key={c}
                type="button"
                onClick={() => setColorBy(c)}
                className={[
                  'px-[var(--space-sm)] py-[var(--space-2xs)]',
                  'text-[length:var(--text-label)] font-[var(--weight-medium)]',
                  'rounded-[var(--radius-sm)]',
                  'transition-[background,color] duration-[var(--duration-fast)]',
                  colorBy === c
                    ? 'bg-[var(--color-bg-card)] text-[color:var(--color-text)]'
                    : 'text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text-secondary)]',
                ].join(' ')}
              >
                {c === 'model' ? 'Model' : 'Account'}
              </button>
            ))}
          </div>
        </div>
        <div className="flex items-center gap-[var(--space-sm)]">
          <div className="flex gap-[var(--space-2xs)] bg-[var(--color-track)] rounded-[var(--radius-sm)] p-[2px] w-fit">
            {([
              { key: 'tokens', label: 'Tokens' },
              { key: 'cost', label: 'Cost' },
            ] as const).map((m) => (
              <button
                key={m.key}
                type="button"
                onClick={() => setMetric(m.key)}
                className={[
                  'px-[var(--space-sm)] py-[var(--space-2xs)]',
                  'text-[length:var(--text-label)] font-[var(--weight-medium)]',
                  'rounded-[var(--radius-sm)]',
                  'transition-[background,color] duration-[var(--duration-fast)]',
                  metric === m.key
                    ? 'bg-[var(--color-bg-card)] text-[color:var(--color-text)]'
                    : 'text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text-secondary)]',
                ].join(' ')}
              >
                {m.label}
              </button>
            ))}
          </div>
          <IconButton label="Export CSV" onClick={handleExport}>
            <IconExport size={13} />
          </IconButton>
        </div>
      </div>

      {/* Model/account legend — states what each stacked color means before
          the eye hits the chart. Only tiers present in the visible range are
          shown for the model legend; the account legend lists every known
          account, matching AccountBadge's coloring elsewhere in the app. */}
      {colorBy === 'model' && legendKeys.length > 0 && (
        <div className="flex items-center gap-[var(--space-md)] px-[2px]">
          {legendKeys.map((key) => (
            <span key={key} className="flex items-center gap-[var(--space-2xs)]">
              <span
                aria-hidden
                className="w-[8px] h-[8px] rounded-[2px] shrink-0"
                style={{ background: MODEL_COLORS[key] }}
              />
              <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                {MODEL_LABELS[key]}
              </span>
            </span>
          ))}
        </div>
      )}
      {colorBy === 'account' && accounts.length > 0 && (
        <div className="flex items-center gap-[var(--space-md)] px-[2px]">
          {accounts.map((a) => (
            <AccountBadge key={a.account_uuid} accountUuid={a.account_uuid} accounts={accounts} />
          ))}
        </div>
      )}

      {/* Chart */}
      <Card className="p-[var(--space-md)]">
        <div className="flex items-end gap-[2px]" style={{ height: chartHeight }}>
          {visibleData.map((day) => {
            const total = day.input_tokens + day.output_tokens;
            const value = metric === 'tokens' ? total : day.cost_usd;
            const heightPct = (value / maxValue) * 100;
            const isSelected = day.date === selectedDate;
            const label = `${new Date(day.date).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}: ${formatTokens(total)} tokens, ${formatCost(day.cost_usd)}`;

            // Split this day's bar into one segment per model tier or account
            // (per `colorBy`), sized by that segment's share of the selected
            // metric — for the model tier, position in the stack (fixed by
            // MODEL_ORDER) is what "which model" answers.
            const dayModels = breakdownByDate.get(day.date)?.models ?? [];
            const dayAccounts = accountBreakdownByDate.get(day.date)?.accounts ?? [];
            const segmentValues: Record<string, number> = {};
            const segmentColors: Record<string, string> = {};
            let segments: string[];
            if (colorBy === 'model') {
              for (const m of dayModels) {
                const key = modelKey(m.model);
                const v = metric === 'tokens' ? m.input_tokens + m.output_tokens : m.cost_usd;
                segmentValues[key] = (segmentValues[key] ?? 0) + v;
                segmentColors[key] = MODEL_COLORS[key as (typeof MODEL_ORDER)[number]] ?? MODEL_COLORS.default;
              }
              segments = MODEL_ORDER.filter((k) => (segmentValues[k] ?? 0) > 0);
            } else {
              for (const a of dayAccounts) {
                const key = a.account_uuid ?? 'unknown';
                const v = metric === 'tokens' ? a.input_tokens + a.output_tokens : a.cost_usd;
                segmentValues[key] = (segmentValues[key] ?? 0) + v;
                segmentColors[key] = colorForAccount(a.account_uuid, accounts);
              }
              segments = Object.keys(segmentValues).filter((k) => segmentValues[k] > 0);
            }

            const anomaly = anomalies.get(day.date);
            const anomalyRatio = anomaly
              ? Math.max(anomaly.costRatio ?? 0, anomaly.tokensRatio ?? 0)
              : null;

            return (
              <div
                key={day.date}
                className="flex-1 flex flex-col justify-end group relative"
                style={{ height: '100%' }}
              >
                <button
                  type="button"
                  aria-label={label}
                  aria-pressed={isSelected}
                  onClick={() => setSelectedDate((d) => (d === day.date ? null : day.date))}
                  className="w-full h-full flex flex-col justify-end"
                >
                  <div
                    data-testid={`day-bar-${day.date}`}
                    className={[
                      'w-full rounded-t-[2px] overflow-hidden flex flex-col-reverse',
                      'transition-[height] duration-[var(--duration-normal)]',
                      isSelected
                        ? 'opacity-100 ring-2 ring-[var(--color-border-focus)]'
                        : 'opacity-80 group-hover:opacity-100',
                    ].join(' ')}
                    style={{ height: `${heightPct}%` }}
                  >
                    {segments.map((key) => (
                      <div
                        key={key}
                        data-testid={`day-bar-${day.date}-${key}`}
                        className="w-full"
                        style={{ flexGrow: segmentValues[key], background: segmentColors[key] }}
                      />
                    ))}
                  </div>
                </button>
                {/* Anomaly badge — quiet, not a notification: a runaway session
                    or accidental loop shows up here without interrupting anyone. */}
                {anomaly && (
                  <span
                    data-testid={`day-anomaly-${day.date}`}
                    aria-hidden
                    className="absolute -top-[4px] left-1/2 -translate-x-1/2 w-[6px] h-[6px] rounded-full"
                    style={{ background: 'var(--color-danger)' }}
                  />
                )}
                {/* Tooltip */}
                <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-[var(--space-xs)] hidden group-hover:block z-10">
                  <div className="bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded-[var(--radius-sm)] px-[var(--space-sm)] py-[var(--space-xs)] whitespace-nowrap">
                    <div className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                      {new Date(day.date).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}
                    </div>
                    <div className="mono text-[length:var(--text-label)] text-[color:var(--color-text)]">
                      {metric === 'tokens' ? formatTokens(total) : formatCost(day.cost_usd)}
                    </div>
                    <div className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                      {metric === 'tokens' ? formatCost(day.cost_usd) : `${formatTokens(total)} tokens`}
                    </div>
                    {anomalyRatio !== null && (
                      <div className="mono text-[length:var(--text-micro)] text-[color:var(--color-danger)]">
                        {anomalyRatio.toFixed(1)}x your recent average
                      </div>
                    )}
                  </div>
                </div>
              </div>
            );
          })}
        </div>

        {/* X-axis labels */}
        <div className="flex mt-[var(--space-xs)]">
          {visibleData.map((day, i) => (
            <span
              key={day.date}
              className="flex-1 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] mono"
            >
              {i % (range === '7d' ? 1 : 5) === 0
                ? new Date(day.date).toLocaleDateString('en-US', { day: 'numeric' })
                : null}
            </span>
          ))}
        </div>
      </Card>

      {/* Day breakdown panel */}
      {selectedDay && selectedBreakdown && (
        <Card data-testid="day-breakdown-panel" className="p-[var(--space-md)] flex flex-col gap-[var(--space-sm)]">
          <div className="flex items-center justify-between">
            <span className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text)]">
              {new Date(selectedDay.date).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}
            </span>
            <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)]">
              {formatTokens(selectedDay.input_tokens + selectedDay.output_tokens)} · {formatCost(selectedDay.cost_usd)}
            </span>
          </div>
          <div className="flex flex-col gap-[var(--space-xs)]">
            {selectedBreakdown.models.map((m) => {
              const dayTotal = selectedDay.input_tokens + selectedDay.output_tokens;
              const modelTotal = m.input_tokens + m.output_tokens;
              const pct = dayTotal > 0 ? (modelTotal / dayTotal) * 100 : 0;
              const key = modelKey(m.model);

              return (
                <div key={m.model} className="flex flex-col gap-[var(--space-2xs)]">
                  <div className="flex items-center gap-[var(--space-sm)]">
                    <Badge variant={MODEL_VARIANT[key] ?? 'default'}>{shortName(m.model)}</Badge>
                    <div className="flex-1">
                      <div className="w-full h-[6px] rounded-[var(--radius-pill)] bg-[var(--color-track)] overflow-hidden">
                        <div
                          data-testid={`model-fill-${m.model}`}
                          className="h-full rounded-[var(--radius-pill)] transition-[width] duration-[var(--duration-bar)] ease-[var(--ease-spring)]"
                          style={{
                            width: `${pct}%`,
                            background: MODEL_COLORS[key as (typeof MODEL_ORDER)[number]] ?? MODEL_COLORS.default,
                          }}
                        />
                      </div>
                    </div>
                    <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-muted)] tabular-nums min-w-[48px] text-right">
                      {formatTokens(modelTotal)}
                    </span>
                    <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)] tabular-nums min-w-[48px] text-right">
                      {formatCost(m.cost_usd)}
                    </span>
                  </div>
                  <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] pl-[calc(var(--space-sm)+7px)]">
                    cache: {formatTokens(m.cache_read_tokens)} read · {formatTokens(m.cache_creation_tokens)} created
                  </span>
                </div>
              );
            })}
          </div>
        </Card>
      )}

      {/* Summary */}
      <div className="flex items-center gap-[var(--space-md)] px-[var(--space-2xs)]">
        <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)]">
          Avg {formatTokens(visibleData.reduce((s, d) => s + d.input_tokens + d.output_tokens, 0) / visibleData.length)}
        </span>
        <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">·</span>
        <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)]">
          ${visibleData.reduce((s, d) => s + d.cost_usd, 0).toFixed(2)} total
        </span>
      </div>
    </div>
  );
}
