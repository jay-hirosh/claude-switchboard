import { useEffect, useState } from 'react';
import { ipc } from './ipc';
import { useAppStore } from './store';

export type ActiveModel = 'opus' | 'sonnet' | 'haiku' | null;

function familyOf(model: string): ActiveModel {
  const m = model.toLowerCase();
  if (m.includes('opus')) return 'opus';
  if (m.includes('sonnet')) return 'sonnet';
  if (m.includes('haiku')) return 'haiku';
  return null;
}

/**
 * Derives the most recently used model family from local session history.
 * Returns null if no events in the last 24h. Re-runs whenever session
 * ingestion advances `sessionDataVersion` so the highlight tracks live.
 */
export function useActiveModel(): ActiveModel {
  const version = useAppStore((s) => s.sessionDataVersion);
  const [model, setModel] = useState<ActiveModel>(null);

  useEffect(() => {
    let cancelled = false;
    // Always local-only, regardless of the Settings toggle: this drives a
    // live "what's active right now" indicator, and a synced peer's recent
    // activity on another machine isn't what's active in this session.
    ipc
      .getSessionHistory(1, true)
      .then((events) => {
        if (cancelled) return;
        if (!events.length) {
          setModel(null);
          return;
        }
        let latestTs = events[0].ts;
        let latest = events[0];
        for (const e of events) {
          if (e.ts > latestTs) {
            latestTs = e.ts;
            latest = e;
          }
        }
        setModel(familyOf(latest.model));
      })
      .catch(() => {
        if (!cancelled) setModel(null);
      });
    return () => {
      cancelled = true;
    };
  }, [version]);

  return model;
}
