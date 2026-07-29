import type { ReactNode } from 'react';
import type { SessionSummary } from '../lib/generated/bindings';
import { Button } from '../components/ui/Button';
import { formatTokens } from '../lib/format';
import { Play } from '../lib/icons';

function span(startIso: string, endIso: string): string {
  const a = new Date(startIso).getTime();
  const b = new Date(endIso).getTime();
  if (!Number.isFinite(a) || !Number.isFinite(b) || b <= a) return '';
  const mins = Math.round((b - a) / 60000);
  if (mins < 60) return `${mins}m`;
  return `${Math.floor(mins / 60)}h${String(mins % 60).padStart(2, '0')}m`;
}

/** The standard context window. A session whose peak exceeded it must have
 *  been running on a 1M window — Claude Code strips the `[1m]` suffix before
 *  writing the transcript, so exceeding this is the only surviving evidence. */
const STANDARD_WINDOW = 200_000;

/** Peak context as "410.0K of 1M · 41%".
 *
 *  The window is inferred, not read: the transcript records the model as
 *  `claude-opus-5` whether or not it ran with `[1m]`. Anything past the
 *  standard window is proof of a 1M one; at or below it, 200K is the only
 *  defensible assumption, so the percentage is presented as an upper bound
 *  rather than a fact — a 1M session that only reached 50K would otherwise
 *  read as 25% full when it was really 5%. */
function contextLabel(peak: number): { text: string; pct: number; inferred1m: boolean } {
  const inferred1m = peak > STANDARD_WINDOW;
  const window = inferred1m ? 1_000_000 : STANDARD_WINDOW;
  return {
    text: `${formatTokens(peak)} of ${inferred1m ? '1M' : '200K'}`,
    pct: Math.min(100, Math.round((peak / window) * 100)),
    inferred1m,
  };
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
  const context = session.peak_context_tokens
    ? contextLabel(session.peak_context_tokens)
    : null;

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
        <span className="flex min-w-0 items-center gap-[var(--space-sm)]">
          <span className="mono shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
            {[ended, duration && `${duration} elapsed`].filter(Boolean).join(' · ')}
          </span>
          {context && (
            <span
              className="flex shrink-0 items-center gap-[var(--space-2xs)]"
              title={
                context.inferred1m
                  ? 'Peak context. Exceeded the 200K standard window, so this session ran on a 1M one.'
                  : 'Peak context, against the standard 200K window. A 1M session that stayed under 200K is indistinguishable in the transcript.'
              }
            >
              <span
                aria-hidden
                className="h-[3px] w-[28px] overflow-hidden rounded-[var(--radius-pill)] bg-[var(--color-track)]"
              >
                <span
                  className="block h-full rounded-[var(--radius-pill)] bg-[var(--color-accent)]"
                  style={{ width: `${context.pct}%` }}
                />
              </span>
              <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
                {context.text}
              </span>
            </span>
          )}
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
