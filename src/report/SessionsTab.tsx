import { useEffect, useMemo, useState } from 'react';
import { AccountBadge } from '../components/ui/AccountBadge';
import { Button } from '../components/ui/Button';
import { ModelBadge } from '../components/ui/ModelBadge';
import { EmptyState } from '../components/ui/EmptyState';
import { formatTokens, formatCost } from '../lib/format';
import { ChevronDown, ChevronRight, IconCompact, IconSessions, IconSubagent } from '../lib/icons';
import { ipc } from '../lib/ipc';
import { useTabData } from '../lib/useTabData';
import { useAppStore } from '../lib/store';
import type { Compaction, PricingEntry, SessionEvent } from '../lib/types';
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

function formatClock(iso: string): string {
  return new Date(iso).toLocaleTimeString('en-US', { hour: 'numeric', minute: '2-digit' });
}

/** Local calendar day of `iso` as `YYYY-MM-DD`.
 *
 * Built from the Date's local getters rather than `toISOString()` (which is
 * UTC) or a locale format (which varies by machine), because this string is
 * the grouping key for a row's cost. It has to agree with the backend's
 * `get_daily_trends`, which buckets on `chrono::Local` — otherwise the Cost
 * tab and the Trends bar for the same day would disagree near midnight. */
function localDayKey(iso: string): string {
  const d = new Date(iso);
  const month = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${d.getFullYear()}-${month}-${day}`;
}

function formatDayLabel(dayKey: string): string {
  const today = localDayKey(new Date().toISOString());
  if (dayKey === today) return 'Today';
  const yesterday = new Date();
  yesterday.setDate(yesterday.getDate() - 1);
  if (dayKey === localDayKey(yesterday.toISOString())) return 'Yesterday';
  // Parse as local midnight — `new Date('2026-07-26')` would be parsed as UTC
  // and could render the previous day west of Greenwich.
  const [y, m, d] = dayKey.split('-').map(Number);
  return new Date(y, m - 1, d).toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
  });
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
  /** Local calendar day (`YYYY-MM-DD`) this row's turns happened on. A row
   * covers exactly one day, so `total_cost_usd` is money actually spent on
   * that date — see `aggregateSessions`. */
  day: string;
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
  /** Every distinct account this row's turns were attributed to (including
   * `null` for turns that predate account tracking or landed in a gap). */
  account_uuids: (string | null)[];
}

/** Splitting rows by day multiplies their count, so the cap is higher than
 *  the old per-conversation 100 — it still bounds the DOM on a heavy month. */
const MAX_ROWS = 250;

/** Days of history this tab aggregates. Mirrored by `TAB_WINDOW_DAYS` in
 *  ExpandedReport so the header label cannot drift from what is queried. */
export const WINDOW_DAYS = 7;

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
  day: string;
  latest_ts: string;
  turn_count: number;
  headline_tokens: number;
  total_cost_usd: number;
  dominant_model: string;
  breakdown: Breakdown;
  cost_breakdown: Breakdown;
  modelTokens: Map<string, number>;
  accountUuids: Set<string | null>;
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
 * Group raw assistant-message events into one row per conversation **per day**.
 * Each unique parent `source_file` (a JSONL path under ~/.claude/projects/) is
 * one conversation; the lines inside it are its turns. Subagent transcripts
 * (Task/Agent tool) are nested under their parent instead of shown as
 * duplicate top-level rows.
 *
 * The day is part of the key, not just a label. Grouping on `source_file`
 * alone produced one row per conversation stamped with its *most recent* turn
 * while summing cost across *every* turn — so resuming a conversation after a
 * few days moved its whole accumulated cost under today. Real case from a user
 * database: a conversation spanning Jul 26–29 rendered as "today · $12.33"
 * when $1.13 was spent that day and $11.20 belonged to the three days before.
 *
 * With the day in the key, each row's total is money spent on that date, so
 * the rows under a day header sum to that day's bar in the Trends tab.
 */
export function aggregateSessions(
  events: SessionEvent[],
  pricing: PricingEntry[] | null,
): AggregatedSession[] {
  const parents = new Map<string, ParentAgg>();

  for (const e of events) {
    const isSub = e.source_file.includes(SUBAGENT_SEGMENT);
    const day = localDayKey(e.ts);
    // `#` cannot occur in a day key and does not appear in Claude Code's
    // session paths, so it is an unambiguous separator for the composite key.
    const pkey = `${parentKeyOf(e.source_file)}#${day}`;
    let p = parents.get(pkey);
    if (!p) {
      p = {
        project: e.project,
        day,
        latest_ts: e.ts,
        turn_count: 0,
        headline_tokens: 0,
        total_cost_usd: 0,
        dominant_model: e.model,
        breakdown: { ...EMPTY_BREAKDOWN },
        cost_breakdown: { ...EMPTY_BREAKDOWN },
        modelTokens: new Map(),
        accountUuids: new Set(),
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
    p.accountUuids.add(e.account_uuid);

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
      day: p.day,
      latest_ts: p.latest_ts,
      turn_count: p.turn_count,
      headline_tokens: p.headline_tokens,
      total_cost_usd: p.total_cost_usd,
      dominant_model: pickDominant(p.modelTokens, p.dominant_model),
      breakdown: p.breakdown,
      cost_breakdown: p.cost_breakdown,
      subagents,
      account_uuids: Array.from(p.accountUuids),
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

/** Compaction boundaries that fall inside one session-day row.
 *
 * Worth surfacing because a compaction is invisible in the turn count but
 * explains two things a bare row cannot: why a single day's session appears
 * to hold more history than its turns suggest, and why cost steps up right
 * afterwards — the surviving context is re-written to a cold cache at full
 * price on the next call. */
function CompactionRows({ items }: { items: Compaction[] }) {
  return (
    <div className="flex flex-col">
      {items.map((c) => {
        const dropped = c.pre_tokens > c.post_tokens ? c.pre_tokens - c.post_tokens : 0;
        return (
          <div
            key={c.uuid}
            className="
              flex items-center gap-[var(--space-sm)]
              pl-[calc(var(--space-sm)+14px+var(--space-sm))] pr-[var(--space-sm)]
              py-[var(--space-2xs)]
            "
          >
            <IconCompact
              size={12}
              aria-hidden
              className="shrink-0 text-[color:var(--color-warn)] opacity-80"
            />
            <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]">
              Context compacted{c.trigger === 'manual' ? '' : ' automatically'} at{' '}
              {formatClock(c.ts)}
            </span>
            <span className="mono text-[length:var(--text-micro)] tabular-nums text-[color:var(--color-text-muted)]">
              {formatTokens(c.pre_tokens)} → {formatTokens(c.post_tokens)}
            </span>
            {dropped > 0 && (
              <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                ({formatTokens(dropped)} dropped)
              </span>
            )}
          </div>
        );
      })}
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
  const accounts = useAppStore((s) => s.accounts);
  const { data, error, loading, reload } = useTabData(
    () =>
      Promise.all([ipc.getSessionHistory(WINDOW_DAYS), ipc.getCompactions(WINDOW_DAYS)]).then(
        ([events, compactions]) => ({ events, compactions }),
      ),
    [version],
  );
  const events = data?.events ?? null;
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [pricing, setPricing] = useState<PricingEntry[] | null>(null);
  const [collapsedDays, setCollapsedDays] = useState<Set<string>>(new Set());

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

  /** Compactions keyed exactly like a session row (`<parentFile>#<day>`), so
   * an expanded row can look up its own boundaries in O(1). */
  const compactionsByRow = useMemo(() => {
    const map = new Map<string, Compaction[]>();
    for (const c of data?.compactions ?? []) {
      const key = `${parentKeyOf(c.source_file)}#${localDayKey(c.ts)}`;
      const list = map.get(key);
      if (list) list.push(c);
      else map.set(key, [c]);
    }
    for (const list of map.values()) {
      list.sort((a, b) => (a.ts < b.ts ? -1 : 1));
    }
    return map;
  }, [data?.compactions]);

  /** Distinct conversations, not rows: a conversation worked on across three
   * days is one session that produced three rows. Counting rows here would
   * inflate the headline the same way the old cost attribution did. */
  const conversationCount = useMemo(
    () => new Set(sessions.map((s) => s.id.slice(0, s.id.lastIndexOf('#')))).size,
    [sessions],
  );

  /** Rows bucketed under their day, newest first. `sessions` is already
   * sorted by `latest_ts` descending and every row covers a single day, so a
   * linear pass produces day groups in order without a second sort. */
  const days = useMemo(() => {
    const out: Array<{ day: string; rows: AggregatedSession[]; cost: number }> = [];
    for (const s of sessions.slice(0, MAX_ROWS)) {
      let group = out[out.length - 1];
      if (!group || group.day !== s.day) {
        group = { day: s.day, rows: [], cost: 0 };
        out.push(group);
      }
      group.rows.push(s);
      group.cost += s.total_cost_usd;
    }
    return out;
  }, [sessions]);

  // Every day group starts collapsed except the most recent — keeps the tab
  // scannable on a heavy week without hiding today's activity by default.
  useEffect(() => {
    if (days.length === 0) return;
    setCollapsedDays(new Set(days.slice(1).map((g) => g.day)));
  }, [events]);

  const toggleDay = (day: string) => {
    setCollapsedDays((prev) => {
      const next = new Set(prev);
      if (next.has(day)) next.delete(day);
      else next.add(day);
      return next;
    });
  };

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
          {conversationCount} {conversationCount === 1 ? 'session' : 'sessions'}
        </span>
        <span className="mono text-[length:var(--text-label)] text-[color:var(--color-text-secondary)]">
          {formatCost(totalCost)}
        </span>
      </div>

      <div className="flex flex-col">
        {days.map((group) => {
          const dayOpen = !collapsedDays.has(group.day);
          const DayChevron = dayOpen ? ChevronDown : ChevronRight;
          return (
          <div key={group.day} className="flex flex-col">
            {/* The day total is the point of the header, not decoration: it is
                what the Trends bar for this date shows, so the two tabs can be
                reconciled by eye. */}
            <button
              type="button"
              onClick={() => toggleDay(group.day)}
              aria-expanded={dayOpen}
              className="
                sticky top-0 z-[1] flex items-center justify-between gap-[var(--space-sm)]
                bg-[var(--color-bg-base)] px-[var(--space-sm)]
                pt-[var(--space-md)] pb-[var(--space-2xs)]
                text-left
              "
            >
              <span className="flex items-center gap-[var(--space-2xs)] min-w-0">
                <DayChevron size={12} className="shrink-0 text-[color:var(--color-text-muted)]" />
                <span
                  className="
                    text-[length:var(--text-micro)] font-[var(--weight-medium)]
                    uppercase tracking-[var(--tracking-label)]
                    text-[color:var(--color-text-muted)]
                  "
                >
                  {formatDayLabel(group.day)}
                </span>
              </span>
              <span className="mono text-[length:var(--text-micro)] tabular-nums text-[color:var(--color-text-muted)]">
                {formatCost(group.cost)}
              </span>
            </button>
            {dayOpen && group.rows.map((session) => {
          const isOpen = expandedId === session.id;
          const Chevron = isOpen ? ChevronDown : ChevronRight;
          const agentCount = session.subagents.length;
          const headless = isHeadlessProject(session.project);
          const compactions = compactionsByRow.get(session.id) ?? [];
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
                    {session.account_uuids.map((uuid) => (
                      <AccountBadge key={uuid ?? 'unknown'} accountUuid={uuid} accounts={accounts} />
                    ))}
                    {/* Collapsed-row marker: without it the boundary is only
                        discoverable by expanding every row one at a time. */}
                    {compactions.length > 0 && (
                      <span
                        title={`Context compacted ${compactions.length}×`}
                        className="inline-flex shrink-0 items-center text-[color:var(--color-warn)] opacity-80"
                      >
                        <IconCompact size={11} aria-label="Context compacted" />
                      </span>
                    )}
                  </div>
                  <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                    {formatClock(session.latest_ts)} · {session.turn_count} {session.turn_count === 1 ? 'turn' : 'turns'}{agentCount > 0 ? ` · ${agentCount} ${agentCount === 1 ? 'agent' : 'agents'}` : ''}
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
                  {compactions.length > 0 && <CompactionRows items={compactions} />}
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
          </div>
          );
        })}
        {sessions.length > MAX_ROWS && (
          <div className="py-[var(--space-md)] text-center text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
            Showing the latest {MAX_ROWS} rows.
          </div>
        )}
      </div>
    </div>
  );
}
