import { useCallback, useEffect, useState } from 'react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import type { Terminal } from '../lib/generated/bindings';
import { ipc } from '../lib/ipc';
import { useProviders } from './useProviders';
import { ProviderRow } from './ProviderRow';
import { ProviderForm } from './ProviderForm';
import { Button } from '../components/ui/Button';
import { Plus } from '../lib/icons';

export function ProvidersTab() {
  const { providers, loading, error, reload } = useProviders();
  const [terminal, setTerminal] = useState<Terminal | null>(null);
  const [editing, setEditing] = useState<string | 'new' | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    void ipc.listAvailableTerminals().then((ts) => setTerminal(ts[0] ?? null));
  }, []);

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

      {!loading &&
        providers.map((p) => (
          <ProviderRow
            key={p.id}
            provider={p}
            onLaunch={handleLaunch}
            onEdit={setEditing}
            onDelete={handleDelete}
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
