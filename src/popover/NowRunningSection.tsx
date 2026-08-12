import type { LiveSessionInfo } from '../lib/types';
import { formatCost } from '../lib/format';
import { formatDurationMinutes } from '../lib/format';
import { modelLabel } from '../report/modelDisplay';

const MAX_ROWS = 3;

function elapsedLabel(firstSeen: number): string {
  const mins = Math.max(0, Math.floor((Date.now() / 1000 - firstSeen) / 60));
  return formatDurationMinutes(mins) || '<1m';
}

function Row({ session }: { session: LiveSessionInfo }) {
  return (
    <div className="flex items-center gap-[var(--space-xs)] px-[var(--popover-pad)] py-[var(--space-2xs)]">
      <span className="flex-1 min-w-0 truncate text-[length:var(--text-micro)] text-[color:var(--color-text)]">
        {session.project}
      </span>
      <span className="shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] uppercase">
        {modelLabel(session.model)}
      </span>
      <span className="mono shrink-0 text-[length:var(--text-micro)] tabular-nums text-[color:var(--color-text-secondary)]">
        {formatCost(session.total_cost_usd)}
      </span>
      <span className="mono shrink-0 text-[length:var(--text-micro)] tabular-nums text-[color:var(--color-text-muted)]">
        {elapsedLabel(session.first_seen)}
      </span>
    </div>
  );
}

export function NowRunningSection({ sessions }: { sessions: LiveSessionInfo[] }) {
  if (sessions.length === 0) return null;
  const shown = sessions.slice(0, MAX_ROWS);
  const overflow = sessions.length - shown.length;
  return (
    <div className="flex flex-col gap-[var(--space-2xs)] border-t border-[var(--color-rule)] py-[var(--space-2xs)]">
      <span className="px-[var(--popover-pad)] text-[length:var(--text-micro)] font-[var(--weight-semibold)] text-[color:var(--color-text-muted)] uppercase tracking-[var(--tracking-label)]">
        Now running
      </span>
      {shown.map((s) => (
        <Row key={s.session_id} session={s} />
      ))}
      {overflow > 0 && (
        <span className="px-[var(--popover-pad)] text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          +{overflow} more
        </span>
      )}
    </div>
  );
}
