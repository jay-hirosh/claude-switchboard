import { useEffect, useRef, useState } from 'react';
import { ResetCountdown } from '../popover/ResetCountdown';
import { ipc } from '../lib/ipc';
import { formatRelativeTime } from '../lib/format';
import type { AccountListEntry, Schedule } from '../lib/generated/bindings';
import { WarmupToggle } from './WarmupToggle';
import { WarmupNowButton, type WarmupNowStatus } from './WarmupNowButton';
import { ScheduleSelector } from './ScheduleSelector';
import { WarmupConsentModal } from '../components/modals/WarmupConsentModal';
import { MoreHorizontal, Trash2, ChevronRight } from '../lib/icons';
import type { WarmupOutcome } from '../lib/generated/bindings';

interface Props {
  entry: AccountListEntry;
  thresholds: [number, number];
  shareHint?: string | null;
  /** Flags this row as the best account to switch to (most headroom). */
  isBest?: boolean;
  onSwap?: () => void;
  swapBusy?: boolean;
  swapping?: boolean;
  onReauth?: () => void;
  reauthBusy?: boolean;
  onRemove?: () => Promise<void> | void;
}

function describeWarmupOutcome(outcome: WarmupOutcome): { status: WarmupNowStatus; message: string } {
  // Tagged variants come back as `{ OtherFailure: { status } }`; bare variants
  // are plain strings. Specta-generated unions don't include a discriminator
  // helper, so we narrow here.
  if (typeof outcome === 'object' && outcome !== null && 'OtherFailure' in outcome) {
    return { status: 'error', message: `Failed (HTTP ${outcome.OtherFailure.status})` };
  }
  switch (outcome) {
    case 'Success':
      return { status: 'success', message: '5h window refreshed' };
    case 'SkippedAlreadyActive':
      return { status: 'success', message: 'Window already active' };
    case 'NeedsReauth':
      return { status: 'error', message: 'Token expired — re-authenticate' };
    case 'AtRateLimit':
      return { status: 'error', message: 'Already at rate limit' };
    case 'AnthropicServerError':
      return { status: 'error', message: 'Anthropic server error' };
    case 'NetworkError':
      return { status: 'error', message: 'Network error' };
  }
}

function PlanBadge({ plan }: { plan: string | null }) {
  if (!plan) return null;
  const isMax = plan.toLowerCase() === 'max';
  return (
    <span
      className={`
        inline-flex items-center rounded-[var(--radius-pill)]
        px-[var(--space-xs)] py-[1px]
        text-[length:var(--text-micro)] font-[var(--weight-semibold)]
        uppercase tracking-[var(--tracking-label)]
        ${isMax
          ? 'bg-[var(--color-accent-dim)] text-[color:var(--color-accent)]'
          : 'bg-[var(--color-track)] text-[color:var(--color-text-secondary)]'}
      `}
    >
      {plan}
    </span>
  );
}

function BestBadge() {
  return (
    <span
      className="
        shrink-0 inline-flex items-center rounded-[var(--radius-pill)]
        bg-[var(--color-teal-dim)] px-[var(--space-xs)] py-[1px]
        text-[length:var(--text-micro)] font-[var(--weight-semibold)]
        uppercase tracking-[var(--tracking-label)] text-[color:var(--color-teal)]
      "
    >
      Best available
    </span>
  );
}

function CompactMeter({
  label,
  data,
  warnAt,
  dangerAt,
}: {
  label: string;
  data: { utilization: number; resets_at?: string | null } | null;
  warnAt: number;
  dangerAt: number;
}) {
  if (!data) return null;
  const clamped = Math.max(0, Math.min(100, data.utilization));
  const isDanger = clamped >= dangerAt;
  const isWarn = clamped >= warnAt;
  const colorClass = isDanger
    ? 'text-[color:var(--color-danger)]'
    : isWarn
      ? 'text-[color:var(--color-warn)]'
      : 'text-[color:var(--color-text)]';

  const barColor = isDanger
    ? 'bg-[var(--color-danger)]'
    : isWarn
      ? 'bg-[var(--color-warn)]'
      : 'bg-[var(--color-accent)]';

  return (
    <div className="flex flex-col gap-[4px] min-w-0">
      <div className="flex items-baseline justify-between gap-1 min-w-0">
        <span className="text-[length:var(--text-micro)] font-[var(--weight-medium)] text-[color:var(--color-text-muted)] uppercase tracking-[var(--tracking-label)] truncate shrink-0">
          {label}
        </span>
        <span className={`mono text-[length:var(--text-pct)] font-[var(--weight-semibold)] tabular-nums leading-none shrink-0 ${colorClass}`}>
          {Math.round(clamped)}%
        </span>
      </div>
      <div className="h-[var(--bar-height-md)] w-full rounded-[var(--radius-pill)] bg-[var(--color-track)] overflow-hidden">
        <div
          className={`h-full rounded-[var(--radius-pill)] ${barColor} transition-[width,background-color] duration-[var(--duration-bar)] ease-[var(--ease-spring)]`}
          style={{ width: `${clamped}%` }}
        />
      </div>
      <div className="min-h-[14px]">
        {data.resets_at && <ResetCountdown resetsAt={data.resets_at} />}
      </div>
    </div>
  );
}

export function AccountRow({
  entry,
  thresholds,
  shareHint,
  isBest = false,
  onSwap,
  swapBusy = false,
  swapping = false,
  onReauth,
  reauthBusy = false,
  onRemove,
}: Props) {
  const cached = entry.cached_usage;
  const fiveHour = cached?.snapshot.five_hour ?? null;
  const sevenDay = cached?.snapshot.seven_day ?? null;
  const hasUsableData = fiveHour != null || sevenDay != null;

  // Warmup state — fetched once per mount from the DB via get_warmup_state.
  const [warmupEnabled, setWarmupEnabled] = useState(false);
  const [schedule, setSchedule] = useState<Schedule>({ type: 'Off' });
  // Suggested anchor time, derived from local session history — not scoped
  // to this account (session_events has no per-account dimension), so every
  // row fetches and shows the same suggestion. null while loading or when
  // there isn't enough history yet (the chip simply doesn't render).
  const [warmupSuggestion, setWarmupSuggestion] = useState<{
    anchor: { hour: number; minute: number };
    activeDays: number;
  } | null>(null);
  const [showConsent, setShowConsent] = useState(false);
  // The warm-up block is collapsed by default to keep account rows
  // scannable — its state stays visible as a one-line summary.
  const [warmupOpen, setWarmupOpen] = useState(false);

  // "Warm up now" button state — busy flag plus a transient outcome chip that
  // auto-clears after a few seconds so the user can tell the click did
  // something and what the backend reported.
  const [warmupBusy, setWarmupBusy] = useState(false);
  const [warmupStatus, setWarmupStatus] = useState<WarmupNowStatus>('idle');
  const [warmupMessage, setWarmupMessage] = useState<string | null>(null);

  // Overflow menu — open/closed, then a two-step Remove confirmation inside
  // the menu (click "Remove account…" arms, click "Confirm" inside 3s
  // commits, otherwise auto-cancels).
  const [menuOpen, setMenuOpen] = useState(false);
  const [removeArmed, setRemoveArmed] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [removeError, setRemoveError] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    function handlePointerDown(ev: PointerEvent) {
      if (menuRef.current && !menuRef.current.contains(ev.target as Node)) {
        setMenuOpen(false);
        setRemoveArmed(false);
        setRemoveError(null);
      }
    }
    document.addEventListener('pointerdown', handlePointerDown);
    return () => document.removeEventListener('pointerdown', handlePointerDown);
  }, [menuOpen]);

  useEffect(() => {
    if (!removeArmed) return;
    const t = setTimeout(() => setRemoveArmed(false), 3000);
    return () => clearTimeout(t);
  }, [removeArmed]);

  async function handleConfirmRemove() {
    if (!onRemove || removing) return;
    setRemoving(true);
    setRemoveError(null);
    try {
      await onRemove();
      // Parent unmounts this row on success; no need to clear state.
    } catch (e) {
      setRemoving(false);
      setRemoveArmed(false);
      setRemoveError(e instanceof Error ? e.message : 'Remove failed');
    }
  }

  useEffect(() => {
    ipc.getWarmupState(entry.account_uuid).then((ws) => {
      setWarmupEnabled(ws.warmup_enabled);
      setSchedule(ws.schedule);
    }).catch(() => {
      // Silently swallow — row still renders without warmup state.
    });
  }, [entry.account_uuid]);

  useEffect(() => {
    ipc.getWarmupSuggestion().then((s) => {
      setWarmupSuggestion(s ? { anchor: s.anchor, activeDays: s.active_days } : null);
    }).catch(() => {
      // Silently swallow — the suggestion chip is a nice-to-have, not core state.
    });
  }, []);

  const handleToggle = async (next: boolean) => {
    if (next) {
      const granted = await ipc.getWarmupConsentGranted().catch(() => false);
      if (!granted) {
        setShowConsent(true);
        return;
      }
    }
    await ipc.setWarmupEnabled(entry.account_uuid, next).catch(() => {});
    setWarmupEnabled(next);
  };

  const handleConsentAccept = async () => {
    await ipc.grantWarmupConsent().catch(() => {});
    await ipc.setWarmupEnabled(entry.account_uuid, true).catch(() => {});
    setWarmupEnabled(true);
    setShowConsent(false);
  };

  const handleScheduleChange = async (s: Schedule) => {
    await ipc.setAccountSchedule(entry.account_uuid, s).catch(() => {});
    setSchedule(s);
  };

  const handleWarmupNow = async () => {
    if (warmupBusy) return;
    setWarmupBusy(true);
    setWarmupStatus('idle');
    setWarmupMessage(null);
    try {
      const outcome = await ipc.warmupAccountNow(entry.account_uuid);
      const { status, message } = describeWarmupOutcome(outcome);
      setWarmupStatus(status);
      setWarmupMessage(message);
    } catch (e) {
      setWarmupStatus('error');
      setWarmupMessage(e instanceof Error ? e.message : 'Warm up failed');
    } finally {
      setWarmupBusy(false);
    }
  };

  useEffect(() => {
    if (warmupStatus === 'idle') return;
    const t = setTimeout(() => {
      setWarmupStatus('idle');
      setWarmupMessage(null);
    }, 4500);
    return () => clearTimeout(t);
  }, [warmupStatus]);

  const errLabel = (() => {
    if (entry.last_error === 'auth_required')
      return 'token expired — re-authenticate';
    // Transient failures (429, timeout, network) only replace the meters when
    // there's no usable snapshot to fall back on.
    if (entry.last_error && !hasUsableData) return 'usage unavailable';
    return null;
  })();

  // With a transient error but valid cached data, keep showing the meters and
  // flag their age — hiding good 2-minute-old numbers behind "usage
  // unavailable" made rate-limit windows read as total outages.
  const staleHint = (() => {
    if (!entry.last_error || entry.last_error === 'auth_required' || !hasUsableData)
      return null;
    const age = cached?.snapshot.fetched_at
      ? formatRelativeTime(cached.snapshot.fetched_at)
      : '';
    return age
      ? `${entry.last_error} — showing last good data from ${age}`
      : `${entry.last_error} — showing last good data`;
  })();

  // One-line state for the collapsed warm-up disclosure.
  const warmupSummary = !warmupEnabled
    ? 'Off'
    : schedule.type === 'Off'
      ? 'On · manual'
      : schedule.type === 'Every5h'
        ? 'Every 5h'
        : `Custom (${schedule.times.length})`;

  const showSwap = !!onSwap && !entry.is_active;

  return (
    <div
      className={`
        group flex flex-col gap-[var(--space-2xs)]
        border-l-[3px] pl-[calc(var(--popover-pad)-3px)] pr-[var(--popover-pad)] py-[var(--space-sm)]
        ${entry.is_active
          ? 'bg-[var(--color-accent-dim)] border-[var(--color-accent)]'
          : 'border-transparent'}
        ${showSwap ? 'hover:bg-[var(--color-track)] focus-within:bg-[var(--color-track)]' : ''}
      `}
    >
      <div className="flex items-center gap-[var(--space-xs)]">
        <span
          className={`flex-1 min-w-0 truncate text-[length:var(--text-body)] ${
            entry.is_active
              ? 'font-[var(--weight-semibold)] text-[color:var(--color-text)]'
              : 'text-[color:var(--color-text)]'
          }`}
          title={entry.email}
        >
          {entry.email}
        </span>
        {isBest && <BestBadge />}
        <PlanBadge plan={entry.subscription_type ?? null} />
        {entry.is_active && (
          <span
            className="
              shrink-0 inline-flex items-center rounded-[var(--radius-pill)]
              bg-[var(--color-accent)] px-[var(--space-xs)] py-[1px]
              text-[length:var(--text-micro)] font-[var(--weight-semibold)]
              uppercase tracking-[var(--tracking-label)] text-white
            "
          >
            Active
          </span>
        )}
        {showSwap && (
          <button
            type="button"
            onClick={onSwap}
            disabled={swapBusy}
            className={`
              shrink-0 rounded-[var(--radius-pill)]
              border border-[var(--color-border)]
              px-[var(--space-xs)] py-[1px]
              text-[length:var(--text-micro)] uppercase tracking-[var(--tracking-label)]
              text-[color:var(--color-text-secondary)]
              transition-opacity duration-[var(--duration-fast)]
              opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100
              hover:text-[color:var(--color-accent)] hover:border-[color:var(--color-accent)]
              disabled:cursor-not-allowed disabled:hover:text-[color:var(--color-text-muted)] disabled:hover:border-[color:var(--color-border)]
              focus-visible:outline-2 focus-visible:outline-[var(--color-border-focus)] focus-visible:outline-offset-2
            `}
          >
            {swapping ? 'Switching…' : 'Switch account'}
          </button>
        )}
        {onRemove && (
          <div ref={menuRef} className="relative shrink-0">
            <button
              type="button"
              aria-label="More actions"
              aria-haspopup="menu"
              aria-expanded={menuOpen}
              onClick={() => {
                setMenuOpen((v) => !v);
                setRemoveArmed(false);
                setRemoveError(null);
              }}
              disabled={removing}
              className={`
                inline-flex items-center justify-center w-[22px] h-[18px]
                rounded-[var(--radius-sm)]
                text-[color:var(--color-text-muted)]
                transition-[background,color,opacity] duration-[var(--duration-fast)]
                ${menuOpen ? 'opacity-100 bg-[var(--color-bg-card)] text-[color:var(--color-text-secondary)]' : 'opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100'}
                hover:bg-[var(--color-bg-card)] hover:text-[color:var(--color-text-secondary)]
                focus-visible:outline-2 focus-visible:outline-[var(--color-border-focus)] focus-visible:outline-offset-1
                disabled:opacity-30 disabled:pointer-events-none
              `}
            >
              <MoreHorizontal size={13} />
            </button>
            {menuOpen && (
              <div
                role="menu"
                className="
                  absolute right-0 top-[calc(100%+4px)] z-20
                  min-w-[200px]
                  rounded-[var(--radius-md)]
                  border border-[var(--color-border)]
                  bg-[var(--color-bg-elevated)]
                  backdrop-blur-[var(--glass-blur)]
                  shadow-[0_8px_24px_oklch(0%_0_0_/_0.35)]
                  p-[var(--space-2xs)]
                "
              >
                {removeArmed ? (
                  <div className="flex flex-col gap-[var(--space-xs)] p-[var(--space-xs)]">
                    <p className="text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)] leading-[var(--leading-label)]">
                      Remove <span className="text-[color:var(--color-text)]">{entry.email}</span>?
                    </p>
                    {entry.is_active && (
                      <p className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] leading-[var(--leading-label)]">
                        Claude Code stays signed in; this only stops Switchboard from managing it.
                      </p>
                    )}
                    <div className="flex gap-[var(--space-2xs)] pt-[var(--space-2xs)]">
                      <button
                        type="button"
                        onClick={() => {
                          setRemoveArmed(false);
                          setMenuOpen(false);
                        }}
                        disabled={removing}
                        className="
                          flex-1 rounded-[var(--radius-sm)]
                          px-[var(--space-xs)] py-[var(--space-2xs)]
                          text-[length:var(--text-micro)]
                          text-[color:var(--color-text-secondary)]
                          hover:bg-[var(--color-bg-card)]
                          focus-visible:outline-2 focus-visible:outline-[var(--color-border-focus)] focus-visible:outline-offset-1
                        "
                      >
                        Cancel
                      </button>
                      <button
                        type="button"
                        onClick={handleConfirmRemove}
                        disabled={removing}
                        className="
                          flex-1 rounded-[var(--radius-sm)]
                          px-[var(--space-xs)] py-[var(--space-2xs)]
                          text-[length:var(--text-micro)] font-[var(--weight-medium)]
                          text-[color:var(--color-danger)]
                          bg-[var(--color-danger-dim)]
                          hover:bg-[color:var(--color-danger)] hover:text-white
                          disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:bg-[var(--color-danger-dim)] disabled:hover:text-[color:var(--color-danger)]
                          focus-visible:outline-2 focus-visible:outline-[var(--color-border-focus)] focus-visible:outline-offset-1
                        "
                      >
                        {removing ? 'Removing…' : 'Remove'}
                      </button>
                    </div>
                  </div>
                ) : (
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => setRemoveArmed(true)}
                    className="
                      w-full flex items-center gap-[var(--space-xs)]
                      rounded-[var(--radius-sm)]
                      px-[var(--space-xs)] py-[var(--space-xs)]
                      text-left text-[length:var(--text-label)]
                      text-[color:var(--color-danger)]
                      hover:bg-[var(--color-danger-dim)]
                      focus-visible:outline-2 focus-visible:outline-[var(--color-border-focus)] focus-visible:outline-offset-1
                    "
                  >
                    <Trash2 size={12} />
                    Remove account…
                  </button>
                )}
                {removeError && (
                  <p className="px-[var(--space-xs)] pt-[var(--space-xs)] text-[length:var(--text-micro)] text-[color:var(--color-danger)]">
                    {removeError}
                  </p>
                )}
              </div>
            )}
          </div>
        )}
      </div>

      {errLabel ? (
        <div className="flex items-center gap-[var(--space-xs)]">
          <span className="flex-1 text-[length:var(--text-micro)] text-[color:var(--color-warn)]">
            {errLabel}
          </span>
          {entry.last_error === 'auth_required' && onReauth && (
            <button
              type="button"
              onClick={onReauth}
              disabled={reauthBusy}
              className="
                shrink-0 rounded-[var(--radius-pill)]
                border border-[color:var(--color-warn)]
                px-[var(--space-xs)] py-[1px]
                text-[length:var(--text-micro)] uppercase tracking-[var(--tracking-label)]
                text-[color:var(--color-warn)]
                hover:bg-[color:var(--color-warn)] hover:text-white
                disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:bg-transparent disabled:hover:text-[color:var(--color-warn)]
                focus-visible:outline-2 focus-visible:outline-[var(--color-border-focus)] focus-visible:outline-offset-2
              "
            >
              {reauthBusy ? 'Opening browser…' : 'Re-authenticate'}
            </button>
          )}
        </div>
      ) : (
        <div className="flex flex-col gap-[var(--space-2xs)]">
          <div className="grid grid-cols-2 gap-[var(--space-xl)]">
            <CompactMeter
              label="5h"
              data={fiveHour}
              warnAt={thresholds[0]}
              dangerAt={thresholds[1]}
            />
            <CompactMeter
              label="7d"
              data={sevenDay}
              warnAt={thresholds[0]}
              dangerAt={thresholds[1]}
            />
          </div>
          {staleHint && (
            <span className="text-[length:var(--text-micro)] text-[color:var(--color-warn)]">
              {staleHint}
            </span>
          )}
          {shareHint && (
            <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
              shares quota with {shareHint}
            </span>
          )}
        </div>
      )}

      {/* Warm-up — collapsed to a one-line summary by default; expands to
          the full controls on click. The summary keeps the configured state
          glanceable without the section eating vertical space in every row. */}
      <div className="border-t border-neutral-700/40 pt-2 mt-1">
        <button
          type="button"
          onClick={() => setWarmupOpen((v) => !v)}
          aria-expanded={warmupOpen}
          className="flex w-full items-center justify-between gap-2"
        >
          <span className="text-[11px] text-neutral-400 uppercase tracking-wide">
            Warm-up
          </span>
          <span className="flex items-center gap-1.5 text-[11px] text-neutral-500">
            {warmupSummary}
            <ChevronRight
              size={12}
              className={`transition-transform duration-[var(--duration-fast)] ${warmupOpen ? 'rotate-90' : ''}`}
            />
          </span>
        </button>
        {warmupOpen && (
          <div className="mt-2 space-y-2">
            <div className="flex items-center justify-between">
              <span className="text-[11px] text-neutral-400 uppercase tracking-wide">
                Enabled
              </span>
              <WarmupToggle enabled={warmupEnabled} onToggle={handleToggle} />
            </div>
            {warmupEnabled && (
              <>
                <ScheduleSelector
                  value={schedule}
                  onChange={handleScheduleChange}
                  suggestion={warmupSuggestion}
                />
                <div className="flex items-center justify-end gap-[var(--space-xs)]">
                  {warmupMessage && (
                    <span
                      className={`text-[length:var(--text-micro)] ${
                        warmupStatus === 'error'
                          ? 'text-[color:var(--color-danger)]'
                          : 'text-[color:var(--color-safe)]'
                      }`}
                    >
                      {warmupMessage}
                    </span>
                  )}
                  <div className="relative group/warmup">
                    <WarmupNowButton
                      enabled={true}
                      busy={warmupBusy}
                      status={warmupStatus}
                      onClick={handleWarmupNow}
                    />
                    {/* Opens upward so the explanation stays within the popover's
                        scroll area even when the row is near the bottom. */}
                    <div
                      role="tooltip"
                      className="
                        pointer-events-none opacity-0
                        group-hover/warmup:opacity-100 group-focus-within/warmup:opacity-100
                        transition-opacity duration-[var(--duration-fast)] ease-[var(--ease-out)]
                        absolute right-0 bottom-[calc(100%+6px)] z-30
                        w-[240px]
                        rounded-[var(--radius-md)]
                        border border-[var(--color-border)]
                        bg-[var(--color-bg-elevated)]
                        backdrop-blur-[var(--glass-blur)]
                        shadow-[0_8px_24px_oklch(0%_0_0_/_0.35)]
                        p-[var(--space-sm)]
                        text-left text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]
                        leading-[var(--leading-label)]
                      "
                    >
                      <p className="font-[var(--weight-semibold)] text-[color:var(--color-text)] mb-[var(--space-2xs)]">
                        What “warm up” does
                      </p>
                      <p>
                        Sends a 1-token <span className="mono text-[color:var(--color-text)]">“hi”</span> to{' '}
                        <span className="mono text-[color:var(--color-text)]">claude-haiku-4-5</span> — Anthropic's
                        cheapest model. That single call opens a fresh 5-hour
                        usage window for this account, at a fraction of a cent.
                      </p>
                    </div>
                  </div>
                </div>
              </>
            )}
          </div>
        )}
      </div>

      {showConsent && (
        <WarmupConsentModal
          onAccept={handleConsentAccept}
          onDismiss={() => setShowConsent(false)}
        />
      )}
    </div>
  );
}
