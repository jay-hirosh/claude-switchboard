import { useEffect, useMemo, useState } from 'react';
import { Button } from '../components/ui/Button';
import { ModelBadge } from '../components/ui/ModelBadge';
import { EmptyState } from '../components/ui/EmptyState';
import { formatTokens, formatCost } from '../lib/format';
import { ChevronDown, ChevronRight, IconSessions, IconSubagent } from '../lib/icons';
import { ipc } from '../lib/ipc';
import { useTabData } from '../lib/useTabData';
import { useAppStore } from '../lib/store';
import type { PricingEntry, SessionEvent } from '../lib/types';
import { costPerCategory, lookupPricing } from '../lib/pricing';

// `modelLabel` now lives in ./modelDisplay alongside modelKey/MODEL_VARIANT so
// the shared <ModelBadge> can reach it without importing a tab component.
// Re-exported here because this module was its original public home.
export { modelLabel } from './modelDisplay';

/** A session whose transcript had no working directory (`cwd` absent) lands
 * in ~/.claude/projects/-/ and resolves to project "-". These are headless /
 * scheduled `claude` invocations (regular cadence, 1 turn, tiny tokens), not
 * interactive work — so relabel them "headless" and visually demote the row
 * instead of showing a bare "-" that reads as a broken session. */
export function isHeadlessProject(project: string): boolean {
  return !project || project === '-';
}

function formatTime(iso: string): string {
  const d = new Date(iso);
  const now = new Date();
  const isToday = d.toDateString() === now.toDateString();
  const time = d.toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
  if (isToday) return time;
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' }) + ' ' + time;
}

type Breakdown = {
  input: number;
  output: number;
  cache_read: number;
  cache_write_5m: number;
  cache_write_1h: number;
};

/** A subagent (Task/Agent tool) transcript rolled under its parent session.
 * Claude Code writes these to `<project>/<sessionId>/subagents/agent-*.jsonl`.
 * They share the parent conversation's `cwd`, so without nesting every
 * subagent would render as a duplicate top-level session row. */
interface SubagentSession {
  id: string;
  label: string;
  latest_ts: string;
  turn_count: number;
  headline_tokens: number;
  total_cost_usd: number;
  dominant_model: string;
}

interface AggregatedSession {
  id: string;
  project: string;
  latest_ts: string;
  turn_count: number;
  /** Headline tokens shown on the collapsed row: input + output only.
   * Combined across the parent and all of its subagents — the row
   * represents the whole conversation, and subagents' API calls are real
   * usage the parent transcript does not include. Cache totals (which can
   * be 10–100x larger but mostly reuse) are surfaced in the expandable
   * breakdown. */
  headline_tokens: number;
  total_cost_usd: number;
  /** Model with the most output tokens — surfaced via the row's badge. */
  dominant_model: string;
  /** Per-category token totals for the expandable breakdown (combined). */
  breakdown: Breakdown;
  /** Per-category cost contributions, summed over all turns. Mirrors the
   * backend cost_for logic so totals match what's billed. */
  cost_breakdown: Breakdown;
  /** Subagent transcripts spawned by this conversation, in chronological
   * order. Empty for sessions that never used the Task/Agent tool. */
  subagents: SubagentSession[];
}

const EMPTY_BREAKDOWN: Breakdown = {
  input: 0,
  output: 0,
  cache_read: 0,
  cache_write_5m: 0,
  cache_write_1h: 0,
};

/** Subagent transcripts live at `<project>/<sessionId>/subagents/agent-*.jsonl`.
 * Detect that path shape and fold them under their parent's key. */
const SUBAGENT_SEGMENT = '/subagents/agent-';

function parentKeyOf(sourceFile: string): string {
  const idx = sourceFile.indexOf('/subagents/');
  // The sibling parent transcript is `<sessionId>.jsonl` in the project dir,
  // i.e. everything before `/subagents/` plus a `.jsonl` suffix.
  return idx < 0 ? sourceFile : sourceFile.slice(0, idx) + '.jsonl';
}

function subagentLabel(sourceFile: string): string {
  const m = sourceFile.match(/agent-([0-9a-f]+)/i);
  return m ? `agent ${m[1].slice(0, 8)}` : 'subagent';
}

function pickDominant(modelTokens: Map<string, number>, fallback: string): string {
  let best = fallback;
  let bestTokens = -1;
  for (const [model, tokens] of modelTokens) {
    if (tokens > bestTokens) {
      bestTokens = tokens;
      best = model;
    }
  }
  return best;
}

interface ParentAgg {
  project: string;
  latest_ts: string;
  turn_count: number;
  headline_tokens: number;
  total_cost_usd: number;
  dominant_model: string;
  breakdown: Breakdown;
  cost_breakdown: Breakdown;
  modelTokens: Map<string, number>;
  subs: Map<string, SubAgg>;
}

interface SubAgg {
  label: string;
  latest_ts: string;
  turn_count: number;
  headline_tokens: number;
  total_cost_usd: number;
  modelTokens: Map<string, number>;
  dominant_model: string;
}

/**
 * Group raw assistant-message events into one row per Claude Code
 * conversation. Each unique parent `source_file` (a JSONL path under
 * ~/.claude/projects/) is one session; the lines inside it are the
 * conversation turns. Subagent transcripts (Task/Agent tool) are nested
 * under their parent instead of shown as duplicate top-level rows.
 */
export function aggregateSessions(
  events: SessionEvent[],
  pricing: PricingEntry[] | null,
): AggregatedSession[] {
  const parents = new Map<string, ParentAgg>();

  for (const e of events) {
    const isSub = e.source_file.includes(SUBAGENT_SEGMENT);
    const pkey = parentKeyOf(e.source_file);
    let p = parents.get(pkey);
    if (!p) {
      p = {
        project: e.project,
        latest_ts: e.ts,
        turn_count: 0,
        headline_tokens: 0,
        total_cost_usd: 0,
        dominant_model: e.model,
        breakdown: { ...EMPTY_BREAKDOWN },
        cost_breakdown: { ...EMPTY_BREAKDOWN },
        modelTokens: new Map(),
        subs: new Map(),
      };
      parents.set(pkey, p);
    }

    // Combined accumulation — the parent row represents the whole
    // conversation, so subagent usage counts toward its totals.
    p.turn_count += 1;
    p.headline_tokens += e.input_tokens + e.output_tokens;
    p.breakdown.input += e.input_tokens;
    p.breakdown.output += e.output_tokens;
    p.breakdown.cache_read += e.cache_read_tokens;
    p.breakdown.cache_write_5m += e.cache_creation_5m_tokens;
    p.breakdown.cache_write_1h += e.cache_creation_1h_tokens;
    p.total_cost_usd += e.cost_usd;
    if (e.ts > p.latest_ts) p.latest_ts = e.ts;
    p.modelTokens.set(e.model, (p.modelTokens.get(e.model) ?? 0) + e.output_tokens);

    // Per-category cost is computed per-turn so the Sonnet 1M-context
    // tier (which switches based on the call's own context size, not the
    // session total) is applied correctly.
    if (pricing) {
      const entry = lookupPricing(pricing, e.model);
      if (entry) {
        const c = costPerCategory(entry, {
          input: e.input_tokens,
          output: e.output_tokens,
          cache_read: e.cache_read_tokens,
          cache_5m: e.cache_creation_5m_tokens,
          cache_1h: e.cache_creation_1h_tokens,
        });
        p.cost_breakdown.input += c.input;
        p.cost_breakdown.output += c.output;
        p.cost_breakdown.cache_read += c.cache_read;
        p.cost_breakdown.cache_write_5m += c.cache_5m;
        p.cost_breakdown.cache_write_1h += c.cache_1h;
      }
    }

    if (isSub) {
      let s = p.subs.get(e.source_file);
      if (!s) {
        s = {
          label: subagentLabel(e.source_file),
          latest_ts: e.ts,
          turn_count: 0,
          headline_tokens: 0,
          total_cost_usd: 0,
          modelTokens: new Map(),
          dominant_model: e.model,
        };
        p.subs.set(e.source_file, s);
      }
      s.turn_count += 1;
      s.headline_tokens += e.input_tokens + e.output_tokens;
      s.total_cost_usd += e.cost_usd;
      if (e.ts > s.latest_ts) s.latest_ts = e.ts;
      s.modelTokens.set(e.model, (s.modelTokens.get(e.model) ?? 0) + e.output_tokens);
    }
  }

  const result: AggregatedSession[] = [];
  for (const [id, p] of parents) {
    const subagents: SubagentSession[] = [];
    for (const [sid, s] of p.subs) {
      subagents.push({
        id: sid,
        label: s.label,
        latest_ts: s.latest_ts,
        turn_count: s.turn_count,
        headline_tokens: s.headline_tokens,
        total_cost_usd: s.total_cost_usd,
        dominant_model: pickDominant(s.modelTokens, s.dominant_model),
      });
    }
    // Chronological so the parent's spawn order reads top-to-bottom.
    subagents.sort((a, b) => (a.latest_ts < b.latest_ts ? -1 : 1));

    result.push({
      id,
      project: p.project,
      latest_ts: p.latest_ts,
      turn_count: p.turn_count,
      headline_tokens: p.headline_tokens,
      total_cost_usd: p.total_cost_usd,
      dominant_model: pickDominant(p.modelTokens, p.dominant_model),
      breakdown: p.breakdown,
      cost_breakdown: p.cost_breakdown,
      subagents,
    });
  }

  result.sort((a, b) => (a.latest_ts < b.latest_ts ? 1 : -1));
  return result;
}

const BREAKDOWN_ROWS: Array<{
  key: keyof AggregatedSession['breakdown'];
  costKey: keyof AggregatedSession['cost_breakdown'];
  label: string;
}> = [
  { key: 'input', costKey: 'input', label: 'Input' },
  { key: 'output', costKey: 'output', label: 'Output' },
  { key: 'cache_read', costKey: 'cache_read', label: 'Cache read' },
  { key: 'cache_write_5m', costKey: 'cache_write_5m', label: 'Cache write (5m)' },
  { key: 'cache_write_1h', costKey: 'cache_write_1h', label: 'Cache write (1h)' },
];

function BreakdownTable({
  breakdown,
  costBreakdown,
}: {
  breakdown: AggregatedSession['breakdown'];
  costBreakdown: AggregatedSession['cost_breakdown'];
}) {
  // Hide rows with zero tokens — keeps the table tight, since most
  // sessions don't use the 1h cache tier at all.
  const rows = BREAKDOWN_ROWS.filter((r) => breakdown[r.key] > 0);
  return (
    <div
      className={[
        'mx-[var(--space-sm)] mb-[var(--space-sm)]',
        'rounded-[var(--radius-md)] border border-[var(--color-border-subtle)]',
        'bg-[var(--color-bg-card)]',
      ].join(' ')}
    >
      {rows.map((r, i) => (
        <div
          key={r.key}
          className={[
            'flex items-center justify-between gap-[var(--space-md)] px-[var(--space-md)] py-[var(--space-xs)]',
            i < rows.length - 1 ? 'border-b border-[var(--color-border-subtle)]' : '',
          ].join(' ')}
        >
          <span className="flex-1 text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
            {r.label}
          </span>
          <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)] tabular-nums min-w-[64px] text-right">
            {formatTokens(breakdown[r.key])}
          </span>
          <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-muted)] tabular-nums min-w-[56px] text-right">
            {formatCost(costBreakdown[r.costKey])}
          </span>
        </div>
      ))}
    </div>
  );
}

/** An indented, muted row for one subagent under its parent session. */
function SubagentRow({ sub }: { sub: SubagentSession }) {
  return (
    <div
      className={[
        'flex items-center gap-[var(--space-sm)]',
        'pl-[calc(var(--space-sm)+14px+var(--space-sm))] pr-[var(--space-sm)] py-[var(--space-2xs)]',
      ].join(' ')}
    >
      <IconSubagent size={12} className="shrink-0 text-[color:var(--color-text-muted)] opacity-60" />
      <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] truncate flex-1 min-w-0">
        {sub.label}
      </span>
      <ModelBadge model={sub.dominant_model} />
      <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums min-w-[64px] text-right">
        {formatTokens(sub.headline_tokens)}
      </span>
      <span className="mono text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] tabular-nums min-w-[48px] text-right">
        {formatCost(sub.total_cost_usd)}
      </span>
    </div>
  );
}

export function SessionsTab() {
  const version = useAppStore((s) => s.sessionDataVersion);
  const { data: events, error, loading, reload } = useTabData(
    () => ipc.getSessionHistory(7),
    [version],
  );
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [pricing, setPricing] = useState<PricingEntry[] | null>(null);

  // Pricing rates are static for the lifetime of the process; fetch once.
  useEffect(() => {
    let cancelled = false;
    ipc.getPricing().then((p) => {
      if (!cancelled) setPricing(p);
    }).catch(() => {});
    return () => { cancelled = true; };
  }, []);

  const sessions = useMemo(
    () => aggregateSessions(events ?? [], pricing),
    [events, pricing],
  );
  const totalCost = useMemo(
    () => sessions.reduce((sum, s) => sum + s.total_cost_usd, 0),
    [sessions],
  );

  if (error) {
    return (
      <EmptyState
        icon={<IconSessions size={32} />}
        title="Couldn't load sessions"
        description={error}
        action={<Button variant="ghost" size="sm" onClick={reload}>Retry</Button>}
      />
    );
  }
  if (loading || !events) {
    return <p className="text-[color:var(--color-text-muted)]">Loading…</p>;
  }

  if (sessions.length === 0) {
    return (
      <EmptyState
        icon={<IconSessions size={32} />}
        title="No sessions yet"
        description="Sessions will appear here as you use Claude Code."
      />
    );
  }

  return (
    <div className="flex flex-col gap-[var(--space-sm)]">
      <div className="flex items-center justify-between px-[var(--space-2xs)]">
        <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">
          {sessions.length} {sessions.length === 1 ? 'session' : 'sessions'}
        </span>
        <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)]">
          {formatCost(totalCost)}
        </span>
      </div>

      <div className="flex flex-col">
        {sessions.slice(0, 100).map((session) => {
          const isOpen = expandedId === session.id;
          const Chevron = isOpen ? ChevronDown : ChevronRight;
          const agentCount = session.subagents.length;
          const headless = isHeadlessProject(session.project);
          return (
            <div key={session.id} className="border-b border-[var(--color-border-subtle)]">
              <button
                type="button"
                onClick={() => setExpandedId(isOpen ? null : session.id)}
                className={[
                  'w-full flex items-center gap-[var(--space-sm)]',
                  'px-[var(--space-sm)] py-[var(--space-sm)]',
                  'text-left',
                  'transition-[background] duration-[var(--duration-fast)]',
                  'hover:bg-[var(--color-bg-card)]',
                  isOpen ? 'bg-[var(--color-bg-card)]' : '',
                  headless ? 'opacity-55' : '',
                ].join(' ')}
                aria-expanded={isOpen}
              >
                <Chevron
                  size={14}
                  className="shrink-0 text-[color:var(--color-text-muted)]"
                />
                <div className="flex flex-col min-w-0 flex-1">
                  <div className="flex items-center gap-[var(--space-sm)]">
                    <span
                      className={[
                        'text-[length:var(--text-body)] truncate',
                        headless
                          ? 'italic text-[color:var(--color-text-muted)]'
                          : 'text-[color:var(--color-text)]',
                      ].join(' ')}
                    >
                      {headless ? 'headless' : session.project}
                    </span>
                    <ModelBadge model={session.dominant_model} />
                  </div>
                  <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                    {formatTime(session.latest_ts)} · {session.turn_count} {session.turn_count === 1 ? 'turn' : 'turns'}{agentCount > 0 ? ` · ${agentCount} ${agentCount === 1 ? 'agent' : 'agents'}` : ''}
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
              </button>
              {isOpen && (
                <>
                  {agentCount > 0 && (
                    <div className="flex flex-col py-[var(--space-2xs)]">
                      {session.subagents.map((sub) => (
                        <SubagentRow key={sub.id} sub={sub} />
                      ))}
                    </div>
                  )}
                  <BreakdownTable
                    breakdown={session.breakdown}
                    costBreakdown={session.cost_breakdown}
                  />
                </>
              )}
            </div>
          );
        })}
        {sessions.length > 100 && (
          <div className="py-[var(--space-md)] text-center text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
            Showing latest 100 sessions.
          </div>
        )}
      </div>
    </div>
  );
}
