import type { SessionSummary } from '../lib/generated/bindings';
import { Button } from '../components/ui/Button';
import { ChevronDown, ChevronRight, Play } from '../lib/icons';
import { modelLabel } from '../report/SessionsTab';
import { SessionRecapCard } from './SessionRecapCard';

interface Props {
  session: SessionSummary;
  expanded: boolean;
  onToggle: (id: string) => void;
  onResume: (id: string) => void;
}

function ago(iso: string): string {
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return '';
  const mins = Math.round((Date.now() - t) / 60000);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.round(mins / 60);
  if (hours < 48) return `${hours}h ago`;
  return `${Math.round(hours / 24)}d ago`;
}

export function SessionRow({ session, expanded, onToggle, onResume }: Props) {
  const Chevron = expanded ? ChevronDown : ChevronRight;
  return (
    <div className="rounded-[var(--radius-sm)] border border-[var(--color-border)] bg-[var(--color-bg-card)] px-[var(--space-sm)] py-[var(--space-xs)]">
      <button
        type="button"
        onClick={() => onToggle(session.session_id)}
        aria-expanded={expanded}
        className="flex w-full items-center gap-[var(--space-xs)] text-left"
      >
        <Chevron size={13} aria-hidden className="shrink-0 text-[color:var(--color-text-muted)]" />
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-[length:var(--text-body)] text-[color:var(--color-text)]">
            {session.title}
          </span>
          <span className="truncate text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
            {session.project_name}
            {session.git_branch && ` · ${session.git_branch}`}
            {` · ${session.turns} turn${session.turns === 1 ? '' : 's'}`}
          </span>
        </span>
        <span className="mono shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]">
          {session.model ? modelLabel(session.model) : 'unknown'}
        </span>
        <span className="shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          {ago(session.ended_at)}
        </span>
      </button>

      {expanded && (
        <>
          <SessionRecapCard session={session} />
          <div className="flex justify-end pt-[var(--space-2xs)]">
            <Button
              variant="primary"
              size="sm"
              onClick={() => onResume(session.session_id)}
              aria-label={`Resume ${session.title}`}
            >
              <Play size={13} aria-hidden />
              Resume
            </Button>
          </div>
        </>
      )}
    </div>
  );
}
