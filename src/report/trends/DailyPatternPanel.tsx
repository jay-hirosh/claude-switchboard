import { useEffect, useMemo, useState } from 'react';
import { Card } from '../../components/ui/Card';
import { Button } from '../../components/ui/Button';
import { EmptyState } from '../../components/ui/EmptyState';
import { formatTokens } from '../../lib/format';
import { IconTimer } from '../../lib/icons';
import { ipc } from '../../lib/ipc';
import { useTabData } from '../../lib/useTabData';
import { useAppStore } from '../../lib/store';
import { HourlyStrip } from './HourlyStrip';
import type { AccountListEntry, HhMm, Schedule } from '../../lib/generated/bindings';
import type { HourCell, WarmupPlan } from '../../lib/types';

const LOOKBACK_OPTIONS = ['today', 7, 14, 30, 90] as const;
type Lookback = (typeof LOOKBACK_OPTIONS)[number];

const CELL_W = 14;
const CELL_H = 14;
const CELL_GAP = 2;
const CELL_STEP_X = CELL_W + CELL_GAP;
const CELL_STEP_Y = CELL_H + CELL_GAP;
const GRID_LEFT = 30;
const GRID_TOP = 18;

const WEEKDAY_LABELS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const HOUR_TICKS = [0, 3, 6, 9, 12, 15, 18, 21];

// Same 5-level ramp as HeatmapTab so a color means the same intensity in
// both grids.
const levelColors: Record<0 | 1 | 2 | 3 | 4, string> = {
  0: 'var(--color-track)',
  1: 'var(--color-accent-muted)',
  2: 'var(--color-accent)',
  3: 'var(--color-warn)',
  4: 'var(--color-danger)',
};

function cellLevel(value: number, max: number): 0 | 1 | 2 | 3 | 4 {
  if (max <= 0 || value <= 0) return 0;
  const ratio = value / max;
  return ratio < 0.25 ? 1 : ratio < 0.5 ? 2 : ratio < 0.75 ? 3 : 4;
}

function fmtHm(h: HhMm): string {
  return `${String(h.hour).padStart(2, '0')}:${String(h.minute).padStart(2, '0')}`;
}

export function DailyPatternPanel() {
  const version = useAppStore((s) => s.sessionDataVersion);
  const accounts = useAppStore((s) => s.accounts);
  const [days, setDays] = useState<Lookback>(30);
  const {
    data: report,
    error,
    loading,
    reload,
  } = useTabData(
    () => (days === 'today' ? ipc.getTodayPattern() : ipc.getDailyPattern(days)),
    [version, days],
  );

  const [hovered, setHovered] = useState<HourCell | null>(null);
  const [selectedAccount, setSelectedAccount] = useState<string>('');
  const [applyStatus, setApplyStatus] = useState<'idle' | 'busy' | 'success' | 'error'>('idle');

  // Default the picker to the active account once accounts load, without
  // clobbering a choice the user already made.
  useEffect(() => {
    if (selectedAccount) return;
    const active = accounts.find((a) => a.is_active) ?? accounts[0];
    if (active) setSelectedAccount(active.account_uuid);
  }, [accounts, selectedAccount]);

  useEffect(() => {
    if (applyStatus === 'idle' || applyStatus === 'busy') return;
    const t = setTimeout(() => setApplyStatus('idle'), 4500);
    return () => clearTimeout(t);
  }, [applyStatus]);

  const maxCell = useMemo(
    () => Math.max(...(report?.cells.map((c) => c.tokens) ?? [0]), 1),
    [report],
  );

  if (error) {
    return (
      <EmptyState
        icon={<IconTimer size={32} />}
        title="Couldn't load daily pattern"
        description={error}
        action={<Button variant="ghost" size="sm" onClick={reload}>Retry</Button>}
      />
    );
  }
  if (loading || !report) {
    return <p className="text-[color:var(--color-text-muted)]">Loading…</p>;
  }
  if (report.active_days === 0) {
    return (
      <EmptyState
        icon={<IconTimer size={32} />}
        title={days === 'today' ? 'No activity yet today' : 'No pattern data yet'}
        description={
          days === 'today'
            ? "Today's hour-by-hour usage will appear here once you start a session."
            : 'Your hour-of-day usage pattern will appear here after a few days of activity.'
        }
      />
    );
  }

  const svgWidth = GRID_LEFT + 24 * CELL_STEP_X;
  const svgHeight = GRID_TOP + 7 * CELL_STEP_Y;

  async function handleApply() {
    if (!report?.plan || !selectedAccount) return;
    setApplyStatus('busy');
    try {
      const schedule: Schedule = { type: 'Every5h', anchor: report.plan.anchor };
      await ipc.setAccountSchedule(selectedAccount, schedule);
      setApplyStatus('success');
    } catch {
      setApplyStatus('error');
    }
  }

  return (
    <div className="flex flex-col gap-[var(--space-md)]">
      {/* Lookback selector */}
      <div className="flex items-center justify-between">
        <span className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text-muted)]">
          {days === 'today'
            ? 'Active today'
            : `${report.active_days} active day${report.active_days === 1 ? '' : 's'} of ${report.lookback_days}`}
        </span>
        <div className="flex gap-[var(--space-2xs)] bg-[var(--color-track)] rounded-[var(--radius-sm)] p-[2px] w-fit">
          {LOOKBACK_OPTIONS.map((d) => (
            <button
              key={d}
              type="button"
              onClick={() => setDays(d)}
              className={[
                'px-[var(--space-sm)] py-[var(--space-2xs)]',
                'text-[length:var(--text-label)] font-[var(--weight-medium)]',
                'rounded-[var(--radius-sm)]',
                'transition-[background,color] duration-[var(--duration-fast)]',
                days === d
                  ? 'bg-[var(--color-bg-card)] text-[color:var(--color-text)]'
                  : 'text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text-secondary)]',
              ].join(' ')}
            >
              {d === 'today' ? 'Today' : `${d}d`}
            </button>
          ))}
        </div>
      </div>

      {/* Hour × weekday grid */}
      <Card className="p-[var(--space-md)] flex flex-col gap-[var(--space-xs)]">
        <div className="overflow-x-auto">
          <svg width={svgWidth} height={svgHeight}>
            {HOUR_TICKS.map((h) => (
              <text
                key={h}
                x={GRID_LEFT + h * CELL_STEP_X}
                y={10}
                className="mono"
                style={{ fontSize: 9, fill: 'var(--color-text-muted)' }}
              >
                {String(h).padStart(2, '0')}
              </text>
            ))}
            {WEEKDAY_LABELS.map((label, i) => (
              <text
                key={label}
                x={GRID_LEFT - 6}
                y={GRID_TOP + i * CELL_STEP_Y + CELL_H / 2 + 3}
                textAnchor="end"
                className="mono"
                style={{ fontSize: 9, fill: 'var(--color-text-muted)' }}
              >
                {label}
              </text>
            ))}
            {report.cells.map((cell) => {
              const x = GRID_LEFT + cell.hour * CELL_STEP_X;
              const y = GRID_TOP + cell.weekday * CELL_STEP_Y;
              const isHovered = hovered?.weekday === cell.weekday && hovered?.hour === cell.hour;
              return (
                <rect
                  key={`${cell.weekday}-${cell.hour}`}
                  data-testid={`pattern-cell-${cell.weekday}-${cell.hour}`}
                  x={x}
                  y={y}
                  width={CELL_W}
                  height={CELL_H}
                  rx={2}
                  fill={levelColors[cellLevel(cell.tokens, maxCell)]}
                  opacity={isHovered ? 1 : 0.8}
                  onMouseEnter={() => setHovered(cell)}
                  onMouseLeave={() => setHovered(null)}
                  className="cursor-pointer transition-opacity"
                />
              );
            })}
          </svg>
        </div>
        {/* Shared readout line — a per-cell floating tooltip would clip
            against the grid's tight 16px row pitch, so hover state surfaces
            here instead of hovering above the cell the way HeatmapTab does. */}
        <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] h-[14px]">
          {hovered
            ? `${WEEKDAY_LABELS[hovered.weekday]} ${String(hovered.hour).padStart(2, '0')}:00 — ${formatTokens(hovered.tokens)} tokens`
            : 'Hover a cell for details'}
        </span>
      </Card>

      {/* Hourly profiles, with the recommended windows shaded behind them */}
      <Card className="p-[var(--space-md)] flex flex-col gap-[var(--space-xs)]">
        <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">
          Tokens by hour of day
        </span>
        <HourlyStrip metric="tokens" totals={report.hourly_totals} windows={report.plan?.windows ?? []} />
      </Card>

      <Card className="p-[var(--space-md)] flex flex-col gap-[var(--space-xs)]">
        <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">
          Cost by hour of day
        </span>
        <HourlyStrip metric="cost" totals={report.hourly_totals} windows={report.plan?.windows ?? []} />
      </Card>

      {/* Recommendation */}
      {report.plan ? (
        <PlanCard
          plan={report.plan}
          accounts={accounts}
          selectedAccount={selectedAccount}
          onSelectAccount={setSelectedAccount}
          applyStatus={applyStatus}
          onApply={handleApply}
        />
      ) : (
        <p className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)] px-[var(--space-2xs)]">
          Need 5 active days to recommend an anchor — {report.active_days} so far.
        </p>
      )}
    </div>
  );
}

function PlanCard({
  plan,
  accounts,
  selectedAccount,
  onSelectAccount,
  applyStatus,
  onApply,
}: {
  plan: WarmupPlan;
  accounts: AccountListEntry[];
  selectedAccount: string;
  onSelectAccount: (uuid: string) => void;
  applyStatus: 'idle' | 'busy' | 'success' | 'error';
  onApply: () => void;
}) {
  const baselinePct = Math.round(plan.baseline_peak_share * 100);
  const recommendedPct = Math.round(plan.recommended_peak_share * 100);

  return (
    <Card className="p-[var(--space-md)] flex flex-col gap-[var(--space-sm)]">
      <span className="text-[length:var(--text-body)] font-[var(--weight-medium)] text-[color:var(--color-text)]">
        Anchor at {fmtHm(plan.anchor)} — busiest window drops from {baselinePct}% to {recommendedPct}% of a
        typical day
      </span>

      <div className="flex flex-col gap-[var(--space-2xs)]">
        {plan.windows.map((w, i) => (
          <div key={i} className="flex items-center gap-[var(--space-sm)]">
            <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)] w-[104px] shrink-0">
              {fmtHm(w.start)}–{fmtHm(w.end)}
            </span>
            <div className="flex-1 h-[6px] rounded-[var(--radius-pill)] bg-[var(--color-track)] overflow-hidden">
              <div
                className="h-full rounded-[var(--radius-pill)]"
                style={{ width: `${Math.round(w.share * 100)}%`, background: 'var(--color-accent)' }}
              />
            </div>
            <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-muted)] w-[36px] text-right">
              {Math.round(w.share * 100)}%
            </span>
          </div>
        ))}
      </div>

      <div className="flex items-center gap-[var(--space-sm)] pt-[var(--space-2xs)] border-t border-[var(--color-border)]">
        <select
          aria-label="Account to apply the schedule to"
          value={selectedAccount}
          onChange={(e) => onSelectAccount(e.target.value)}
          className="flex-1 bg-[var(--color-track)] text-[color:var(--color-text)] text-[length:var(--text-label)] rounded-[var(--radius-sm)] px-[var(--space-sm)] py-[var(--space-2xs)]"
        >
          {accounts.length === 0 && <option value="">No accounts</option>}
          {accounts.map((a) => (
            <option key={a.account_uuid} value={a.account_uuid}>
              {a.email}
            </option>
          ))}
        </select>
        <Button variant="primary" size="sm" disabled={!selectedAccount || applyStatus === 'busy'} onClick={onApply}>
          Apply
        </Button>
        {applyStatus === 'success' && (
          <span className="text-[length:var(--text-label)] text-[color:var(--color-safe)]">Applied</span>
        )}
        {applyStatus === 'error' && (
          <span className="text-[length:var(--text-label)] text-[color:var(--color-danger)]">Failed</span>
        )}
      </div>
      <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
        Warm-up must be enabled for the chosen account, and Switchboard must be running for scheduled fires to
        happen.
      </span>
    </Card>
  );
}
