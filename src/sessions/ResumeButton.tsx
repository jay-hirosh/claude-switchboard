import type { LaunchSurface } from '../lib/generated/bindings';
import { Button } from '../components/ui/Button';
import { Play, SquareTerminal, SquareCode } from '../lib/icons';

interface Props {
  /** Session title, so each option's accessible name identifies what resumes. */
  label: string;
  /** Set when the project folder is gone — resume is impossible by any route. */
  disabledReason?: string;
  /** False when the `code` CLI or the Claude Code extension is missing. */
  vsCodeAvailable: boolean;
  onResume: (surface: LaunchSurface) => void;
}

/** The two surfaces, in the order they appear. Terminal is first because it is
 *  the path that carries everything — a VS Code tab cannot take the provider's
 *  CLI flags or the session's permission mode. */
const OPTIONS = [
  {
    surface: 'terminal' as LaunchSurface,
    Icon: SquareTerminal,
    what: 'a terminal',
    hint: 'Resume in a terminal window — carries the provider’s flags and permission mode',
  },
  {
    surface: 'vs_code_tab' as LaunchSurface,
    Icon: SquareCode,
    what: 'a VS Code tab',
    hint: 'Resume as a Claude Code tab in a new VS Code window — the provider’s credentials carry, but its CLI flags and permission mode do not',
  },
] as const;

/**
 * One Resume button that splits into a terminal option and a VS Code option on
 * hover or keyboard focus.
 *
 * Both real buttons are always in the DOM, and the collapsed "Resume" face is
 * `aria-hidden` decoration stacked on top of them. Rendering the face as a third
 * button would make the control announce three actions where there are two, and
 * would let a click land on a label rather than a choice.
 */
export function ResumeButton({ label, disabledReason, vsCodeAvailable, onResume }: Props) {
  // A dead end stays a single button: there is nothing to choose between when
  // neither surface can work, and a control that expands into two disabled
  // options invites the click it is about to refuse.
  if (disabledReason) {
    return (
      <Button variant="primary" size="sm" disabled aria-label={`Resume ${label}`} title={disabledReason}>
        <Play size={12} aria-hidden />
        Resume
      </Button>
    );
  }

  return (
    <span className="group relative inline-grid">
      <span
        aria-hidden
        className="
          pointer-events-none z-10 [grid-area:1/1]
          inline-flex items-center justify-center gap-[var(--space-xs)]
          rounded-[var(--radius-sm)] bg-[var(--color-accent)]
          px-[var(--space-sm)] py-[var(--space-2xs)]
          text-[length:var(--text-label)] font-[var(--weight-medium)]
          text-[color:var(--color-bg-base)] select-none
          transition-opacity duration-[var(--duration-fast)] ease-[var(--ease-out)]
          group-hover:opacity-0 group-focus-within:opacity-0
        "
      >
        <Play size={12} />
        Resume
      </span>

      <span
        className="
          [grid-area:1/1]
          inline-flex items-stretch overflow-hidden
          rounded-[var(--radius-sm)] bg-[var(--color-accent)]
          opacity-0 transition-opacity duration-[var(--duration-fast)] ease-[var(--ease-out)]
          group-hover:opacity-100 group-focus-within:opacity-100
        "
      >
        {OPTIONS.map(({ surface, Icon, what, hint }, i) => {
          const unavailable = surface === 'vs_code_tab' && !vsCodeAvailable;
          return (
            <button
              key={surface}
              type="button"
              disabled={unavailable}
              onClick={() => onResume(surface)}
              aria-label={`Resume ${label} in ${what}`}
              title={
                unavailable
                  ? 'VS Code is unavailable — needs the `code` command on PATH and the Claude Code extension installed'
                  : hint
              }
              className={[
                'inline-flex items-center justify-center',
                'px-[var(--space-sm)] py-[var(--space-2xs)]',
                'text-[color:var(--color-bg-base)]',
                'transition-[filter,opacity] duration-[var(--duration-fast)] ease-[var(--ease-out)]',
                'hover:brightness-110 active:brightness-95',
                'focus-visible:outline-2 focus-visible:outline-[var(--color-border-focus)] focus-visible:outline-offset-1',
                'disabled:opacity-40',
                // A hairline between the two halves, so the split reads as two
                // targets rather than one wide button.
                i > 0 ? 'border-l border-[color:var(--color-bg-base)]/25' : '',
              ].join(' ')}
            >
              <Icon size={13} aria-hidden />
            </button>
          );
        })}
      </span>
    </span>
  );
}
