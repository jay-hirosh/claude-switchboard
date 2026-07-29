import { useState } from 'react';
import type { Provider, SessionSummary } from '../lib/generated/bindings';
import { ModalShell } from '../components/modals/ModalShell';
import { Button } from '../components/ui/Button';
import { SelectChevron } from '../components/ui/SelectChevron';
import { labelClass, selectClass } from '../components/ui/field';
import { IconWarning } from '../lib/icons';

interface Props {
  session: SessionSummary;
  providers: Provider[];
  onCancel: () => void;
  onConfirm: (providerId: string) => void;
}

export function ResumeProviderPicker({ session, providers, onCancel, onConfirm }: Props) {
  const [choice, setChoice] = useState(providers[0]?.id ?? '');
  const chosen = providers.find((p) => p.id === choice);
  const recorded = session.model ?? 'unknown';
  const differs = Boolean(session.model) && chosen?.env['ANTHROPIC_MODEL'] !== session.model;

  return (
    <ModalShell id="resume-picker" title="Which provider?" onDismiss={onCancel}>
      {/* ModalShell renders children raw, so the padding has to come from
          here. Without it the body sat flush against the modal border while
          the title strip kept its own inset — the two left edges disagreed,
          which is most of why this dialog read as unfinished. */}
      <div className="flex flex-col gap-[var(--space-md)] px-[var(--space-md)] py-[var(--space-md)]">
        <p
          className="
            text-[length:var(--text-label)] leading-[var(--leading-body)]
            text-[color:var(--color-text-secondary)]
          "
        >
          This session ran on{' '}
          <span
            className="
              mono rounded-[var(--radius-sm)] bg-[var(--color-track)]
              px-[var(--space-2xs)] py-[1px] text-[color:var(--color-text)]
            "
          >
            {recorded}
          </span>
          , which doesn’t match any provider you’ve configured.
        </p>

        <label className="flex flex-col gap-[var(--space-2xs)]">
          <span className={labelClass}>Resume with</span>
          <span className="relative block">
            <select
              aria-label="Provider"
              value={choice}
              onChange={(e) => setChoice(e.target.value)}
              className={selectClass}
            >
              {providers.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
            <SelectChevron />
          </span>
        </label>

        {differs && (
          /* Not a <Banner>: that lays its icon out with `items-center`, which
             parks it halfway down a four-line warning. Same tokens, top
             alignment. */
          <div
            role="alert"
            className="
              flex items-start gap-[var(--space-sm)]
              rounded-[var(--radius-sm)] border border-[var(--color-warn)]/20
              bg-[var(--color-warn-dim)] px-[var(--space-sm)] py-[var(--space-xs)]
            "
          >
            <IconWarning
              size={13}
              aria-hidden
              className="mt-[2px] shrink-0 text-[color:var(--color-warn)]"
            />
            <p
              className="
                text-[length:var(--text-micro)] leading-[var(--leading-body)]
                text-[color:var(--color-warn)]
              "
            >
              Continuing on a different model discards the recorded thinking blocks (their
              signatures won’t validate), cold-starts the prompt cache, and changes the
              effective context window.
            </p>
          </div>
        )}

        <div
          className="
            flex justify-end gap-[var(--space-xs)]
            border-t border-[var(--color-rule)] pt-[var(--space-md)]
          "
        >
          <Button variant="ghost" size="sm" onClick={onCancel}>
            Cancel
          </Button>
          <Button variant="primary" size="sm" onClick={() => onConfirm(choice)} disabled={!choice}>
            Resume
          </Button>
        </div>
      </div>
    </ModalShell>
  );
}
