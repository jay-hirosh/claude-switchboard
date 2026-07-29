import { useMemo, useState } from 'react';
import type { SessionSummary } from '../lib/generated/bindings';
import { EmptyState } from '../components/ui/EmptyState';
import { useResumableSessions } from './useResumableSessions';
import { SessionRow } from './SessionRow';
import { useResume } from './useResume';

function matches(s: SessionSummary, q: string): boolean {
  const hay = [
    s.title,
    s.project_name,
    s.git_branch ?? '',
    s.model ?? '',
    s.recap ?? '',
    s.asked,
    s.left_off ?? '',
    ...s.touched_files,
  ]
    .join(' ')
    .toLowerCase();
  return hay.includes(q);
}

export function SessionsBrowserTab() {
  const { sessions, loading, error } = useResumableSessions();
  const [query, setQuery] = useState('');
  const [expanded, setExpanded] = useState<string | null>(null);
  const { resume, dialog, notice } = useResume();

  const q = query.trim().toLowerCase();
  const filtered = useMemo(
    () => (q ? sessions.filter((s) => matches(s, q)) : sessions),
    [sessions, q],
  );

  // Search flattens the grouping: a two-result query should not be split
  // across two project headers.
  const groups = useMemo(() => {
    if (q) return null;
    const by = new Map<string, SessionSummary[]>();
    for (const s of filtered) {
      const list = by.get(s.project_name) ?? [];
      list.push(s);
      by.set(s.project_name, list);
    }
    return [...by.entries()];
  }, [filtered, q]);

  function toggle(id: string) {
    setExpanded((cur) => (cur === id ? null : id));
  }

  const rows = (list: SessionSummary[]) =>
    list.map((s) => (
      <SessionRow
        key={s.session_id}
        session={s}
        expanded={expanded === s.session_id}
        onToggle={toggle}
        onResume={() => resume(s)}
      />
    ));

  return (
    <div className="flex flex-col gap-[var(--space-sm)] p-[var(--space-md)]">
      <input
        aria-label="Search sessions"
        placeholder="Search sessions…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        className="w-full rounded-[var(--radius-sm)] border border-[var(--color-border)] bg-[var(--color-bg-base)] px-[var(--space-xs)] py-[var(--space-2xs)] text-[length:var(--text-body)] text-[color:var(--color-text)]"
      />

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

      {!loading && !error && sessions.length === 0 && (
        <EmptyState
          title="No sessions yet"
          description="Sessions you run with Claude Code will appear here."
        />
      )}
      {!loading && sessions.length > 0 && filtered.length === 0 && (
        <p className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          No sessions match “{query}”.
        </p>
      )}

      {groups
        ? groups.map(([project, list]) => (
            <section key={project} className="flex flex-col gap-[var(--space-2xs)]">
              <h3 className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                {project}
              </h3>
              {rows(list)}
            </section>
          ))
        : rows(filtered)}

      {dialog}
    </div>
  );
}
