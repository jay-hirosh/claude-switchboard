import type { SessionSummary } from '../lib/generated/bindings';
import { ModelBadge } from '../components/ui/ModelBadge';
import { ChevronDown, ChevronRight } from '../lib/icons';
import { SessionRecapCard } from './SessionRecapCard';

interface Props {
  session: SessionSummary;
  expanded: boolean;
  /** Grouped mode repeats the project on every row for no gain — the group
   *  header already says it. Search flattens the grouping, so there it is
   *  the one piece of context a result is missing. */
  showProject?: boolean;
  onToggle: (id: string) => void;
  onResume: (id: string) => void;
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
  onToggle,
  onResume,
}: Props) {
  const Chevron = expanded ? ChevronDown : ChevronRight;
  const meta = [
    showProject ? session.project_name : null,
    session.git_branch,
    `${session.turns} turn${session.turns === 1 ? '' : 's'}`,
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
        {/* Fixed width + right alignment so the age column reads as a column
            rather than drifting with each badge's width. */}
        <span
          className="
            mono shrink-0 min-w-[34px] text-right tabular-nums
            text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]
          "
        >
          {ago(session.ended_at)}
        </span>
      </button>

      {expanded && <SessionRecapCard session={session} onResume={onResume} />}
    </div>
  );
}
