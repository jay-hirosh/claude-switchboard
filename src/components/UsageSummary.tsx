/**
 * UsageSummary — the hero usage readout extracted from CompactPopover so both
 * compact and expanded views can render it. Shows 5h/7d instrument columns,
 * burn rate projection, Opus/Sonnet sub-rows, and pay-as-you-go.
 *
 * When `condensed` is true the layout is tightened for embedding at the top
 * of the expanded report — same data, less vertical real-estate.
 */

import { motion } from 'framer-motion';
import { InstrumentColumn, InstrumentRow } from '../popover/InstrumentRow';
import { ChevronRight } from '../lib/icons';
import { formatCents } from '../lib/format';
import type { BurnRateProjection, CachedUsage, ExtraBurnRate, Utilization } from '../lib/types';

interface UsageSummaryProps {
  usage: CachedUsage;
  thresholds: [number, number];
  /** Tighter layout for embedding at the top of the expanded report. */
  condensed?: boolean;
  /** Highlights the matching Opus/Sonnet sub-row as currently in use. */
  activeModel?: 'opus' | 'sonnet' | 'haiku' | null;
  /** When true, the model-split / pay-as-you-go detail rows collapse behind a
   * disclosure row (compact popover default). Omit for always-expanded. */
  collapsible?: boolean;
  detailsOpen?: boolean;
  onToggleDetails?: () => void;
}

export function UsageSummary({
  usage,
  thresholds,
  condensed,
  activeModel,
  collapsible,
  detailsOpen,
  onToggleDetails,
}: UsageSummaryProps) {
  const snap = usage.snapshot;
  const extra = snap.extra_usage;
  const [warn, danger] = thresholds;
  // Per-model 7-day utilization is a Max-plan-only field — the usage API
  // returns null for both opus and sonnet on Pro / Team-Pro plans (they have
  // a single combined 7-day quota, no per-model split). Render the rows only
  // when at least one bucket actually carries data, so they never sit empty
  // on plans that don't provide it.
  const hasModelSplit =
    snap.seven_day_opus != null || snap.seven_day_sonnet != null;
  const showModelSplit =
    hasModelSplit && snap.seven_day != null && (!collapsible || detailsOpen);
  const showExtra = extra?.is_enabled && (!collapsible || detailsOpen);

  return (
    <div className={condensed ? 'flex flex-col gap-[var(--space-xs)]' : 'flex flex-col'}>
      {/* Hero: two-column instrument readout — tighter padding in the
          collapsed glance view so the shorter popover height still fits the
          footer. */}
      <motion.div
        initial={{ opacity: 0, y: 6 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.32, ease: [0.22, 1, 0.36, 1] }}
        className={
          condensed
            ? 'grid grid-cols-2 gap-x-[var(--space-lg)] px-[var(--popover-pad)] py-[var(--space-sm)]'
            : `grid grid-cols-2 gap-x-[var(--space-lg)] px-[var(--popover-pad)] ${
                collapsible && !detailsOpen
                  ? 'pt-[var(--space-xs)] pb-[var(--space-sm)]'
                  : 'pt-[var(--space-md)] pb-[var(--space-lg)]'
              }`
        }
      >
        <div className="flex flex-col gap-[var(--space-xs)]">
          <InstrumentColumn
            label="5h"
            data={snap.five_hour}
            warnAt={warn}
            dangerAt={danger}
          />
          <BurnRateCaption
            burnRate={usage.burn_rate}
            warnAt={warn}
            dangerAt={danger}
          />
        </div>
        <InstrumentColumn
          label="7d"
          data={snap.seven_day}
          warnAt={warn}
          dangerAt={danger}
        />
      </motion.div>

      {collapsible && (
        <>
          <Hairline />
          <button
            type="button"
            onClick={onToggleDetails}
            aria-expanded={detailsOpen}
            className="flex items-center justify-between px-[var(--popover-pad)] py-[var(--space-xs)]"
          >
            <span className="text-[length:var(--text-micro)] font-[var(--weight-medium)] tracking-[var(--tracking-label)] uppercase text-[color:var(--color-text-muted)]">
              Details
            </span>
            <ChevronRight
              size={12}
              className={`text-[color:var(--color-text-muted)] transition-transform duration-[var(--duration-fast)] ${detailsOpen ? 'rotate-90' : ''}`}
            />
          </button>
        </>
      )}

      {/* Opus / Sonnet sub-row — rendered whenever 7d data is present so the
       * model split stays visible even when one bucket is idle. The currently-
       * in-use family (derived from local session events) is highlighted. */}
      {showModelSplit && (
        <>
          <Hairline />
          <div className="grid grid-cols-2 gap-x-[var(--space-lg)] px-[var(--popover-pad)] py-[var(--space-sm)]">
            <InstrumentRow
              label="Opus"
              data={snap.seven_day_opus}
              warnAt={warn}
              dangerAt={danger}
              active={activeModel === 'opus'}
            />
            <InstrumentRow
              label="Sonnet"
              data={snap.seven_day_sonnet}
              warnAt={warn}
              dangerAt={danger}
              active={activeModel === 'sonnet'}
            />
          </div>
        </>
      )}

      {/* Pay-as-you-go — its own row, hairline-divided */}
      {showExtra && extra && (
        <>
          <Hairline />
          <div className="px-[var(--popover-pad)] py-[var(--space-sm)]">
            <ExtraRow
              pct={extra.utilization ?? 0}
              resetsAt={extra.resets_at ?? null}
              usedCents={extra.used_credits_cents ?? 0}
              limitCents={extra.monthly_limit_cents ?? 0}
              burn={usage.extra_burn_rate ?? null}
              warnAt={warn}
              dangerAt={danger}
            />
          </div>
        </>
      )}
    </div>
  );
}

/* ───────────────────────── Private helpers ───────────────────────── */

/**
 * Tiny caption beneath the 5h instrument that answers "should I keep
 * coding?" — extrapolates current usage slope to the window reset and
 * shows the projected % at reset. Color-cued against the same warn/danger
 * thresholds the meter uses. Hidden when there's no projection yet
 * (cold start) or when the projection is essentially flat.
 */
function BurnRateCaption({
  burnRate,
  warnAt,
  dangerAt,
}: {
  burnRate: BurnRateProjection | null | undefined;
  warnAt: number;
  dangerAt: number;
}) {
  if (!burnRate) return null;
  // Hide jitter under ~0.1%/min — anything that small extrapolates to a
  // ≤6% delta over a full 5h window, which isn't actionable signal.
  if (Math.abs(burnRate.utilization_per_min) < 0.1) return null;

  const projected = Math.max(0, burnRate.projected_at_reset);
  const color =
    projected >= dangerAt
      ? 'var(--color-danger)'
      : projected >= warnAt
        ? 'var(--color-warn)'
        : 'var(--color-text-muted)';

  return (
    <span
      className="text-[length:var(--text-micro)] tabular-nums"
      style={{ color }}
      title={`${burnRate.utilization_per_min >= 0 ? '+' : ''}${burnRate.utilization_per_min.toFixed(2)}%/min`}
    >
      → ~{Math.round(projected)}% by reset
    </span>
  );
}

function Hairline() {
  return <div className="mx-[var(--popover-pad)] border-t border-[var(--color-rule)]" />;
}

function ExtraRow({
  pct,
  resetsAt,
  usedCents,
  limitCents,
  burn,
  warnAt,
  dangerAt,
}: {
  pct: number;
  resetsAt: string | null;
  usedCents: number;
  limitCents: number;
  burn: ExtraBurnRate | null;
  warnAt: number;
  dangerAt: number;
}) {
  const data: Utilization | null = resetsAt
    ? { utilization: pct, resets_at: resetsAt }
    : null;
  // Hide a flat forecast: under one cent a day extrapolates to nothing.
  const showForecast =
    burn != null &&
    burn.projected_cents_at_reset != null &&
    Math.abs(burn.cents_per_min) * 1440 >= 1;
  return (
    <div className="flex flex-col gap-[var(--space-2xs)]">
      <InstrumentRow
        label="Pay-as-you-go"
        caption={resetsAt ? undefined : 'no reset window'}
        value={pct}
        data={data}
        warnAt={warnAt}
        dangerAt={dangerAt}
      />
      {(limitCents > 0 || showForecast) && (
        <div className="flex items-baseline justify-between gap-[var(--space-xs)]">
          {limitCents > 0 ? (
            <span className="mono text-[length:var(--text-micro)] tabular-nums text-[color:var(--color-text-secondary)]">
              {formatCents(usedCents)} of {formatCents(limitCents)}
            </span>
          ) : (
            <span />
          )}
          {showForecast && (
            <span
              className="text-[length:var(--text-micro)] tabular-nums text-[color:var(--color-text-muted)]"
              title={`${burn.cents_per_min >= 0 ? '+' : ''}${(burn.cents_per_min * 14.4).toFixed(2)} $/day`}
            >
              → ~${Math.round((burn.projected_cents_at_reset as number) / 100)} by reset
            </span>
          )}
        </div>
      )}
    </div>
  );
}
