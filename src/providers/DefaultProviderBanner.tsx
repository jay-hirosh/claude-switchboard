interface Props {
  providerName: string;
  onClear: () => void;
}

export function DefaultProviderBanner({ providerName, onClear }: Props) {
  return (
    <div
      role="status"
      className="
        flex items-start gap-[var(--space-sm)]
        rounded-[var(--radius-sm)] border border-[var(--color-warn)]
        bg-[var(--color-warn-dim)]
        px-[var(--space-sm)] py-[var(--space-2xs)]
      "
    >
      <div className="flex-1 text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]">
        <div className="text-[color:var(--color-text)]">
          <strong>{providerName}</strong> is the default for every Claude Code session.
        </div>
        <div>
          This overrides <code className="mono">ANTHROPIC_*</code> variables exported by your shell,
          so your own launch scripts stop taking effect. Sessions already running keep their current
          provider until restarted.
        </div>
      </div>
      <button
        type="button"
        onClick={onClear}
        className="shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-accent)] hover:underline"
      >
        Turn off
      </button>
    </div>
  );
}
