import { useEffect, useMemo, useState } from 'react';
import type { PresetInfo, Provider } from '../lib/generated/bindings';
import { ipc } from '../lib/ipc';
import { ModalShell } from '../components/modals/ModalShell';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { Eye, EyeOff } from '../lib/icons';

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

export function ProviderForm({ providerId, onClose, onSaved }: Props) {
  const [presets, setPresets] = useState<PresetInfo[]>([]);
  const [presetId, setPresetId] = useState('');
  const [name, setName] = useState('');
  const [baseUrl, setBaseUrl] = useState('');
  const [token, setToken] = useState('');
  const [model, setModel] = useState('');
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
  }

  const title = useMemo(() => (providerId ? 'Edit provider' : 'Add provider'), [providerId]);

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
      const merged = { ...env };
      if (model.trim()) merged['ANTHROPIC_MODEL'] = model.trim();
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

        <label className="flex flex-col gap-[var(--space-2xs)]">
          <span className={labelClass}>Model</span>
          <input
            aria-label="Model"
            className={`${inputClass} mono`}
            value={model}
            onChange={(e) => setModel(e.target.value)}
          />
        </label>

        <label className="flex flex-col gap-[var(--space-2xs)]">
          <span className={labelClass}>Extra CLI arguments</span>
          <input
            aria-label="Extra CLI arguments"
            className={`${inputClass} mono`}
            value={extraArgs}
            onChange={(e) => setExtraArgs(e.target.value)}
            placeholder="--dangerously-skip-permissions"
          />
          <span className={hintClass}>
            Passed to <code className="mono">claude</code> on launch. The script runs the binary
            directly, so shell aliases and functions do not apply.
          </span>
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
