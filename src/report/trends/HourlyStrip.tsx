import type { HourTotal, PlannedWindow } from '../../lib/types';
import { formatCost, formatTokens } from '../../lib/format';

/** Windows can wrap past midnight (e.g. 22:00–03:00) — split into two bands
 *  for shading rather than drawing a single range that reads backwards. */
function windowBands(startHour: number, endHour: number): Array<[number, number]> {
  if (endHour > startHour) return [[startHour, endHour]];
  return [
    [startHour, 24],
    [0, endHour],
  ];
}

export function HourlyStrip({
  metric,
  totals,
  windows,
}: {
  metric: 'tokens' | 'cost';
  totals: HourTotal[];
  windows: PlannedWindow[];
}) {
  const value = (t: HourTotal) => (metric === 'tokens' ? t.tokens : t.cost_usd);
  const formatValue = (v: number) => (metric === 'tokens' ? `${formatTokens(v)} tokens` : formatCost(v));
  const max = Math.max(...totals.map(value), metric === 'tokens' ? 1 : 0.01);
  const barHeight = 60;

  return (
    <div>
      <div className="relative" style={{ height: barHeight }}>
        {/* Recommended-window shading, drawn behind the bars */}
        <div className="absolute inset-0 flex" aria-hidden>
          {Array.from({ length: 24 }, (_, hour) => {
            const inWindow = windows.some((w) =>
              windowBands(w.start.hour, w.end.hour).some(([s, e]) => hour >= s && hour < e),
            );
            return (
              <div
                key={hour}
                className="flex-1"
                style={{ background: inWindow ? 'var(--color-accent-muted)' : 'transparent', opacity: 0.3 }}
              />
            );
          })}
        </div>
        <div className="relative flex items-end gap-[1px]" style={{ height: barHeight }}>
          {totals.map((t) => (
            <div
              key={t.hour}
              data-testid={`hour-bar-${metric}-${t.hour}`}
              className="flex-1 rounded-t-[1px] bg-[var(--color-accent)]"
              style={{ height: `${(value(t) / max) * 100}%`, opacity: 0.85 }}
              title={`${String(t.hour).padStart(2, '0')}:00 — ${formatValue(value(t))}`}
            />
          ))}
        </div>
      </div>
      <div className="flex mt-[2px]">
        {totals.map((t) => (
          <span
            key={t.hour}
            className="flex-1 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] mono"
          >
            {t.hour % 3 === 0 ? String(t.hour).padStart(2, '0') : ''}
          </span>
        ))}
      </div>
    </div>
  );
}
