import { useMemo, useState } from 'react';
import type { SessionSummary } from '../lib/generated/bindings';
import { Banner } from '../components/ui/Banner';
import { Card } from '../components/ui/Card';
import { EmptyState } from '../components/ui/EmptyState';
import { ArrowUpDown, ChevronDown, ChevronRight, IconSessions, Search } from '../lib/icons';
import { durationMinutes, formatCost, formatDurationMinutes, formatTokens } from '../lib/format';
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

type SortMode = 'recent' | 'cost';

export function SessionsBrowserTab() {
  const { sessions, loading, error } = useResumableSessions();
  const [query, setQuery] = useState('');
  const [sortMode, setSortMode] = useState<SortMode>('recent');
  const [expanded, setExpanded] = useState<string | null>(null);
  // Project groups collapse by default — a project header should read as a
  // total, not a promise to scroll through every session inside it.
  const [openProjects, setOpenProjects] = useState<Set<string>>(new Set());
  const { resume, dialog, notice, vsCodeAvailable } = useResume();

  const q = query.trim().toLowerCase();
  const filtered = useMemo(() => {
    const base = q ? sessions.filter((s) => matches(s, q)) : sessions;
    if (sortMode === 'recent') return base;
    return [...base].sort((a, b) => b.total_cost_usd - a.total_cost_usd);
  }, [sessions, q, sortMode]);

  // Aggregate across whatever is currently listed, so a search narrows the
  // totals along with the rows.
  const totals = useMemo(
    () =>
      filtered.reduce(
        (acc, s) => ({
          tokens: acc.tokens + s.total_tokens,
          cost: acc.cost + s.total_cost_usd,
          minutes: acc.minutes + durationMinutes(s.started_at, s.ended_at),
        }),
        { tokens: 0, cost: 0, minutes: 0 },
      ),
    [filtered],
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
    const result = [...by.entries()].map(([project, list]) => ({
      project,
      list,
      tokens: list.reduce((sum, s) => sum + s.total_tokens, 0),
      cost: list.reduce((sum, s) => sum + s.total_cost_usd, 0),
      minutes: list.reduce((sum, s) => sum + durationMinutes(s.started_at, s.ended_at), 0),
    }));
    if (sortMode === 'cost') result.sort((a, b) => b.cost - a.cost);
    return result;
  }, [filtered, q, sortMode]);

  function toggle(id: string) {
    setExpanded((cur) => (cur === id ? null : id));
  }

  function toggleProject(project: string) {
    setOpenProjects((cur) => {
      const next = new Set(cur);
      if (next.has(project)) next.delete(project);
      else next.add(project);
      return next;
    });
  }

  const rows = (list: SessionSummary[]) =>
    list.map((s) => (
      <SessionRow
        key={s.session_id}
        session={s}
        expanded={expanded === s.session_id}
        showProject={Boolean(q)}
        vsCodeAvailable={vsCodeAvailable}
        onToggle={toggle}
        onResume={(_id, surface) => resume(s, surface)}
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
          <>
            <button
              type="button"
              onClick={() => setSortMode((m) => (m === 'recent' ? 'cost' : 'recent'))}
              className="
                flex shrink-0 items-center gap-[var(--space-xs)] rounded-[var(--radius-sm)]
                border border-[var(--color-border)] bg-[var(--color-bg-card)]
                px-[var(--space-sm)] py-[var(--space-xs)]
                text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]
                transition-[opacity] duration-[var(--duration-fast)]
                hover:opacity-80
              "
              title="Toggle sort order"
            >
              <ArrowUpDown size={11} aria-hidden />
              {sortMode === 'recent' ? 'Recent' : 'Cost'}
            </button>
            <span className="mono shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
              {filtered.length} of {sessions.length}
            </span>
          </>
        )}
      </div>

      {filtered.length > 0 && (
        <div className="flex items-center gap-[var(--space-md)] px-[2px]">
          <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
            Total
          </span>
          <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
            {formatDurationMinutes(totals.minutes)}
          </span>
          <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
            {formatTokens(totals.tokens)} tokens
          </span>
          <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)] tabular-nums">
            {formatCost(totals.cost)}
          </span>
        </div>
      )}

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
        ? groups.map(({ project, list, tokens, cost, minutes }) => {
            const isOpen = openProjects.has(project);
            const Chevron = isOpen ? ChevronDown : ChevronRight;
            return (
              <section key={project} className="flex flex-col gap-[var(--space-xs)]">
                <button
                  type="button"
                  onClick={() => toggleProject(project)}
                  aria-expanded={isOpen}
                  className="
                    flex items-center gap-[var(--space-sm)] px-[2px] py-[2px]
                    text-left transition-[opacity] duration-[var(--duration-fast)]
                    hover:opacity-80
                  "
                >
                  <Chevron
                    size={12}
                    aria-hidden
                    className="shrink-0 text-[color:var(--color-text-muted)]"
                  />
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
                  <span className="flex-1" />
                  <span className="mono shrink-0 min-w-[40px] text-right text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
                    {formatDurationMinutes(minutes)}
                  </span>
                  <span className="mono shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
                    {formatTokens(tokens)}
                  </span>
                  <span className="mono shrink-0 min-w-[46px] text-right text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)] tabular-nums">
                    {formatCost(cost)}
                  </span>
                </button>
                {isOpen && <RowGroup>{rows(list)}</RowGroup>}
              </section>
            );
          })
        : filtered.length > 0 && <RowGroup>{rows(filtered)}</RowGroup>}

      {dialog}
    </div>
  );
}
