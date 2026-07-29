import { useCallback, useState } from 'react';
import type { Provider, SessionSummary, Terminal } from '../lib/generated/bindings';
import { ipc } from '../lib/ipc';
import { resolveProvider } from './resolveProvider';
import { ResumeProviderPicker } from './ResumeProviderPicker';

export function useResume() {
  const [pending, setPending] = useState<{
    session: SessionSummary;
    providers: Provider[];
  } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const pickTerminal = useCallback(async (): Promise<Terminal | null> => {
    const [settings, available] = await Promise.all([
      ipc.getSettings(),
      ipc.listAvailableTerminals(),
    ]);
    const configured = settings.terminal;
    return configured && available.includes(configured) ? configured : (available[0] ?? null);
  }, []);

  const launch = useCallback(
    async (session: SessionSummary, providerId: string) => {
      const terminal = await pickTerminal();
      if (!terminal) {
        setNotice('No supported terminal found. Install Ghostty or use Copy command.');
        return;
      }
      try {
        // The launcher appends --fork-session, so resuming a session that is
        // still open elsewhere cannot put two processes on one transcript.
        await ipc.launchProviderSession(providerId, session.cwd, terminal, session.session_id);
        setNotice(null);
      } catch (e) {
        setNotice(e instanceof Error ? e.message : String(e));
      }
    },
    [pickTerminal],
  );

  const resume = useCallback(
    async (session: SessionSummary) => {
      const providers = await ipc.listProviders();
      const resolution = resolveProvider(session.model, providers);
      if (resolution.kind === 'resolved') {
        await launch(session, resolution.providerId);
        return;
      }
      // Never guess: an unresolved model must be confirmed, or we risk
      // silently continuing a conversation on the wrong model.
      setPending({ session, providers });
    },
    [launch],
  );

  const dialog = pending ? (
    <ResumeProviderPicker
      session={pending.session}
      providers={pending.providers}
      onCancel={() => setPending(null)}
      onConfirm={async (providerId) => {
        const { session } = pending;
        setPending(null);
        await launch(session, providerId);
      }}
    />
  ) : null;

  return { resume, dialog, notice };
}
