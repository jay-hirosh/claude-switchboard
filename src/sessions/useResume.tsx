import { useCallback, useEffect, useState } from 'react';
import type {
  LaunchSurface,
  Provider,
  SessionSummary,
  Terminal,
} from '../lib/generated/bindings';
import { ipc } from '../lib/ipc';
import { resolveProvider } from './resolveProvider';
import { ResumeProviderPicker } from './ResumeProviderPicker';

export function useResume() {
  const [pending, setPending] = useState<{
    session: SessionSummary;
    providers: Provider[];
    surface: LaunchSurface;
  } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [vsCodeAvailable, setVsCodeAvailable] = useState(false);

  // Probed once: neither the `code` command nor the extension appears while the
  // popover is open, and re-probing per row would stat the filesystem on hover.
  useEffect(() => {
    void (async () => {
      try {
        setVsCodeAvailable(await ipc.vscodeTabAvailable());
      } catch {
        // A failed probe cannot be read as "the surface works". Leaving the
        // option disabled is the safe reading and the terminal path is
        // unaffected, so this one does not warrant a banner.
        setVsCodeAvailable(false);
      }
    })();
  }, []);

  const pickTerminal = useCallback(async (): Promise<Terminal | null> => {
    const [settings, available] = await Promise.all([
      ipc.getSettings(),
      ipc.listAvailableTerminals(),
    ]);
    const configured = settings.terminal;
    return configured && available.includes(configured) ? configured : (available[0] ?? null);
  }, []);

  const launch = useCallback(
    async (session: SessionSummary, provider: Provider, surface: LaunchSurface) => {
      // A VS Code tab needs no terminal emulator, and demanding one would
      // refuse the launch on a machine that has an editor but no supported
      // terminal.
      let terminal: Terminal | null = null;
      if (surface === 'terminal') {
        terminal = await pickTerminal();
        if (!terminal) {
          setNotice('No supported terminal found. Install Ghostty or use Copy command.');
          return;
        }
      }
      try {
        // The launcher appends --fork-session, so resuming a session that is
        // still open elsewhere cannot put two processes on one transcript.
        // The permission mode rides along so the continued session behaves the
        // way the one it continues did.
        await ipc.launchProviderSession(
          provider.id,
          session.cwd,
          // Ignored by the backend for a VS Code tab, but the argument is not
          // optional — one unused terminal is as good as another.
          terminal ?? 'power_shell',
          session.session_id,
          session.permission_mode,
          surface,
        );
        // The extension builds its own argv, so neither of these crosses into a
        // tab. Saying so beats letting a bypassPermissions session come back up
        // asking for permission with no explanation — but only mention what the
        // session actually had, or the warning is noise.
        const dropped =
          surface === 'vs_code_tab'
            ? [
                provider.extra_args.length > 0 ? 'the provider’s CLI flags' : null,
                session.permission_mode ? 'its permission mode' : null,
              ].filter(Boolean)
            : [];
        setNotice(
          dropped.length > 0
            ? `Opening a VS Code tab — ${dropped.join(' and ')} will not carry over; the extension sets its own.`
            : null,
        );
      } catch (e) {
        setNotice(e instanceof Error ? e.message : String(e));
      }
    },
    [pickTerminal],
  );

  const resume = useCallback(
    async (session: SessionSummary, surface: LaunchSurface) => {
      const providers = await ipc.listProviders();
      const resolution = resolveProvider(session.model, providers);
      if (resolution.kind === 'resolved') {
        const provider = providers.find((p) => p.id === resolution.providerId);
        if (provider) {
          await launch(session, provider, surface);
          return;
        }
      }
      // Never guess: an unresolved model must be confirmed, or we risk
      // silently continuing a conversation on the wrong model.
      setPending({ session, providers, surface });
    },
    [launch],
  );

  const dialog = pending ? (
    <ResumeProviderPicker
      session={pending.session}
      providers={pending.providers}
      onCancel={() => setPending(null)}
      onConfirm={async (providerId) => {
        const { session, providers, surface } = pending;
        const provider = providers.find((p) => p.id === providerId);
        setPending(null);
        if (provider) await launch(session, provider, surface);
      }}
    />
  ) : null;

  return { resume, dialog, notice, vsCodeAvailable };
}
