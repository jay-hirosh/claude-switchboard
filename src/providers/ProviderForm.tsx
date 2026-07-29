import { useEffect, useMemo, useState } from 'react';
import type { PresetInfo, Provider } from '../lib/generated/bindings';
import { ipc } from '../lib/ipc';
import { ModalShell } from '../components/modals/ModalShell';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { Check, Eye, EyeOff, Plus } from '../lib/icons';
import { applyModelEnv } from './modelEnv';

interface Props {
  providerId: string | null;
  onClose: () => void;
  onSaved: () => void | Promise<void>;
}

const inputClass = [
  'w-full rounded-[var(--radius-sm)]',
  'border border-[var(--color-border)] bg-[var(--color-bg-base)]',
  'px-[var(--space-sm)] py-[var(--space-xs)]',
  'text-[length:var(--text-body)] text-[color:var(--color-text)]',
  'placeholder:text-[color:var(--color-text-muted)]',
  'transition-[border-color] duration-[var(--duration-fast)]',
  'focus:border-[var(--color-border-focus)] focus:outline-none',
].join(' ');

const labelClass = [
  'text-[length:var(--text-micro)] font-[var(--weight-medium)]',
  'uppercase tracking-[var(--tracking-label)]',
  'text-[color:var(--color-text-muted)]',
].join(' ');

const hintClass =
  'text-[length:var(--text-micro)] leading-[var(--leading-body)] text-[color:var(--color-text-muted)]';

function newId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `p-${Date.now()}`;
}

/** Flags common enough to be worth one click. Kept short on purpose — this is
 *  a shortcut for the usual case, not a catalogue of the CLI. */
const COMMON_ARGS = ['--dangerously-skip-permissions', '--continue'] as const;

export function ProviderForm({ providerId, onClose, onSaved }: Props) {
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
    });
  }, [providerId]);

  function applyPreset(id: string) {
    setPresetId(id);
    const p = presets.find((x) => x.id === id);
    if (!p) return;
    setName(p.name);
    setBaseUrl(p.base_url);
    setEnv(p.env as Record<string, string>);
    setModel(p.env['ANTHROPIC_MODEL'] ?? '');
    setQuickModel(p.env['ANTHROPIC_SMALL_FAST_MODEL'] ?? '');
  }

  const title = useMemo(() => (providerId ? 'Edit provider' : 'Add provider'), [providerId]);

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
    if (!name.trim() || !baseUrl.trim()) {
      setError('Name and base URL are both required.');
      return;
    }
    if (!/^https?:\/\//i.test(baseUrl.trim())) {
      setError('Base URL must start with https:// (or http:// for a local endpoint).');
      return;
    }
    setSaving(true);
    try {
      const merged = applyModelEnv(env, model, quickModel);
      const provider: Provider = {
        id: existing?.id ?? providerId ?? newId(),
        name: name.trim(),
        kind: 'third_party',
        base_url: baseUrl.trim(),
        auth_token: token,
        env: merged,
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

        <label className="flex flex-col gap-[var(--space-2xs)]">
          <span className={labelClass}>Preset</span>
          <select
            aria-label="Preset"
            className={inputClass}
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
              />
            </label>
          </div>
          <span className={hintClass}>
            The quick model runs background work — titles, summaries, file
            classification. Leave it blank to let the provider decide. Both fields also
            set the <code className="mono">/model</code> aliases, so{' '}
            <code className="mono">opus</code> and <code className="mono">sonnet</code> point
            at Model and <code className="mono">haiku</code> at Quick model.
          </span>
        </div>

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
          <span className={hintClass}>
            {Object.keys(env).length} environment variable
            {Object.keys(env).length === 1 ? '' : 's'} set on launch.
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
