import { useState, useEffect } from 'react';

interface Props {
  backendUrl: string | null;
  status: { lastRunAt: string; outcome: string; pushed: number; pulled: number } | null;
  pairingCode: string | null;
  onSaveBackendUrl: (url: string) => void;
  onBootstrap: () => void;
  onGenerateCode: () => void;
  onJoin: (code: string, deviceName: string) => void;
  onSyncNow: () => void;
}

export function SyncSettings({
  backendUrl,
  status,
  pairingCode,
  onSaveBackendUrl,
  onBootstrap,
  onGenerateCode,
  onJoin,
  onSyncNow,
}: Props) {
  const [urlInput, setUrlInput] = useState(backendUrl ?? '');
  const [joinCode, setJoinCode] = useState('');
  const [deviceName, setDeviceName] = useState('');
  const configured = Boolean(backendUrl);

  // `backendUrl` starts `null` and is loaded asynchronously by the parent
  // (SettingsPanel) after mount — `useState(backendUrl ?? '')` above only
  // reads it once, at first render, so without this effect `urlInput` would
  // stay pinned to '' forever once the real value arrives later.
  useEffect(() => {
    setUrlInput(backendUrl ?? '');
  }, [backendUrl]);

  return (
    <div className="space-y-3 text-[12px]">
      <div className="flex items-center gap-2">
        <input
          type="text"
          value={urlInput}
          onChange={(e) => setUrlInput(e.target.value)}
          placeholder="https://your-sync-backend.example.com"
          className="px-2 py-0.5 rounded bg-[var(--color-bg-surface)] text-[11px] flex-1"
        />
        <button
          type="button"
          onClick={() => onSaveBackendUrl(urlInput)}
          className="px-2 py-0.5 rounded bg-[var(--color-bg-surface-hover)] hover:bg-[var(--color-border-hover)] text-[color:var(--color-text)] text-[11px]"
        >
          Save
        </button>
      </div>
      <div className="flex items-center justify-between">
        <span className="text-neutral-300">
          {status ? `Last sync: ${status.outcome} (${status.pushed} pushed, ${status.pulled} pulled)` : 'Not configured'}
        </span>
        <button
          type="button"
          onClick={onSyncNow}
          disabled={!configured}
          className="px-2 py-0.5 rounded bg-[var(--color-teal-dim)] hover:bg-teal-500/25 text-[color:var(--color-teal)] text-[11px] disabled:opacity-40"
        >
          Sync now
        </button>
      </div>
      <div className="flex items-center gap-2">
        <button
          disabled={!configured}
          type="button"
          onClick={onBootstrap}
          className="px-2 py-0.5 rounded bg-[var(--color-bg-surface-hover)] hover:bg-[var(--color-border-hover)] text-[color:var(--color-text)] text-[11px] disabled:opacity-40"
        >
          Enable on this device
        </button>
        <button
          disabled={!configured}
          type="button"
          onClick={onGenerateCode}
          className="px-2 py-0.5 rounded bg-[var(--color-bg-surface-hover)] hover:bg-[var(--color-border-hover)] text-[color:var(--color-text)] text-[11px] disabled:opacity-40"
        >
          Generate pairing code
        </button>
      </div>
      {pairingCode && (
        <div className="text-[color:var(--color-teal)] font-mono text-[13px]">{pairingCode}</div>
      )}
      <div className="flex items-center gap-2">
        <input
          type="text"
          value={joinCode}
          onChange={(e) => setJoinCode(e.target.value)}
          placeholder="Pairing code"
          className="px-2 py-0.5 rounded bg-[var(--color-bg-surface)] text-[11px] w-24"
        />
        <input
          type="text"
          value={deviceName}
          onChange={(e) => setDeviceName(e.target.value)}
          placeholder="This device's name"
          className="px-2 py-0.5 rounded bg-[var(--color-bg-surface)] text-[11px] flex-1"
        />
        <button
          disabled={!configured}
          type="button"
          onClick={() => onJoin(joinCode, deviceName)}
          className="px-2 py-0.5 rounded bg-[var(--color-teal-dim)] hover:bg-teal-500/25 text-[color:var(--color-teal)] text-[11px] disabled:opacity-40"
        >
          Join
        </button>
      </div>
    </div>
  );
}
