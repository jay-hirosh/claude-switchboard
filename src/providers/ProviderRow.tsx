import type { Provider } from '../lib/generated/bindings';
import { Badge } from '../components/ui/Badge';
import { Button } from '../components/ui/Button';
import { IconButton } from '../components/ui/IconButton';
import { ModelBadge } from '../components/ui/ModelBadge';
import { Play, Pencil, Trash2, Star } from '../lib/icons';

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
        px-[var(--space-md)] py-[var(--space-sm)]
        transition-[background] duration-[var(--duration-fast)] ease-[var(--ease-out)]
        hover:bg-[var(--color-bg-card-hover)]
      "
    >
      <div className="flex min-w-0 flex-1 flex-col">
        <span
          className="
            truncate text-[length:var(--text-body)] leading-[var(--leading-title)]
            font-[var(--weight-medium)] text-[color:var(--color-text)]
          "
        >
          {provider.name}
        </span>
        <span className="truncate text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          {isOfficial ? 'Uses your active account' : hostOf(provider.base_url)}
        </span>
      </div>

      <ModelBadge model={model} />

      {/* Every trailing control lives in a fixed-width slot, present even when
          empty. The official row has no default toggle and no edit/delete, so
          without reserved slots its Launch button sat further right than every
          other row's — the ragged edge that made the list read as broken. */}
      <div className="flex w-[84px] shrink-0 justify-end">
        {onSetDefault &&
          (isDefault ? (
            <Badge variant="accent" icon={<Star size={10} aria-hidden />}>
              Default
            </Badge>
          ) : (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onSetDefault(provider.id)}
              aria-label={`Set ${provider.name} as default`}
              /* The official row's default is the absence of one: it undoes
                 whatever a third-party default wrote to settings.json, rather
                 than writing an env block of its own. Two very different
                 consequences behind one label, so each says which it is. */
              title={
                isOfficial
                  ? 'Removes the provider env Switchboard added to ~/.claude/settings.json and restores what was there before, so a bare `claude` uses your account again. A backup is written first.'
                  : `Writes this provider's env into ~/.claude/settings.json, so a bare \`claude\` uses ${provider.name} everywhere. A backup is written first, and turning it off restores your own values.`
              }
            >
              Set default
            </Button>
          ))}
      </div>

      <Button
        variant="primary"
        size="sm"
        onClick={() => onLaunch(provider.id)}
        aria-label={`Launch ${provider.name}`}
      >
        <Play size={12} aria-hidden />
        Launch
      </Button>

      <div className="flex w-[62px] shrink-0 justify-end gap-[2px]">
        {/* The official row is editable too: its credentials come from the
            active account, but its launch flags are the user's, and there was
            no way to set them. Delete stays hidden — the store refuses it. */}
        {/* IconButton takes a required `label` prop and applies it as
            aria-label itself — do not pass aria-label directly. */}
        <IconButton label={`Edit ${provider.name}`} onClick={() => onEdit(provider.id)}>
          <Pencil size={13} aria-hidden />
        </IconButton>
        {!isOfficial && (
          <IconButton label={`Delete ${provider.name}`} onClick={() => onDelete(provider.id)}>
            <Trash2 size={13} aria-hidden />
          </IconButton>
        )}
      </div>
    </div>
  );
}
