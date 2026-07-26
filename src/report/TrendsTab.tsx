import { useEffect, useMemo, useState } from 'react';
import { Card } from '../components/ui/Card';
import { Badge } from '../components/ui/Badge';
import { Button } from '../components/ui/Button';
import { EmptyState } from '../components/ui/EmptyState';
import { formatTokens, formatCost } from '../lib/format';
import { IconTrends } from '../lib/icons';
import { ipc } from '../lib/ipc';
import { useTabData } from '../lib/useTabData';
import { useAppStore } from '../lib/store';
import { MODEL_VARIANT, modelKey, shortName } from './modelDisplay';

export function TrendsTab() {
  const version = useAppStore((s) => s.sessionDataVersion);
  const { data, error, loading, reload } = useTabData(
    () =>
      Promise.all([ipc.getDailyTrends(30), ipc.getDailyModelBreakdown(30)]).then(
        ([trends, breakdown]) => ({ trends, breakdown }),
      ),
    [version],
  );
  const [range, setRange] = useState<'7d' | '30d'>('30d');
  const [selectedDate, setSelectedDate] = useState<string | null>(null);

  useEffect(() => {
    setSelectedDate(null);
  }, [range]);

  const trends = data?.trends ?? null;
  const breakdown = data?.breakdown ?? null;

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
    ...visibleData.map((d) => d.input_tokens + d.output_tokens),
    1,
  );
  const chartHeight = 160;

  return (
    <div className="flex flex-col gap-[var(--space-md)]">
      {/* Range selector */}
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

      {/* Chart */}
      <Card className="p-[var(--space-md)]">
        <div className="flex items-end gap-[2px]" style={{ height: chartHeight }}>
          {visibleData.map((day) => {
            const total = day.input_tokens + day.output_tokens;
            const heightPct = (total / maxValue) * 100;
            const isDanger = day.cost_usd >= 3;
            const isWarn = day.cost_usd >= 1.5 && !isDanger;
            const isSelected = day.date === selectedDate;
            const label = `${new Date(day.date).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}: ${formatTokens(total)} tokens, ${formatCost(day.cost_usd)}`;

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
                      'w-full rounded-t-[2px] transition-[height,background-color] duration-[var(--duration-normal)]',
                      isDanger
                        ? 'bg-[var(--color-danger)]'
                        : isWarn
                          ? 'bg-[var(--color-warn)]'
                          : 'bg-[var(--color-accent)]',
                      isSelected
                        ? 'opacity-100 ring-2 ring-[var(--color-border-focus)]'
                        : 'opacity-80 group-hover:opacity-100',
                    ].join(' ')}
                    style={{ height: `${heightPct}%` }}
                  />
                </button>
                {/* Tooltip */}
                <div className="absolute bottom-full left-1/2 -translate-x-1/2 mb-[var(--space-xs)] hidden group-hover:block z-10">
                  <div className="bg-[var(--color-bg-elevated)] border border-[var(--color-border)] rounded-[var(--radius-sm)] px-[var(--space-sm)] py-[var(--space-xs)] whitespace-nowrap">
                    <div className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                      {new Date(day.date).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}
                    </div>
                    <div className="mono text-[length:var(--text-label)] text-[color:var(--color-text)]">
                      {formatTokens(total)}
                    </div>
                    <div className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                      ${day.cost_usd.toFixed(2)}
                    </div>
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
                            background:
                              key === 'opus'
                                ? 'var(--color-model-opus)'
                                : key === 'sonnet'
                                  ? 'var(--color-model-sonnet)'
                                  : 'var(--color-model-haiku)',
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
