import { useState } from 'react';
import type { Provider, SessionSummary } from '../lib/generated/bindings';
import { ModalShell } from '../components/modals/ModalShell';
import { Button } from '../components/ui/Button';

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
      <div className="flex flex-col gap-[var(--space-sm)]">
        <p className="text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]">
          This session ran on <span className="mono">{recorded}</span>, which doesn’t match any
          provider you’ve configured.
        </p>

        <label className="flex flex-col gap-[var(--space-2xs)] text-[length:var(--text-micro)]">
          Resume with
          <select
            aria-label="Provider"
            value={choice}
            onChange={(e) => setChoice(e.target.value)}
            className="w-full rounded-[var(--radius-sm)] border border-[var(--color-border)] bg-[var(--color-bg-base)] px-[var(--space-xs)] py-[var(--space-2xs)] text-[length:var(--text-body)]"
          >
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </label>

        {differs && (
          <p className="text-[length:var(--text-micro)] text-[color:var(--color-warn)]">
            Continuing on a different model discards the recorded thinking blocks (their signatures
            won’t validate), cold-starts the prompt cache, and changes the effective context window.
          </p>
        )}

        <div className="flex justify-end gap-[var(--space-xs)]">
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
