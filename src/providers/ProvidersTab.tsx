import { useCallback, useEffect, useState } from 'react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import type { DefaultProviderState, Terminal } from '../lib/generated/bindings';
import { ipc } from '../lib/ipc';
import { useProviders } from './useProviders';
import { ProviderRow } from './ProviderRow';
import { ProviderForm } from './ProviderForm';
import { DefaultProviderBanner } from './DefaultProviderBanner';
import { Button } from '../components/ui/Button';
import { Plus } from '../lib/icons';

export function ProvidersTab() {
  const { providers, loading, error, reload } = useProviders();
  const [terminal, setTerminal] = useState<Terminal | null>(null);
  const [editing, setEditing] = useState<string | 'new' | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [defaultState, setDefaultState] = useState<DefaultProviderState | null>(null);

  useEffect(() => {
    void (async () => {
      const [settings, available] = await Promise.all([
        ipc.getSettings(),
        ipc.listAvailableTerminals(),
      ]);
      const configured = settings.terminal;
      setTerminal(
        configured && available.includes(configured) ? configured : (available[0] ?? null),
      );
    })();
  }, []);

  const reloadDefault = useCallback(async () => {
    setDefaultState(await ipc.getDefaultProvider());
  }, []);

  useEffect(() => {
    void reloadDefault();
  }, [reloadDefault]);

  const handleSetDefault = useCallback(
    async (id: string) => {
      const outcome = await ipc.setDefaultProvider(id, false);
      if (outcome.status === 'needs_confirmation') {
        const keys = outcome.unmanaged_keys.join(', ');
        const ok = window.confirm(
          `~/.claude/settings.json already sets ${keys}. Switchboard did not write these — another tool or a manual edit did.\n\nOverwrite them?`,
        );
        if (!ok) return;
        await ipc.setDefaultProvider(id, true);
      }
      await reloadDefault();
    },
    [reloadDefault],
  );

  const handleClearDefault = useCallback(async () => {
    // Returns keys left untouched because the user hand-edited them while the
    // default was active (spec §4.2). Silently reverting someone's edit would
    // be worse than leaving it, so say what was skipped.
    const skipped = await ipc.clearDefaultProvider();
    if (skipped.length > 0) {
      setNotice(`Default turned off. Left your own edits to ${skipped.join(', ')} in place.`);
    }
    await reloadDefault();
  }, [reloadDefault]);

  const handleLaunch = useCallback(
    async (id: string) => {
      const dir = await openDialog({ directory: true, multiple: false, title: 'Choose a folder' });
      if (typeof dir !== 'string') return;
      if (!terminal) {
        setNotice('No supported terminal found. Install Ghostty or Windows Terminal.');
        return;
      }
      try {
        await ipc.launchProviderSession(id, dir, terminal, null);
        setNotice(null);
      } catch (e) {
        setNotice(e instanceof Error ? e.message : String(e));
      }
    },
    [terminal],
  );

  const handleDelete = useCallback(
    async (id: string) => {
      await ipc.deleteProvider(id);
      await reload();
    },
    [reload],
  );

  const thirdParty = providers.filter((p) => p.kind !== 'official');

  return (
    <div className="flex flex-col gap-[var(--space-sm)] p-[var(--space-md)]">
      {error && (
        <div
          role="alert"
          className="rounded-[var(--radius-sm)] border border-[var(--color-danger)] bg-[var(--color-danger-dim)] px-[var(--space-sm)] py-[var(--space-2xs)] text-[length:var(--text-micro)]"
        >
          {error}
        </div>
      )}
      {notice && (
        <div
          role="status"
          className="rounded-[var(--radius-sm)] border border-[var(--color-warn)] bg-[var(--color-warn-dim)] px-[var(--space-sm)] py-[var(--space-2xs)] text-[length:var(--text-micro)]"
        >
          {notice}
        </div>
      )}

      {defaultState && (
        <DefaultProviderBanner
          providerName={
            providers.find((p) => p.id === defaultState.provider_id)?.name ?? 'A provider'
          }
          onClear={handleClearDefault}
        />
      )}

      {!loading &&
        providers.map((p) => (
          <ProviderRow
            key={p.id}
            provider={p}
            isDefault={defaultState?.provider_id === p.id}
            onLaunch={handleLaunch}
            onEdit={setEditing}
            onDelete={handleDelete}
            onSetDefault={handleSetDefault}
          />
        ))}

      {!loading && !error && thirdParty.length === 0 && (
        <p className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          Add a provider to run Claude Code against a custom endpoint.
        </p>
      )}

      <div>
        <Button variant="ghost" size="sm" onClick={() => setEditing('new')}>
          <Plus size={13} aria-hidden />
          Add provider
        </Button>
      </div>

      {editing && (
        <ProviderForm
          providerId={editing === 'new' ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={async () => {
            setEditing(null);
            await reload();
          }}
        />
      )}
    </div>
  );
}
