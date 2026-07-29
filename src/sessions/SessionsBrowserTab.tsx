import { useMemo, useState } from 'react';
import type { SessionSummary } from '../lib/generated/bindings';
import { Banner } from '../components/ui/Banner';
import { Card } from '../components/ui/Card';
import { EmptyState } from '../components/ui/EmptyState';
import { IconSessions, Search } from '../lib/icons';
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

/** Rows share one bordered surface with hairline dividers, the way the Cost
 *  tab lists sessions. One card per row — the previous shape — put a border
 *  and a gap around every single line, which is what made the list read as
 *  undifferentiated blocks instead of a scannable column. */
function RowGroup({ children }: { children: React.ReactNode }) {
  return (
    <Card className="overflow-hidden [&>*+*]:border-t [&>*+*]:border-[var(--color-border-subtle)]">
      {children}
    </Card>
  );
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
        showProject={Boolean(q)}
        onToggle={toggle}
        onResume={() => resume(s)}
      />
    ));

  return (
    <div className="flex flex-col gap-[var(--space-md)]">
      <div className="flex items-center gap-[var(--space-md)]">
        <div className="relative min-w-0 flex-1">
          <Search
            size={13}
            aria-hidden
            className="
              pointer-events-none absolute left-[var(--space-sm)] top-1/2 -translate-y-1/2
              text-[color:var(--color-text-muted)]
            "
          />
          <input
            aria-label="Search sessions"
            placeholder="Search titles, recaps, files…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            className="
              w-full rounded-[var(--radius-sm)]
              border border-[var(--color-border)] bg-[var(--color-bg-card)]
              py-[var(--space-xs)] pl-[26px] pr-[var(--space-sm)]
              text-[length:var(--text-body)] text-[color:var(--color-text)]
              placeholder:text-[color:var(--color-text-muted)]
              transition-[border-color] duration-[var(--duration-fast)]
              focus:border-[var(--color-border-focus)] focus:outline-none
            "
          />
        </div>
        {sessions.length > 0 && (
          <span className="mono shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
            {filtered.length} of {sessions.length}
          </span>
        )}
      </div>

      {error && <Banner variant="error">{error}</Banner>}
      {notice && (
        <Banner variant="warning" role="status">
          {notice}
        </Banner>
      )}

      {loading && sessions.length === 0 && (
        <p className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">
          Loading…
        </p>
      )}

      {!loading && !error && sessions.length === 0 && (
        <EmptyState
          icon={<IconSessions size={32} />}
          title="No sessions yet"
          description="Sessions you run with Claude Code will appear here."
        />
      )}
      {!loading && sessions.length > 0 && filtered.length === 0 && (
        <p className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">
          No sessions match “{query}”.
        </p>
      )}

      {groups
        ? groups.map(([project, list]) => (
            <section key={project} className="flex flex-col gap-[var(--space-xs)]">
              <header className="flex items-baseline gap-[var(--space-sm)] px-[2px]">
                <h3
                  className="
                    truncate text-[length:var(--text-label)] font-[var(--weight-semibold)]
                    uppercase tracking-[var(--tracking-label)]
                    text-[color:var(--color-text-secondary)]
                  "
                >
                  {project}
                </h3>
                <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
                  {list.length}
                </span>
              </header>
              <RowGroup>{rows(list)}</RowGroup>
            </section>
          ))
        : filtered.length > 0 && <RowGroup>{rows(filtered)}</RowGroup>}

      {dialog}
    </div>
  );
}
