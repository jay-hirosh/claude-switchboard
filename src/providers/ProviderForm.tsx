import { useEffect, useMemo, useState } from 'react';
import type { PresetInfo, Provider } from '../lib/generated/bindings';
import { ipc } from '../lib/ipc';
import { ModalShell } from '../components/modals/ModalShell';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { SelectChevron } from '../components/ui/SelectChevron';
import { hintClass, inputClass, labelClass, selectClass } from '../components/ui/field';
import { Check, Eye, EyeOff, Plus } from '../lib/icons';
import { applyModelEnv } from './modelEnv';

interface Props {
  providerId: string | null;
  /** Supplied by the caller, which already has the row. Deriving it from the
   *  async load instead would flash the credential fields before hiding them
   *  on the one provider that has none. */
  providerKind?: Provider['kind'] | null;
  onClose: () => void;
  onSaved: () => void | Promise<void>;
}

function newId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `p-${Date.now()}`;
}

/** Flags common enough to be worth one click. Kept short on purpose — this is
 *  a shortcut for the usual case, not a catalogue of the CLI. */
const COMMON_ARGS = ['--dangerously-skip-permissions', '--continue'] as const;

export function ProviderForm({ providerId, providerKind = null, onClose, onSaved }: Props) {
  const [presets, setPresets] = useState<PresetInfo[]>([]);
  const [presetId, setPresetId] = useState('');
  const [name, setName] = useState('');
  const [baseUrl, setBaseUrl] = useState('');
  const [token, setToken] = useState('');
  const [model, setModel] = useState('');
  const [quickModel, setQuickModel] = useState('');
  const [extraArgs, setExtraArgs] = useState('');
  const [env, setEnv] = useState<Record<string, string>>({});
  const [existing, setExisting] = useState<Provider | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [revealToken, setRevealToken] = useState(false);
  // Suggestions for the Model / Quick model fields when the Ollama preset is
  // active — the local model list, not a closed set of choices. Kept as a
  // <datalist> rather than a <select> so a value that came from an existing
  // provider (or one not pulled yet) is never silently replaced.
  const [ollamaModels, setOllamaModels] = useState<string[]>([]);
  const [ollamaError, setOllamaError] = useState<string | null>(null);

  useEffect(() => {
    void ipc.listProviderPresets().then(setPresets);
  }, []);

  useEffect(() => {
    if (!providerId) return;
    void ipc.listProviders().then((all) => {
      const p = all.find((x) => x.id === providerId);
      if (!p) return;
      setExisting(p);
      setName(p.name);
      setBaseUrl(p.base_url ?? '');
      setToken(p.auth_token ?? '');
      setModel(p.env['ANTHROPIC_MODEL'] ?? '');
      setQuickModel(p.env['ANTHROPIC_SMALL_FAST_MODEL'] ?? '');
      setExtraArgs(p.extra_args.join(' '));
      setEnv(p.env as Record<string, string>);
      setPresetId(p.preset_id ?? '');
      if (p.preset_id === 'ollama' && p.base_url) void loadOllamaModels(p.base_url);
    });
  }, [providerId]);

  async function loadOllamaModels(url: string) {
    setOllamaError(null);
    try {
      setOllamaModels(await ipc.listOllamaModels(url));
    } catch (e) {
      setOllamaModels([]);
      setOllamaError(e instanceof Error ? e.message : String(e));
    }
  }

  function applyPreset(id: string) {
    setPresetId(id);
    const p = presets.find((x) => x.id === id);
    if (!p) return;
    setName(p.name);
    setBaseUrl(p.base_url);
    setEnv(p.env as Record<string, string>);
    setModel(p.env['ANTHROPIC_MODEL'] ?? '');
    setQuickModel(p.env['ANTHROPIC_SMALL_FAST_MODEL'] ?? '');
    if (id === 'ollama') void loadOllamaModels(p.base_url);
  }

  // The official provider's credentials, endpoint and model all come from the
  // active account — `resolved_env` returns an empty map for it, so those
  // fields would be inert controls that silently do nothing. Its launch flags
  // are the one thing it does own, so that is all it shows.
  const isOfficial = (existing?.kind ?? providerKind) === 'official';

  const title = useMemo(
    () => (isOfficial ? 'Edit Anthropic (official)' : providerId ? 'Edit provider' : 'Add provider'),
    [isOfficial, providerId],
  );

  function toggleArg(flag: string) {
    setExtraArgs((cur) => {
      const parts = cur.split(/\s+/).filter(Boolean);
      const next = parts.includes(flag)
        ? parts.filter((p) => p !== flag)
        : [...parts, flag];
      return next.join(' ');
    });
  }

  async function save() {
    // The official row has no name or URL to validate — it is seeded, not
    // authored, and only its flags are editable.
    if (!isOfficial && (!name.trim() || !baseUrl.trim())) {
      setError('Name and base URL are both required.');
      return;
    }
    if (!isOfficial && !/^https?:\/\//i.test(baseUrl.trim())) {
      setError('Base URL must start with https:// (or http:// for a local endpoint).');
      return;
    }
    setSaving(true);
    try {
      const merged = applyModelEnv(env, model, quickModel);
      const provider: Provider = {
        id: existing?.id ?? providerId ?? newId(),
        name: isOfficial ? (existing?.name ?? name) : name.trim(),
        // Never hardcode third_party: doing so on the official row would
        // rewrite it into a custom endpoint. The store pins this too, so the
        // invariant does not rest on the form alone.
        kind: isOfficial ? 'official' : 'third_party',
        base_url: isOfficial ? null : baseUrl.trim(),
        auth_token: isOfficial ? null : token,
        env: isOfficial ? (existing?.env ?? {}) : merged,
        // Whitespace-split is deliberate: each token becomes its own argv
        // entry, quoted separately by the script renderer.
        extra_args: extraArgs.trim() ? extraArgs.trim().split(/\s+/) : [],
        preset_id: presetId || null,
        sort_index: existing?.sort_index ?? Date.now() % 100000,
      };
      await ipc.upsertProvider(provider);
      await onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    // ModalShell's dismiss prop is `onDismiss`, and `id` is required — it keys
    // the modal in the app store's modal stack.
    <ModalShell id="provider-form" title={title} onDismiss={onClose}>
      {/* ModalShell renders children raw — the padding has to come from here,
          or every field runs flush into the modal border. */}
      <div className="flex flex-col gap-[var(--space-md)] px-[var(--space-md)] py-[var(--space-md)]">
        {error && <Banner variant="error">{error}</Banner>}

        {isOfficial && (
          <p className={hintClass}>
            Credentials, endpoint and model come from your active account and are not set
            here. These flags are added to every launch of this provider.
          </p>
        )}

        {!isOfficial && (
          <>
        <label className="flex flex-col gap-[var(--space-2xs)]">
          <span className={labelClass}>Preset</span>
          <span className="relative block">
            <select
              aria-label="Preset"
              className={selectClass}
              value={presetId}
              onChange={(e) => applyPreset(e.target.value)}
            >
              <option value="">Custom</option>
              {presets.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
            <SelectChevron />
          </span>
        </label>

        <label className="flex flex-col gap-[var(--space-2xs)]">
          <span className={labelClass}>Name</span>
          <input
            aria-label="Name"
            className={inputClass}
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </label>

        <label className="flex flex-col gap-[var(--space-2xs)]">
          <span className={labelClass}>Base URL</span>
          <input
            aria-label="Base URL"
            className={`${inputClass} mono`}
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder="https://api.example.com/anthropic"
          />
        </label>

        <label className="flex flex-col gap-[var(--space-2xs)]">
          <span className={labelClass}>API key</span>
          <div className="relative">
            <input
              aria-label="API key"
              type={revealToken ? 'text' : 'password'}
              className={`${inputClass} mono pr-[34px]`}
              value={token}
              onChange={(e) => setToken(e.target.value)}
            />
            <button
              type="button"
              onClick={() => setRevealToken((v) => !v)}
              /* Deliberately not "…API key": that string would also match the
                 field's own label and make getByLabelText ambiguous. */
              aria-label={revealToken ? 'Hide key' : 'Show key'}
              className="
                absolute right-[var(--space-2xs)] top-1/2 -translate-y-1/2
                inline-flex h-[24px] w-[24px] items-center justify-center
                rounded-[var(--radius-sm)] text-[color:var(--color-text-muted)]
                transition-colors duration-[var(--duration-fast)]
                hover:text-[color:var(--color-text-secondary)]
              "
            >
              {revealToken ? <EyeOff size={13} aria-hidden /> : <Eye size={13} aria-hidden />}
            </button>
          </div>
        </label>

        <div className="flex flex-col gap-[var(--space-2xs)]">
          <div className="flex gap-[var(--space-sm)]">
            <label className="flex min-w-0 flex-1 flex-col gap-[var(--space-2xs)]">
              <span className={labelClass}>Model</span>
              <input
                aria-label="Model"
                className={`${inputClass} mono`}
                value={model}
                onChange={(e) => setModel(e.target.value)}
                list={presetId === 'ollama' ? 'ollama-model-list' : undefined}
              />
            </label>
            <label className="flex min-w-0 flex-1 flex-col gap-[var(--space-2xs)]">
              <span className={labelClass}>Quick model</span>
              <input
                aria-label="Quick model"
                className={`${inputClass} mono`}
                value={quickModel}
                onChange={(e) => setQuickModel(e.target.value)}
                placeholder="same as Model"
                list={presetId === 'ollama' ? 'ollama-model-list' : undefined}
              />
            </label>
          </div>
          {presetId === 'ollama' && (
            <datalist id="ollama-model-list">
              {ollamaModels.map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
          )}
          {presetId === 'ollama' && ollamaError && (
            <span className={hintClass}>
              Couldn't reach Ollama at <code className="mono">{baseUrl}</code> to list installed
              models — enter a model name manually (see <code className="mono">ollama list</code>).
            </span>
          )}
          <span className={hintClass}>
            The quick model runs background work — titles, summaries, file
            classification. Leave it blank to let the provider decide. Both fields also
            set the <code className="mono">/model</code> aliases, so{' '}
            <code className="mono">opus</code> and <code className="mono">sonnet</code> point
            at Model and <code className="mono">haiku</code> at Quick model.
          </span>
        </div>
          </>
        )}

        <label className="flex flex-col gap-[var(--space-2xs)]">
          <span className={labelClass}>Extra CLI arguments</span>
          <input
            aria-label="Extra CLI arguments"
            className={`${inputClass} mono`}
            value={extraArgs}
            onChange={(e) => setExtraArgs(e.target.value)}
            placeholder="e.g. --verbose"
          />
          <span className={hintClass}>
            Passed to <code className="mono">claude</code> on launch. The script runs the binary
            directly, so shell aliases and functions do not apply.
          </span>
          {/* The common flag was previously *only* a placeholder, which looks
              like a prefilled value but is not one — nothing typed it, nothing
              saved it, and there was no affordance to accept it. A one-click
              chip that appends the real text removes the ambiguity. */}
          <div className="flex flex-wrap items-center gap-[var(--space-2xs)] pt-[var(--space-2xs)]">
            {COMMON_ARGS.map((flag) => {
              const present = extraArgs.split(/\s+/).includes(flag);
              return (
                <button
                  key={flag}
                  type="button"
                  onClick={() => toggleArg(flag)}
                  aria-pressed={present}
                  className={[
                    'mono inline-flex items-center gap-[var(--space-2xs)]',
                    'rounded-[var(--radius-pill)] px-[var(--space-xs)] py-[1px]',
                    'text-[length:var(--text-micro)]',
                    'transition-[background,color] duration-[var(--duration-fast)]',
                    present
                      ? 'bg-[var(--color-accent-dim)] text-[color:var(--color-accent)]'
                      : 'bg-[var(--color-track)] text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text-secondary)]',
                  ].join(' ')}
                >
                  {present ? <Check size={10} aria-hidden /> : <Plus size={10} aria-hidden />}
                  {flag}
                </button>
              );
            })}
          </div>
        </label>

        <div className="flex items-center justify-between gap-[var(--space-md)] border-t border-[var(--color-rule)] pt-[var(--space-md)]">
          {/* The official provider deliberately sets no env of its own — the
              session inherits the active account — so a "0 environment
              variables" line would read as something being wrong. */}
          <span className={hintClass}>
            {isOfficial
              ? 'Runs on your active account.'
              : `${Object.keys(env).length} environment variable${
                  Object.keys(env).length === 1 ? '' : 's'
                } set on launch.`}
          </span>
          <div className="flex shrink-0 gap-[var(--space-xs)]">
            <Button variant="ghost" size="sm" onClick={onClose}>
              Cancel
            </Button>
            <Button variant="primary" size="sm" onClick={save} disabled={saving}>
              Save
            </Button>
          </div>
        </div>
      </div>
    </ModalShell>
  );
}
