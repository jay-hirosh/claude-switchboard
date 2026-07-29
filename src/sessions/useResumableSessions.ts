import { useCallback, useEffect, useState } from 'react';
import type { SessionSummary } from '../lib/generated/bindings';
import { ipc } from '../lib/ipc';

export function useResumableSessions() {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      setSessions(await ipc.listResumableSessions());
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  return { sessions, loading, error, reload };
}
