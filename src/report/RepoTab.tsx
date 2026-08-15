import { useState } from 'react';
import { AccountBadge } from '../components/ui/AccountBadge';
import { Card } from '../components/ui/Card';
import { Button } from '../components/ui/Button';
import { EmptyState } from '../components/ui/EmptyState';
import { formatCost, formatTokens } from '../lib/format';
import { ChevronDown, ChevronRight, GitBranch, IconChart } from '../lib/icons';
import { ipc } from '../lib/ipc';
import { useTabData } from '../lib/useTabData';
import { useAppStore } from '../lib/store';

export function RepoTab() {
  const version = useAppStore((s) => s.sessionDataVersion);
  const accounts = useAppStore((s) => s.accounts);
  const { data, error, loading, reload } = useTabData(() => ipc.getRepoBreakdown(), [version]);
  const [expanded, setExpanded] = useState<string | null>(null);

  if (error) {
    return (
      <EmptyState
        icon={<IconChart size={32} />}
        title="Couldn't load repositories"
        description={error}
        action={<Button variant="ghost" size="sm" onClick={reload}>Retry</Button>}
      />
    );
  }
  if (loading || !data) {
    return <p className="text-[color:var(--color-text-muted)]">Loading…</p>;
  }

  if (data.length === 0) {
    return (
      <EmptyState
        icon={<GitBranch size={32} />}
        title="No repository data"
        description="Repositories will appear as you use Claude Code in different git checkouts."
      />
    );
  }

  const maxCost = Math.max(...data.map((r) => r.total_cost_usd), 1);

  return (
    <div className="flex flex-col gap-[var(--space-sm)]">
      {data.map((repo) => {
        const widthPct = (repo.total_cost_usd / maxCost) * 100;
        const isOpen = expanded === repo.repo;
        // A repo worked from a single directory has nothing to disclose —
        // its one project is already the row itself.
        const expandable = repo.projects.length > 1;

        return (
          <Card key={repo.repo} className="overflow-hidden">
            <button
              type="button"
              onClick={() => expandable && setExpanded(isOpen ? null : repo.repo)}
              aria-expanded={expandable ? isOpen : undefined}
              className={[
                'flex w-full flex-col gap-[var(--space-sm)] p-[var(--space-md)] text-left',
                expandable ? 'cursor-default' : 'cursor-default',
              ].join(' ')}
            >
              <div className="flex items-center justify-between gap-[var(--space-sm)]">
                <div className="flex min-w-0 items-center gap-[var(--space-sm)]">
                  {expandable ? (
                    isOpen ? (
                      <ChevronDown size={13} className="shrink-0 text-[color:var(--color-text-muted)]" />
                    ) : (
                      <ChevronRight size={13} className="shrink-0 text-[color:var(--color-text-muted)]" />
                    )
                  ) : (
                    <GitBranch size={13} className="shrink-0 text-[color:var(--color-text-muted)]" />
                  )}
                  <span className="truncate text-[length:var(--text-body)] font-[var(--weight-medium)] text-[color:var(--color-text)]">
                    {repo.repo}
                  </span>
                  <span className="shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                    {repo.session_count} session{repo.session_count === 1 ? '' : 's'}
                    {repo.projects.length > 1 ? ` · ${repo.projects.length} projects` : ''}
                  </span>
                  <div className="flex shrink-0 gap-[var(--space-2xs)]">
                    {repo.account_uuids.map((uuid) => (
                      <AccountBadge key={uuid} accountUuid={uuid} accounts={accounts} />
                    ))}
                  </div>
                </div>
                <div className="flex shrink-0 items-baseline gap-[var(--space-sm)]">
                  <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
                    {formatTokens(repo.total_tokens)} tokens
                  </span>
                  <span className="mono text-[length:var(--text-label)] font-[var(--weight-semibold)] text-[color:var(--color-text)] tabular-nums">
                    {formatCost(repo.total_cost_usd)}
                  </span>
                </div>
              </div>

              <div className="flex h-[6px] rounded-[var(--radius-pill)] bg-[var(--color-track)] overflow-hidden">
                <div
                  className="h-full rounded-[var(--radius-pill)] bg-[var(--color-accent)] transition-[width] duration-[var(--duration-bar)]"
                  style={{ width: `${widthPct}%` }}
                />
              </div>
            </button>

            {expandable && isOpen && (
              <div className="border-t border-[var(--color-border-subtle)] [&>*+*]:border-t [&>*+*]:border-[var(--color-border-subtle)]">
                {repo.projects.map((p) => (
                  <div
                    key={p.cwd}
                    className="flex items-center justify-between gap-[var(--space-sm)] px-[var(--space-md)] py-[var(--space-sm)] pl-[calc(var(--space-md)+21px)]"
                  >
                    <div className="flex min-w-0 flex-col">
                      <span className="truncate text-[length:var(--text-label)] text-[color:var(--color-text-secondary)]">
                        {p.project}
                      </span>
                      <span className="truncate text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                        {p.cwd}
                      </span>
                    </div>
                    <div className="flex shrink-0 items-baseline gap-[var(--space-sm)]">
                      <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums">
                        {formatTokens(p.total_tokens)} tokens
                      </span>
                      <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)] tabular-nums">
                        {formatCost(p.total_cost_usd)}
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </Card>
        );
      })}
    </div>
  );
}
