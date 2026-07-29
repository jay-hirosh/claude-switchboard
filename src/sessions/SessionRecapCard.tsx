import type { SessionSummary } from '../lib/generated/bindings';

function span(startIso: string, endIso: string): string {
  const a = new Date(startIso).getTime();
  const b = new Date(endIso).getTime();
  if (!Number.isFinite(a) || !Number.isFinite(b) || b <= a) return '';
  const mins = Math.round((b - a) / 60000);
  if (mins < 60) return `${mins}m`;
  return `${Math.floor(mins / 60)}h${String(mins % 60).padStart(2, '0')}m`;
}

const labelClass =
  'shrink-0 w-[58px] text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]';

export function SessionRecapCard({ session }: { session: SessionSummary }) {
  const duration = span(session.started_at, session.ended_at);
  return (
    <div className="flex flex-col gap-[var(--space-2xs)] pt-[var(--space-2xs)]">
      <div className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
        {session.turns} turn{session.turns === 1 ? '' : 's'}
        {duration && ` over ${duration}`}
      </div>

      {session.recap && (
        <div className="flex gap-[var(--space-xs)]">
          <span className={labelClass}>Recap</span>
          <span className="flex-1 text-[length:var(--text-micro)] text-[color:var(--color-text)]">
            {session.recap}
          </span>
        </div>
      )}

      <div className="flex gap-[var(--space-xs)]">
        <span className={labelClass}>Asked</span>
        <span className="flex-1 text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]">
          {session.asked}
        </span>
      </div>

      {session.left_off && (
        <div className="flex gap-[var(--space-xs)]">
          <span className={labelClass}>Left off</span>
          <span className="flex-1 text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]">
            {session.left_off}
          </span>
        </div>
      )}

      {session.touched_files.length > 0 && (
        <div className="flex gap-[var(--space-xs)]">
          <span className={labelClass}>Touched</span>
          <span className="flex flex-1 flex-wrap gap-[var(--space-2xs)]">
            {session.touched_files.map((f) => (
              <span
                key={f}
                className="mono rounded-[var(--radius-sm)] bg-[var(--color-bg-card)] px-[var(--space-2xs)] text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]"
              >
                {f}
              </span>
            ))}
            {session.touched_overflow > 0 && (
              <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                +{session.touched_overflow} more
              </span>
            )}
          </span>
        </div>
      )}
    </div>
  );
}
