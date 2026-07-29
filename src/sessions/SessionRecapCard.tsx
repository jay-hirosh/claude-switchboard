import type { ReactNode } from 'react';
import type { SessionSummary } from '../lib/generated/bindings';
import { Button } from '../components/ui/Button';
import { Play, ShieldOff } from '../lib/icons';
import { contextReadout } from './contextWindow';

/** The CLI's mode names, in the words the app uses everywhere else. */
const MODE_LABELS: Record<string, string> = {
  bypassPermissions: 'bypass permissions',
  acceptEdits: 'accept edits',
  dontAsk: "don't ask",
  plan: 'plan mode',
  auto: 'auto',
  manual: 'manual',
};

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
  const context = session.peak_context_tokens
    ? contextReadout(session.peak_context_tokens, session.model)
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
              title={context.hint}
            >
              {/* No bar without a denominator — a fill needs something to be a
                  fraction of, and a third-party window is provider-set and
                  unrecorded. The token count still stands on its own. */}
              {context.pct !== null && (
                <span
                  aria-hidden
                  className="h-[3px] w-[28px] overflow-hidden rounded-[var(--radius-pill)] bg-[var(--color-track)]"
                >
                  <span
                    className="block h-full rounded-[var(--radius-pill)] bg-[var(--color-accent)]"
                    style={{ width: `${context.pct}%` }}
                  />
                </span>
              )}
              <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
                {context.text}
              </span>
            </span>
          )}
        </span>
        {/* Mode and button travel together as one right-hand cluster —
            justify-between would otherwise strand the mode in mid-row. */}
        <span className="flex shrink-0 items-center gap-[var(--space-sm)]">
        {/* Resuming re-applies the mode the session ended under. Skipping
            straight past a permission prompt is exactly what the user asked
            for, but it must never be a surprise — so the mode is stated next
            to the button that will apply it, and bypass gets danger colour
            because it is the one that removes a safety check. */}
        {session.permission_mode && (
          <span
            className={[
              'flex shrink-0 items-center gap-[var(--space-2xs)]',
              'text-[length:var(--text-micro)]',
              session.permission_mode === 'bypassPermissions'
                ? 'text-[color:var(--color-danger)]'
                : 'text-[color:var(--color-text-muted)]',
            ].join(' ')}
            title={`Resumes with --permission-mode ${session.permission_mode}, matching how this session ran`}
          >
            {session.permission_mode === 'bypassPermissions' && (
              <ShieldOff size={11} aria-hidden />
            )}
            {MODE_LABELS[session.permission_mode] ?? session.permission_mode}
          </span>
        )}
        {/* Resuming is a `cd` into session.cwd, and Claude Code finds a
            transcript only via the directory it is launched in — so a deleted
            project folder makes resume impossible by any route, not merely
            inconvenient. Disabled with the reason attached beats a button that
            opens a terminal only to fail inside it. */}
        <Button
          variant="primary"
          size="sm"
          disabled={!session.cwd_exists}
          onClick={() => onResume(session.session_id)}
          aria-label={`Resume ${session.title}`}
          title={
            session.cwd_exists
              ? undefined
              : `Cannot resume — the project folder no longer exists: ${session.cwd}`
          }
        >
          <Play size={12} aria-hidden />
          Resume
        </Button>
        </span>
      </div>
    </div>
  );
}
