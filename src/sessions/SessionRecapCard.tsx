import type { ReactNode } from 'react';
import type { SessionSummary } from '../lib/generated/bindings';
import { Button } from '../components/ui/Button';
import { Play } from '../lib/icons';

function span(startIso: string, endIso: string): string {
  const a = new Date(startIso).getTime();
  const b = new Date(endIso).getTime();
  if (!Number.isFinite(a) || !Number.isFinite(b) || b <= a) return '';
  const mins = Math.round((b - a) / 60000);
  if (mins < 60) return `${mins}m`;
  return `${Math.floor(mins / 60)}h${String(mins % 60).padStart(2, '0')}m`;
}

function endedAt(iso: string): string {
  const d = new Date(iso);
  if (!Number.isFinite(d.getTime())) return '';
  return d.toLocaleString('en-US', {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

/** One label/value pair in the detail grid. The label column is a fixed width
 *  so the three values start on the same x — a ragged left edge here was the
 *  main reason the card read as a text dump. */
function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex gap-[var(--space-sm)]">
      <span
        className="
          shrink-0 w-[62px] pt-[1px]
          text-[length:var(--text-micro)] font-[var(--weight-medium)]
          uppercase tracking-[var(--tracking-label)]
          text-[color:var(--color-text-muted)]
        "
      >
        {label}
      </span>
      <span className="min-w-0 flex-1">{children}</span>
    </div>
  );
}

export function SessionRecapCard({
  session,
  onResume,
}: {
  session: SessionSummary;
  onResume: (id: string) => void;
}) {
  const duration = span(session.started_at, session.ended_at);
  const ended = endedAt(session.ended_at);

  return (
    <div
      className="
        flex flex-col gap-[var(--space-sm)]
        border-t border-[var(--color-border-subtle)]
        px-[var(--space-md)] pt-[var(--space-sm)] pb-[var(--space-sm)]
      "
    >
      {/* Claude Code's own end-of-session summary is the single most
          identifying thing in a transcript, so it gets its own tinted block
          instead of competing as one more row in the grid. */}
      {session.recap && (
        <div
          className="
            rounded-[var(--radius-md)] bg-[var(--color-accent-dim)]
            px-[var(--space-sm)] py-[var(--space-xs)]
          "
        >
          <div
            className="
              pb-[2px]
              text-[length:var(--text-micro)] font-[var(--weight-semibold)]
              uppercase tracking-[var(--tracking-label)]
              text-[color:var(--color-accent)]
            "
          >
            Recap
          </div>
          <p
            className="
              text-[length:var(--text-label)] leading-[var(--leading-body)]
              text-[color:var(--color-text)]
            "
          >
            {session.recap}
          </p>
        </div>
      )}

      <div className="flex flex-col gap-[var(--space-xs)]">
        <Field label="Asked">
          <p
            className="
              text-[length:var(--text-label)] leading-[var(--leading-body)]
              text-[color:var(--color-text-secondary)]
            "
          >
            {session.asked}
          </p>
        </Field>

        {session.left_off && (
          <Field label="Left off">
            <p
              className="
                text-[length:var(--text-label)] leading-[var(--leading-body)]
                text-[color:var(--color-text-secondary)]
              "
            >
              {session.left_off}
            </p>
          </Field>
        )}

        {session.touched_files.length > 0 && (
          <Field label="Touched">
            <span className="flex flex-wrap items-center gap-[var(--space-2xs)]">
              {session.touched_files.map((f) => (
                <span
                  key={f}
                  /* --color-track, not --color-bg-card: the card background
                     is what these chips sit on, so a bg-card chip was
                     literally invisible. */
                  className="
                    mono rounded-[var(--radius-sm)] bg-[var(--color-track)]
                    px-[var(--space-2xs)] py-[1px]
                    text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]
                  "
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
          </Field>
        )}
      </div>

      <div className="flex items-center justify-between gap-[var(--space-sm)]">
        <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
          {[ended, duration && `${duration} elapsed`].filter(Boolean).join(' · ')}
        </span>
        <Button
          variant="primary"
          size="sm"
          onClick={() => onResume(session.session_id)}
          aria-label={`Resume ${session.title}`}
        >
          <Play size={12} aria-hidden />
          Resume
        </Button>
      </div>
    </div>
  );
}
