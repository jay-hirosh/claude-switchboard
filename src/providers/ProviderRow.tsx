import type { Provider } from '../lib/generated/bindings';
import { Button } from '../components/ui/Button';
import { IconButton } from '../components/ui/IconButton';
import { Play, Pencil, Trash2 } from '../lib/icons';

interface Props {
  provider: Provider;
  isDefault?: boolean;
  onLaunch: (id: string) => void;
  onEdit: (id: string) => void;
  onDelete: (id: string) => void;
  onSetDefault?: (id: string) => void;
}

function hostOf(url: string | null): string {
  if (!url) return '';
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export function ProviderRow({
  provider,
  isDefault = false,
  onLaunch,
  onEdit,
  onDelete,
  onSetDefault,
}: Props) {
  const isOfficial = provider.kind === 'official';
  const model = provider.env['ANTHROPIC_MODEL'] ?? null;

  return (
    <div
      className="
        flex items-center gap-[var(--space-sm)]
        rounded-[var(--radius-sm)] border border-[var(--color-border)]
        bg-[var(--color-bg-card)]
        px-[var(--space-sm)] py-[var(--space-xs)]
      "
    >
      <div className="flex min-w-0 flex-1 flex-col gap-[var(--space-2xs)]">
        <span className="truncate text-[length:var(--text-body)] text-[color:var(--color-text)]">
          {provider.name}
        </span>
        <span className="truncate text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          {isOfficial ? 'Uses your active account' : hostOf(provider.base_url)}
        </span>
      </div>

      {model && (
        <span className="mono shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]">
          {model}
        </span>
      )}

      {onSetDefault && !isOfficial && (
        <button
          type="button"
          onClick={() => onSetDefault(provider.id)}
          aria-label={`Set ${provider.name} as default`}
          className={[
            'shrink-0 text-[length:var(--text-micro)] hover:underline',
            isDefault
              ? 'text-[color:var(--color-warn)]'
              : 'text-[color:var(--color-text-muted)]',
          ].join(' ')}
        >
          {isDefault ? 'Default' : 'Set default'}
        </button>
      )}

      <Button
        variant="primary"
        size="sm"
        onClick={() => onLaunch(provider.id)}
        aria-label={`Launch ${provider.name}`}
      >
        <Play size={13} aria-hidden />
        Launch
      </Button>

      {!isOfficial && (
        <>
          {/* IconButton takes a required `label` prop and applies it as
              aria-label itself — do not pass aria-label directly. */}
          <IconButton label={`Edit ${provider.name}`} onClick={() => onEdit(provider.id)}>
            <Pencil size={14} aria-hidden />
          </IconButton>
          <IconButton label={`Delete ${provider.name}`} onClick={() => onDelete(provider.id)}>
            <Trash2 size={14} aria-hidden />
          </IconButton>
        </>
      )}
    </div>
  );
}
