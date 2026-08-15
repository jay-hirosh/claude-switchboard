import { useMemo, useState } from 'react';
import { EmptyState } from '../components/ui/EmptyState';
import { Button } from '../components/ui/Button';
import type { DailyAccountBucket, HeatmapCell, SessionEvent } from '../lib/types';
import { formatTokens } from '../lib/format';
import { IconHeatmap } from '../lib/icons';
import { ipc } from '../lib/ipc';
import { useTabData } from '../lib/useTabData';
import { useAppStore } from '../lib/store';
import { colorForAccount, labelForAccount } from './accountDisplay';

const CELL_SIZE = 11;
const CELL_GAP = 3;
const CELL_STEP = CELL_SIZE + CELL_GAP;

const MONTH_LABELS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

function sessionsToHeatmap(events: SessionEvent[]): HeatmapCell[] {
  const byDate = new Map<string, number>();
  for (const e of events) {
    const date = new Date(e.ts).toISOString().slice(0, 10);
    byDate.set(date, (byDate.get(date) ?? 0) + e.input_tokens + e.output_tokens);
  }
  const cells: HeatmapCell[] = [];
  const values = Array.from(byDate.values());
  const maxVal = Math.max(...values, 1);
  for (let i = 180; i >= 0; i--) {
    const d = new Date();
    d.setDate(d.getDate() - i);
    const key = d.toISOString().slice(0, 10);
    const value = byDate.get(key) ?? 0;
    const ratio = value / maxVal;
    const level = ratio === 0 ? 0 : ratio < 0.25 ? 1 : ratio < 0.5 ? 2 : ratio < 0.75 ? 3 : 4;
    cells.push({ date: key, value, level: level as 0 | 1 | 2 | 3 | 4 });
  }
  return cells;
}

function getMonthPositions(cells: HeatmapCell[]): { label: string; x: number }[] {
  const seen = new Set<string>();
  const months: { label: string; x: number }[] = [];
  // Monday-first: shift Sunday (0) to position 6, Mon (1) to 0, etc.
  const startDay = (new Date(cells[0]?.date ?? '').getDay() + 6) % 7;

  cells.forEach((cell, idx) => {
    const d = new Date(cell.date);
    const key = `${d.getFullYear()}-${d.getMonth()}`;
    if (!seen.has(key)) {
      seen.add(key);
      const col = Math.floor((idx + startDay) / 7);
      months.push({ label: MONTH_LABELS[d.getMonth()], x: col * CELL_STEP });
    }
  });
  return months;
}

const levelColors: Record<number, string> = {
  0: 'var(--color-track)',
  1: 'var(--color-accent-muted)',
  2: 'var(--color-accent)',
  3: 'var(--color-warn)',
  4: 'var(--color-danger)',
};

export function HeatmapTab() {
  const version = useAppStore((s) => s.sessionDataVersion);
  const accounts = useAppStore((s) => s.accounts);
  const { data: events, error, loading, reload } = useTabData(
    () => ipc.getSessionHistory(180),
    [version],
  );
  const { data: accountBuckets } = useTabData(
    () => ipc.getDailyAccountBreakdown(180),
    [version],
  );
  const accountsByDate = useMemo(
    () => new Map((accountBuckets ?? []).map((b: DailyAccountBucket) => [b.date, b.accounts])),
    [accountBuckets],
  );

  const data = useMemo(() => (events ? sessionsToHeatmap(events) : []), [events]);
  const [hovered, setHovered] = useState<string | null>(null);

  const startDay = useMemo(() => {
    const d = new Date(data[0]?.date ?? '');
    return (d.getDay() + 6) % 7;
  }, [data]);

  const totalDays = data.length;
  const weeks = Math.ceil((totalDays + startDay) / 7);
  const svgWidth = weeks * CELL_STEP + 30;
  const svgHeight = 7 * CELL_STEP + 20;

  const monthPositions = useMemo(() => getMonthPositions(data), [data]);

  if (error) {
    return (
      <EmptyState
        icon={<IconHeatmap size={32} />}
        title="Couldn't load heatmap"
        description={error}
        action={<Button variant="ghost" size="sm" onClick={reload}>Retry</Button>}
      />
    );
  }

  if (loading || !events) {
    return <p className="text-[color:var(--color-text-muted)]">Loading…</p>;
  }

  if (data.length === 0) {
    return (
      <EmptyState
        icon={<IconHeatmap size={32} />}
        title="No heatmap data"
        description="Usage activity will appear here over time."
      />
    );
  }

  const totalValue = data.reduce((s, c) => s + c.value, 0);
  const DAY_LABELS = ['Mon', '', 'Wed', '', 'Fri', '', ''];

  return (
    <div className="flex flex-col gap-[var(--space-md)]">
      {/* Legend */}
      <div className="flex items-center justify-between px-[var(--space-2xs)]">
        <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">
          Last 6 months
        </span>
        <div className="flex items-center gap-[var(--space-2xs)]">
          <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">Less</span>
          {[0, 1, 2, 3, 4].map((level) => (
            <div
              key={level}
              className="w-[10px] h-[10px] rounded-[2px]"
              style={{ background: levelColors[level] }}
            />
          ))}
          <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">More</span>
        </div>
      </div>

      {/* Heatmap grid */}
      <div className="overflow-x-auto">
        <svg width={svgWidth} height={svgHeight}>
          {/* Month labels */}
          {monthPositions.map((m, i) => (
            <text
              key={i}
              x={30 + m.x}
              y={10}
              className="mono"
              style={{ fontSize: 9, fill: 'var(--color-text-muted)' }}
            >
              {m.label}
            </text>
          ))}

          {/* Day labels */}
          {DAY_LABELS.map((label, i) =>
            label ? (
              <text
                key={i}
                x={24}
                y={18 + i * CELL_STEP + CELL_SIZE / 2 + 3}
                textAnchor="end"
                className="mono"
                style={{ fontSize: 9, fill: 'var(--color-text-muted)' }}
              >
                {label}
              </text>
            ) : null,
          )}

          {/* Cells */}
          {data.map((cell, i) => {
            const col = Math.floor((i + startDay) / 7);
            const row = (i + startDay) % 7;
            const x = 30 + col * CELL_STEP;
            const y = 18 + row * CELL_STEP;

            return (
              <g key={cell.date}>
                <rect
                  data-testid={`heatmap-cell-${cell.date}`}
                  x={x}
                  y={y}
                  width={CELL_SIZE}
                  height={CELL_SIZE}
                  rx={2}
                  fill={levelColors[cell.level]}
                  opacity={hovered === cell.date ? 1 : 0.75}
                  onMouseEnter={() => setHovered(cell.date)}
                  onMouseLeave={() => setHovered(null)}
                  className="cursor-pointer transition-opacity"
                />
                {(() => {
                  const dayAccounts = accountsByDate.get(cell.date) ?? [];
                  if (dayAccounts.length === 0) return null;
                  const dominant = [...dayAccounts].sort(
                    (a, b) => (b.input_tokens + b.output_tokens) - (a.input_tokens + a.output_tokens),
                  )[0];
                  return (
                    <rect
                      x={x - 1}
                      y={y - 1}
                      width={CELL_SIZE + 2}
                      height={CELL_SIZE + 2}
                      rx={2}
                      fill="none"
                      stroke={colorForAccount(dominant.account_uuid, accounts)}
                      strokeWidth={1}
                      pointerEvents="none"
                    />
                  );
                })()}
                {hovered === cell.date && (
                  <g>
                    <rect
                      x={x - 2}
                      y={y - 2}
                      width={CELL_SIZE + 4}
                      height={CELL_SIZE + 4}
                      rx={3}
                      fill="none"
                      stroke="var(--color-text)"
                      strokeWidth={1.5}
                    />
                    <text
                      x={x + CELL_SIZE / 2}
                      y={y - 6}
                      textAnchor="middle"
                      className="mono"
                      style={{ fontSize: 9, fill: 'var(--color-text-secondary)' }}
                    >
                      {new Date(cell.date).toLocaleDateString('en-US', { month: 'short', day: 'numeric' })}
                    </text>
                    {(() => {
                      const dayAccounts = accountsByDate.get(cell.date) ?? [];
                      const total = dayAccounts.reduce((s, a) => s + a.input_tokens + a.output_tokens, 0);
                      if (total === 0) return null;
                      const parts = dayAccounts
                        .map((a) => `${labelForAccount(a.account_uuid, accounts)} ${Math.round(((a.input_tokens + a.output_tokens) / total) * 100)}%`)
                        .join(' · ');
                      return (
                        <text
                          x={x + CELL_SIZE / 2}
                          y={y - 16}
                          textAnchor="middle"
                          className="mono"
                          style={{ fontSize: 8, fill: 'var(--color-text-muted)' }}
                        >
                          {parts}
                        </text>
                      );
                    })()}
                  </g>
                )}
              </g>
            );
          })}
        </svg>
      </div>

      {/* Summary */}
      <div className="flex items-center gap-[var(--space-md)] px-[var(--space-2xs)]">
        <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)]">
          {data.filter((c) => c.level > 0).length} active days
        </span>
        <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">
          {formatTokens(totalValue)} total
        </span>
      </div>
    </div>
  );
}
