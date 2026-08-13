import { useCallback, useEffect, useState } from 'react';
import { Banner } from '../components/ui/Banner';
import { Button } from '../components/ui/Button';
import { ipc } from '../lib/ipc';
import type { StatuslineInstallState } from '../lib/generated/bindings';

export function StatuslineSettings() {
  const [state, setState] = useState<StatuslineInstallState | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setState(await ipc.getStatuslineInstallState());
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const handleInstall = useCallback(async () => {
    setBusy(true);
    try {
      const outcome = await ipc.installStatusline(false);
      if (outcome.status === 'needs_confirmation') {
        const cmd =
          typeof outcome.foreign_value === 'object' &&
          outcome.foreign_value !== null &&
          'command' in outcome.foreign_value
            ? String((outcome.foreign_value as { command: unknown }).command)
            : JSON.stringify(outcome.foreign_value);
        const ok = window.confirm(
          `~/.claude/settings.json already has a statusLine command (${cmd}). Switchboard did not write this — another tool or a manual edit did.\n\nOverwrite it?`,
        );
        if (!ok) return;
        await ipc.installStatusline(true);
      }
      setNotice(null);
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
      await reload();
    }
  }, [reload]);

  const handleUninstall = useCallback(async () => {
    setBusy(true);
    try {
      await ipc.uninstallStatusline();
      setNotice(null);
    } catch (e) {
      setNotice(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
      await reload();
    }
  }, [reload]);

  return (
    <div className="flex flex-col gap-[var(--space-sm)]">
      <div className="flex items-center justify-between">
        <div className="flex flex-col gap-[var(--space-2xs)]">
          <span className="text-[length:var(--text-body)] text-[color:var(--color-text)]">
            Terminal statusline
          </span>
          <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
            Shows your 5-hour usage % in the Claude Code terminal prompt. Only works while
            Switchboard is running.
          </span>
        </div>
        {state ? (
          <Button variant="ghost" size="sm" onClick={handleUninstall} disabled={busy}>
            Uninstall
          </Button>
        ) : (
          <Button variant="ghost" size="sm" onClick={handleInstall} disabled={busy}>
            Install
          </Button>
        )}
      </div>
      {notice && (
        <Banner variant="warning" role="status">
          {notice}
        </Banner>
      )}
    </div>
  );
}
