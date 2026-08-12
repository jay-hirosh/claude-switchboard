import type { LaunchSurface, SessionSummary } from '../lib/generated/bindings';
import { ModelBadge } from '../components/ui/ModelBadge';
import { ChevronDown, ChevronRight } from '../lib/icons';
import { formatCost, formatDuration, formatTokens } from '../lib/format';
import { SessionRecapCard } from './SessionRecapCard';

interface Props {
  session: SessionSummary;
  expanded: boolean;
  /** Grouped mode repeats the project on every row for no gain — the group
   *  header already says it. Search flattens the grouping, so there it is
   *  the one piece of context a result is missing. */
  showProject?: boolean;
  /** Probed once by the tab; forwarded so the card can disable the VS Code
   *  half of Resume with a reason instead of failing after the click. */
  vsCodeAvailable?: boolean;
  onToggle: (id: string) => void;
  onResume: (id: string, surface: LaunchSurface) => void;
}

function ago(iso: string): string {
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return '';
  const mins = Math.round((Date.now() - t) / 60000);
  if (mins < 60) return `${mins}m`;
  const hours = Math.round(mins / 60);
  if (hours < 48) return `${hours}h`;
  return `${Math.round(hours / 24)}d`;
}

export function SessionRow({
  session,
  expanded,
  showProject = false,
  vsCodeAvailable = false,
  onToggle,
  onResume,
}: Props) {
  const Chevron = expanded ? ChevronDown : ChevronRight;
  const meta = [
    showProject ? session.project_name : null,
    session.git_branch,
    `${session.turns} turn${session.turns === 1 ? '' : 's'}`,
    formatDuration(session.started_at, session.ended_at),
  ]
    .filter(Boolean)
    .join(' · ');

  return (
    <div className={expanded ? 'bg-[var(--color-bg-card-hover)]' : ''}>
      <button
        type="button"
        onClick={() => onToggle(session.session_id)}
        aria-expanded={expanded}
        className="
          flex w-full items-center gap-[var(--space-sm)]
          px-[var(--space-md)] py-[var(--space-sm)] text-left
          transition-[background] duration-[var(--duration-fast)] ease-[var(--ease-out)]
          hover:bg-[var(--color-bg-card-hover)]
          focus-visible:outline-2 focus-visible:outline-[var(--color-border-focus)]
          focus-visible:-outline-offset-2
        "
      >
        <Chevron
          size={13}
          aria-hidden
          className="shrink-0 text-[color:var(--color-text-muted)]"
        />
        <span className="flex min-w-0 flex-1 flex-col">
          <span
            className="
              truncate text-[length:var(--text-body)] leading-[var(--leading-title)]
              font-[var(--weight-medium)] text-[color:var(--color-text)]
            "
          >
            {session.title}
          </span>
          <span className="truncate text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
            {meta}
          </span>
        </span>
        <ModelBadge model={session.model} />
        {/* Lifetime totals for the whole conversation, subagents included —
            one number each, because a session can switch models partway and a
            per-model split would not fit a browse row. Fixed widths keep all
            three trailing values reading as columns rather than drifting with
            each badge's width. A session with no ingested usage renders blank
            rather than "0 $0.00", which would read as a measurement. */}
        <span
          className="
            mono shrink-0 min-w-[52px] text-right tabular-nums
            text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]
          "
        >
          {session.total_tokens > 0 ? formatTokens(session.total_tokens) : ''}
        </span>
        <span
          className="
            mono shrink-0 min-w-[46px] text-right tabular-nums
            text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]
          "
        >
          {session.total_tokens > 0 ? formatCost(session.total_cost_usd) : ''}
        </span>
        <span
          className="
            mono shrink-0 min-w-[34px] text-right tabular-nums
            text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]
          "
        >
          {ago(session.ended_at)}
        </span>
      </button>

      {expanded && (
        <SessionRecapCard
          session={session}
          vsCodeAvailable={vsCodeAvailable}
          onResume={onResume}
        />
      )}
    </div>
  );
}
