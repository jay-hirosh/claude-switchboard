import { useMemo, useState } from 'react';
import { Card } from '../components/ui/Card';
import { Button } from '../components/ui/Button';
import { EmptyState } from '../components/ui/EmptyState';
import { AccountBadge } from '../components/ui/AccountBadge';
import { ModelBadge } from '../components/ui/ModelBadge';
import { Badge } from '../components/ui/Badge';
import { IconButton } from '../components/ui/IconButton';
import { formatCost, formatTokens } from '../lib/format';
import { IconTrends, IconExport, GitBranch, ChevronDown, ChevronRight } from '../lib/icons';
import { ipc } from '../lib/ipc';
import { useTabData } from '../lib/useTabData';
import { useAppStore } from '../lib/store';
import { localDayKey, weekStartDayKey } from '../lib/dayKey';
import type { DailyPatternReport, RepoStats, CacheStats, SessionEvent } from '../lib/types';
import type { AccountListEntry } from '../lib/generated/bindings';
import { aggregateSessions, isHeadlessProject, formatClock, type AggregatedSession } from './SessionsTab';
import { MODEL_VARIANT, modelKey, shortName } from './modelDisplay';
import { HourlyStrip } from './trends/HourlyStrip';

const MAX_SESSION_ROWS = 50;

// Covers today, yesterday, and the 7-day "This Week" window with buffer for
// the local-vs-UTC offset near midnight.
const SESSION_HISTORY_DAYS = 8;

const MODEL_BAR_COLORS: Record<string, string> = {
  opus: 'var(--color-model-opus)',
  sonnet: 'var(--color-model-sonnet)',
  haiku: 'var(--color-model-haiku)',
};

type PeriodKey = 'today' | 'yesterday' | 'week';

const PERIOD_SWITCHES: { key: PeriodKey; label: string }[] = [
  { key: 'today', label: 'Today' },
  { key: 'yesterday', label: 'Yesterday' },
  { key: 'week', label: 'This Week' },
];

interface PeriodModelStat {
  model: string;
  tokens: number;
  cost_usd: number;
}

/** Folds a period's events by model. `cost_usd` is summed straight from each
 * event's own (already backend-priced) field — no pricing table needed. */
function foldByModel(events: { model: string; input_tokens: number; output_tokens: number; cost_usd: number }[]): PeriodModelStat[] {
  const byModel = new Map<string, PeriodModelStat>();
  for (const e of events) {
    const tokens = e.input_tokens + e.output_tokens;
    const existing = byModel.get(e.model);
    if (existing) {
      existing.tokens += tokens;
      existing.cost_usd += e.cost_usd;
    } else {
      byModel.set(e.model, { model: e.model, tokens, cost_usd: e.cost_usd });
    }
  }
  return [...byModel.values()].sort((a, b) => b.cost_usd - a.cost_usd);
}

interface PeriodConfig {
  key: PeriodKey;
  label: string;
  pattern: DailyPatternReport;
  repos: RepoStats[];
  cache: CacheStats;
  events: SessionEvent[];
}

export function DashboardTab() {
  const [selectedPeriod, setSelectedPeriod] = useState<PeriodKey>('today');
  const version = useAppStore((s) => s.sessionDataVersion);
  const accounts = useAppStore((s) => s.accounts);
  const { data, error, loading, reload } = useTabData(
    () =>
      Promise.all([
        ipc.getSessionHistory(SESSION_HISTORY_DAYS),
        ipc.getTodayPattern(),
        ipc.getTodayRepoBreakdown(),
        ipc.getTodayCacheStats(),
        ipc.getYesterdayPattern(),
        ipc.getYesterdayRepoBreakdown(),
        ipc.getYesterdayCacheStats(),
        ipc.getWeekPattern(),
        ipc.getWeekRepoBreakdown(),
        ipc.getWeekCacheStats(),
      ]).then(
        ([
          events,
          todayPattern,
          todayRepos,
          todayCache,
          yesterdayPattern,
          yesterdayRepos,
          yesterdayCache,
          weekPattern,
          weekRepos,
          weekCache,
        ]) => ({
          events,
          todayPattern,
          todayRepos,
          todayCache,
          yesterdayPattern,
          yesterdayRepos,
          yesterdayCache,
          weekPattern,
          weekRepos,
          weekCache,
        }),
      ),
    [version],
  );

  const todayKey = localDayKey(new Date().toISOString());
  const yesterdayKey = useMemo(() => {
    const d = new Date();
    d.setDate(d.getDate() - 1);
    return localDayKey(d.toISOString());
  }, []);
  const weekStartKey = useMemo(() => weekStartDayKey(), []);

  const events = data?.events ?? [];
  const todayEvents = useMemo(() => events.filter((e) => localDayKey(e.ts) === todayKey), [events, todayKey]);
  const yesterdayEvents = useMemo(() => events.filter((e) => localDayKey(e.ts) === yesterdayKey), [events, yesterdayKey]);
  const weekEvents = useMemo(() => events.filter((e) => localDayKey(e.ts) >= weekStartKey), [events, weekStartKey]);

  if (error) {
    return (
      <EmptyState
        icon={<IconTrends size={32} />}
        title="Couldn't load dashboard activity"
        description={error}
        action={<Button variant="ghost" size="sm" onClick={reload}>Retry</Button>}
      />
    );
  }
  if (loading || !data) {
    return <p className="text-[color:var(--color-text-muted)]">Loading…</p>;
  }

  const periods: PeriodConfig[] = [
    { key: 'today', label: 'Today', pattern: data.todayPattern, repos: data.todayRepos, cache: data.todayCache, events: todayEvents },
    { key: 'yesterday', label: 'Yesterday', pattern: data.yesterdayPattern, repos: data.yesterdayRepos, cache: data.yesterdayCache, events: yesterdayEvents },
    { key: 'week', label: 'This Week', pattern: data.weekPattern, repos: data.weekRepos, cache: data.weekCache, events: weekEvents },
  ];
  const activePeriod = periods.find((p) => p.key === selectedPeriod) ?? periods[0];

  return (
    <div className="flex flex-col gap-[var(--space-2xl)]">
      <h1 className="hidden print:block text-[length:var(--text-title)] font-[var(--weight-semibold)] text-[color:var(--color-text)]">
        Claude Switchboard · {activePeriod.label} · {new Date().toLocaleDateString('en-US', { month: 'long', day: 'numeric', year: 'numeric' })}
      </h1>
      <div className="flex items-center justify-between print:hidden">
        <div className="flex gap-[var(--space-2xs)] bg-[var(--color-track)] rounded-[var(--radius-sm)] p-[2px] w-fit">
          {PERIOD_SWITCHES.map((p) => (
            <button
              key={p.key}
              type="button"
              onClick={() => setSelectedPeriod(p.key)}
              aria-pressed={selectedPeriod === p.key}
              data-testid={`period-switch-${p.key}`}
              className={[
                'px-[var(--space-sm)] py-[var(--space-2xs)]',
                'text-[length:var(--text-label)] font-[var(--weight-medium)]',
                'rounded-[var(--radius-sm)]',
                'transition-[background,color] duration-[var(--duration-fast)]',
                selectedPeriod === p.key
                  ? 'bg-[var(--color-bg-card)] text-[color:var(--color-text)]'
                  : 'text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text-secondary)]',
              ].join(' ')}
            >
              {p.label}
            </button>
          ))}
        </div>
        <IconButton label="Export PDF" onClick={() => window.print()}>
          <IconExport size={13} />
        </IconButton>
      </div>
      <PeriodSection period={activePeriod} accounts={accounts} />
    </div>
  );
}

function PeriodSection({ period, accounts }: { period: PeriodConfig; accounts: AccountListEntry[] }) {
  const sessions = useMemo(() => aggregateSessions(period.events, null), [period.events]);
  const totalCost = useMemo(() => sessions.reduce((s, r) => s + r.total_cost_usd, 0), [sessions]);
  const totalTokens = useMemo(() => sessions.reduce((s, r) => s + r.headline_tokens, 0), [sessions]);
  const modelStats = useMemo(() => foldByModel(period.events), [period.events]);

  return (
    <section className="flex flex-col gap-[var(--space-md)]">
      {period.events.length === 0 ? (
        <EmptyState
          icon={<IconTrends size={24} />}
          title={period.key === 'today' ? 'No activity yet today' : `No activity ${period.label.toLowerCase()}`}
        />
      ) : (
        <div className="flex flex-col gap-[var(--space-lg)]">
          <HeadlineRow periodKey={period.key} cost={totalCost} tokens={totalTokens} sessionCount={sessions.length} />
          <HourlySection pattern={period.pattern} />
          <SessionsSection periodKey={period.key} sessions={sessions} accounts={accounts} />
          <RepoSection repos={period.repos} accounts={accounts} />
          <ModelSection stats={modelStats} />
          <CacheSection cache={period.cache} />
        </div>
      )}
    </section>
  );
}

function HeadlineRow({ periodKey, cost, tokens, sessionCount }: { periodKey: PeriodKey; cost: number; tokens: number; sessionCount: number }) {
  return (
    <div className="grid grid-cols-3 gap-[var(--space-sm)]">
      <Card className="p-[var(--space-md)] flex flex-col gap-[4px]">
        <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">Cost</span>
        <span
          data-testid={`${periodKey}-cost`}
          className="mono text-[length:var(--text-title)] font-[var(--weight-semibold)] text-[color:var(--color-text)]"
        >
          {formatCost(cost)}
        </span>
      </Card>
      <Card className="p-[var(--space-md)] flex flex-col gap-[4px]">
        <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">Tokens</span>
        <span
          data-testid={`${periodKey}-tokens`}
          className="mono text-[length:var(--text-title)] font-[var(--weight-semibold)] text-[color:var(--color-text)]"
        >
          {formatTokens(tokens)}
        </span>
      </Card>
      <Card className="p-[var(--space-md)] flex flex-col gap-[4px]">
        <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">Sessions</span>
        <span
          data-testid={`${periodKey}-sessions`}
          className="mono text-[length:var(--text-title)] font-[var(--weight-semibold)] text-[color:var(--color-text)]"
        >
          {sessionCount}
        </span>
      </Card>
    </div>
  );
}

function HourlySection({ pattern }: { pattern: DailyPatternReport }) {
  return (
    <div className="flex flex-col gap-[var(--space-sm)]">
      <span className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text-muted)] px-[var(--space-2xs)]">
        Hourly activity
      </span>
      <Card className="p-[var(--space-md)] flex flex-col gap-[var(--space-xs)]">
        <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">Tokens by hour</span>
        <HourlyStrip metric="tokens" totals={pattern.hourly_totals} windows={[]} />
      </Card>
      <Card className="p-[var(--space-md)] flex flex-col gap-[var(--space-xs)]">
        <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">Cost by hour</span>
        <HourlyStrip metric="cost" totals={pattern.hourly_totals} windows={[]} />
      </Card>
    </div>
  );
}

/** Collapsed by default — the sessions list is the longest section in a
 * period block and dominates the tab unless the reader opts in. The body
 * stays mounted and is hidden with a CSS class rather than unmounted, so the
 * print stylesheet can force it visible for PDF export regardless of the
 * on-screen collapsed state. */
function SessionsSection({ periodKey, sessions, accounts }: { periodKey: PeriodKey; sessions: AggregatedSession[]; accounts: AccountListEntry[] }) {
  const [open, setOpen] = useState(false);
  const shown = sessions.slice(0, MAX_SESSION_ROWS);
  const Chevron = open ? ChevronDown : ChevronRight;
  return (
    <div className="flex flex-col gap-[var(--space-sm)]">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        data-testid={`${periodKey}-sessions-toggle`}
        className="flex items-center gap-[var(--space-2xs)] px-[var(--space-2xs)] text-left cursor-default print:hidden"
      >
        <Chevron size={12} className="shrink-0 text-[color:var(--color-text-muted)]" />
        <span className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text-muted)]">
          Sessions ({sessions.length})
        </span>
      </button>
      <div
        data-testid={`${periodKey}-sessions-body`}
        className={['flex flex-col gap-[var(--space-sm)]', open ? '' : 'hidden print:flex'].join(' ')}
      >
        <Card className="flex flex-col [&>*+*]:border-t [&>*+*]:border-[var(--color-border-subtle)]">
          {shown.map((session) => {
            const headless = isHeadlessProject(session.project);
            return (
              <div
                key={session.id}
                data-testid={`${periodKey}-session-row`}
                className={[
                  'flex items-center gap-[var(--space-sm)] px-[var(--space-sm)] py-[var(--space-sm)]',
                  headless ? 'opacity-55' : '',
                ].join(' ')}
              >
                <div className="flex flex-col min-w-0 flex-1">
                  <div className="flex items-center gap-[var(--space-sm)]">
                    <span
                      className={[
                        'text-[length:var(--text-body)] truncate',
                        headless ? 'italic text-[color:var(--color-text-muted)]' : 'text-[color:var(--color-text)]',
                      ].join(' ')}
                    >
                      {headless ? 'headless' : session.project}
                    </span>
                    <ModelBadge model={session.dominant_model} />
                    {session.account_uuids.map((uuid) => (
                      <AccountBadge key={uuid ?? 'unknown'} accountUuid={uuid} accounts={accounts} />
                    ))}
                  </div>
                  <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                    {formatClock(session.latest_ts)} · {session.turn_count} {session.turn_count === 1 ? 'turn' : 'turns'}
                  </span>
                </div>
                <div className="flex items-center gap-[var(--space-md)] shrink-0">
                  <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)] tabular-nums">
                    {formatTokens(session.headline_tokens)}
                  </span>
                  <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-muted)] tabular-nums min-w-[48px] text-right">
                    {formatCost(session.total_cost_usd)}
                  </span>
                </div>
              </div>
            );
          })}
        </Card>
        {sessions.length > MAX_SESSION_ROWS && (
          <div className="text-center text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
            Showing the latest {MAX_SESSION_ROWS} of {sessions.length} sessions.
          </div>
        )}
      </div>
    </div>
  );
}

function ModelSection({ stats }: { stats: PeriodModelStat[] }) {
  const totalTokens = stats.reduce((s, r) => s + r.tokens, 0);
  return (
    <div className="flex flex-col gap-[var(--space-sm)]">
      <span className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text-muted)] px-[var(--space-2xs)]">
        By model
      </span>
      <Card className="p-[var(--space-sm)] flex flex-col gap-[var(--space-sm)]">
        {stats.map((r) => {
          const key = modelKey(r.model);
          const pct = totalTokens > 0 ? (r.tokens / totalTokens) * 100 : 0;
          return (
            <div key={r.model} className="flex items-center gap-[var(--space-sm)]">
              <Badge variant={MODEL_VARIANT[key] ?? 'default'}>{shortName(r.model)}</Badge>
              <div className="flex-1 h-[6px] rounded-[var(--radius-pill)] bg-[var(--color-track)] overflow-hidden">
                <div
                  className="h-full rounded-[var(--radius-pill)] transition-[width] duration-[var(--duration-bar)] ease-[var(--ease-spring)]"
                  style={{ width: `${pct}%`, background: MODEL_BAR_COLORS[key] ?? 'var(--color-text-muted)' }}
                />
              </div>
              <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-muted)] tabular-nums min-w-[48px] text-right">
                {formatTokens(r.tokens)}
              </span>
              <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)] tabular-nums min-w-[48px] text-right">
                {formatCost(r.cost_usd)}
              </span>
            </div>
          );
        })}
      </Card>
    </div>
  );
}

function RepoSection({ repos, accounts }: { repos: RepoStats[]; accounts: AccountListEntry[] }) {
  if (repos.length === 0) return null;
  const maxCost = Math.max(...repos.map((r) => r.total_cost_usd));
  return (
    <div className="flex flex-col gap-[var(--space-sm)]">
      <span className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text-muted)] px-[var(--space-2xs)]">
        By repo
      </span>
      {repos.map((repo) => {
        const widthPct = (repo.total_cost_usd / maxCost) * 100;
        return (
          <Card key={repo.repo} className="p-[var(--space-md)] flex flex-col gap-[var(--space-sm)]">
            <div className="flex items-center justify-between gap-[var(--space-sm)]">
              <div className="flex min-w-0 items-center gap-[var(--space-sm)]">
                <GitBranch size={13} className="shrink-0 text-[color:var(--color-text-muted)]" />
                <span className="truncate text-[length:var(--text-body)] font-[var(--weight-medium)] text-[color:var(--color-text)]">
                  {repo.repo}
                </span>
                <span className="shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                  {repo.session_count} session{repo.session_count === 1 ? '' : 's'}
                </span>
                <div className="flex shrink-0 gap-[var(--space-2xs)]">
                  {repo.account_uuids.map((uuid) => (
                    <AccountBadge key={uuid ?? 'unknown'} accountUuid={uuid} accounts={accounts} />
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
                className="h-full rounded-[var(--radius-pill)] bg-[var(--color-accent)]"
                style={{ width: `${widthPct}%` }}
              />
            </div>
          </Card>
        );
      })}
    </div>
  );
}

function CacheSection({ cache }: { cache: CacheStats }) {
  const totalCacheTokens = cache.total_cache_read_tokens + cache.total_cache_creation_tokens;
  if (totalCacheTokens === 0) return null;
  const hitRatePct = cache.hit_ratio * 100;
  return (
    <div className="flex flex-col gap-[var(--space-sm)]">
      <span className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-text-muted)] px-[var(--space-2xs)]">
        Cache
      </span>
      <Card className="p-[var(--space-md)] flex items-center gap-[var(--space-md)]">
        <div className="flex flex-col">
          <span className="mono text-[length:var(--text-title)] font-[var(--weight-semibold)] text-[color:var(--color-accent)]">
            {Math.round(hitRatePct)}%
          </span>
          <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">hit rate</span>
        </div>
        <div className="flex-1 grid grid-cols-3 gap-[var(--space-sm)]">
          <div className="flex flex-col gap-[2px]">
            <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">Reads</span>
            <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text)]">
              {formatTokens(cache.total_cache_read_tokens)}
            </span>
          </div>
          <div className="flex flex-col gap-[2px]">
            <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">Writes</span>
            <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text)]">
              {formatTokens(cache.total_cache_creation_tokens)}
            </span>
          </div>
          <div className="flex flex-col gap-[2px]">
            <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">Savings</span>
            <span className="mono text-[length:var(--text-label)] text-[color:var(--color-safe)]">
              ${cache.estimated_savings_usd.toFixed(2)}
            </span>
          </div>
        </div>
      </Card>
    </div>
  );
}
