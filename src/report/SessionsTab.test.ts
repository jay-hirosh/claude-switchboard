import { describe, it, expect } from 'vitest';
import { aggregateSessions, isHeadlessProject, modelLabel } from './SessionsTab';
import type { SessionEvent } from '../lib/types';

const PROJ = '-Users-feixu-Developer-Ai-life-os';

let idCounter = 0;
/** Build a minimal SessionEvent with sane defaults; only the fields that
 * matter for grouping need to be overridden. */
function mk(over: Partial<SessionEvent> & { source_file: string; project: string }): SessionEvent {
  idCounter += 1;
  return {
    ts: '2026-07-26T03:00:00Z',
    model: 'claude-sonnet-5',
    input_tokens: 100,
    output_tokens: 50,
    cache_read_tokens: 0,
    cache_creation_5m_tokens: 0,
    cache_creation_1h_tokens: 0,
    cost_usd: 0.1,
    source_line: 0,
    event_id: `id-${idCounter}`,
    account_uuid: null,
    ...over,
  };
}

describe('aggregateSessions — subagent nesting', () => {
  it('rolls subagent transcripts under their parent conversation', () => {
    const events = [
      // parent conversation (1 turn)
      mk({ source_file: `${PROJ}/8723ad3b.jsonl`, project: 'life-os', input_tokens: 10, output_tokens: 0, cost_usd: 0.4 }),
      // two subagents spawned by it
      mk({ source_file: `${PROJ}/8723ad3b/subagents/agent-a47b7bea.jsonl`, project: 'life-os', ts: '2026-07-26T03:01:00Z', input_tokens: 1000, output_tokens: 500, cost_usd: 1.5 }),
      mk({ source_file: `${PROJ}/8723ad3b/subagents/agent-aa8a0578.jsonl`, project: 'life-os', ts: '2026-07-26T03:02:00Z', input_tokens: 500, output_tokens: 200, cost_usd: 1.0 }),
    ];

    const sessions = aggregateSessions(events, null);

    // ONE row for the whole conversation, not three.
    expect(sessions).toHaveLength(1);
    const s = sessions[0];
    expect(s.project).toBe('life-os');
    expect(s.subagents).toHaveLength(2);
    // combined totals: parent + both subagents
    expect(s.turn_count).toBe(3);
    expect(s.total_cost_usd).toBeCloseTo(2.9, 5);
    expect(s.headline_tokens).toBe(10 + 1500 + 700);
    // subagent labels derived from the agent hash
    expect(s.subagents.map((x) => x.label)).toEqual(['agent a47b7bea', 'agent aa8a0578']);
  });

  it('keeps unrelated parent sessions as separate rows', () => {
    const sessions = aggregateSessions(
      [
        mk({ source_file: `${PROJ}/aaa.jsonl`, project: 'a' }),
        mk({ source_file: `${PROJ}/bbb.jsonl`, project: 'b' }),
      ],
      null,
    );
    expect(sessions).toHaveLength(2);
    expect(sessions.every((s) => s.subagents.length === 0)).toBe(true);
  });

  it('groups a subagent whose parent transcript has no usage events', () => {
    // The parent .jsonl exists on disk but produced no assistant-usage lines,
    // so only the subagent was ingested. It should still roll up under the
    // parent key (one row, one nested subagent) rather than vanish or error.
    const sessions = aggregateSessions(
      [mk({ source_file: `${PROJ}/ccc/subagents/agent-deadbeef.jsonl`, project: 'ccc-project' })],
      null,
    );
    expect(sessions).toHaveLength(1);
    expect(sessions[0].subagents).toHaveLength(1);
    expect(sessions[0].subagents[0].label).toBe('agent deadbeef');
  });

  it('preserves total cost across a mixed set (no drop, no double-count)', () => {
    const events = [
      mk({ source_file: `${PROJ}/p1.jsonl`, project: 'p1', cost_usd: 0.3 }),
      mk({ source_file: `${PROJ}/p1/subagents/agent-aaa.jsonl`, project: 'p1', cost_usd: 0.7 }),
      mk({ source_file: `${PROJ}/p2.jsonl`, project: 'p2', cost_usd: 1.1 }),
      mk({ source_file: `-/headless.jsonl`, project: '-', cost_usd: 0.12 }),
    ];
    const sessions = aggregateSessions(events, null);
    const eventSum = events.reduce((t, e) => t + e.cost_usd, 0);
    const sessionSum = sessions.reduce((t, s) => t + s.total_cost_usd, 0);
    expect(sessionSum).toBeCloseTo(eventSum, 5);
  });
});

describe('aggregateSessions — per-day attribution', () => {
  // The regression this suite exists for: a conversation that spans days used
  // to collapse into ONE row stamped with its most recent turn while summing
  // cost across every turn. Resuming an old conversation therefore parked its
  // entire historical cost under today. Measured on real data before the fix:
  // session 57ca2089 rendered as "today, $12.33" when only $1.13 was spent
  // that day — the other $11.20 belonged to the three preceding days.
  it('splits a conversation that spans days into one row per day', () => {
    const events = [
      mk({ source_file: `${PROJ}/spanning.jsonl`, project: 'p', ts: '2026-07-26T03:00:00Z', cost_usd: 11.2 }),
      mk({ source_file: `${PROJ}/spanning.jsonl`, project: 'p', ts: '2026-07-29T03:00:00Z', cost_usd: 1.13 }),
    ];
    const sessions = aggregateSessions(events, null);

    expect(sessions).toHaveLength(2);
    // Newest day first, carrying only that day's money.
    expect(sessions[0].total_cost_usd).toBeCloseTo(1.13, 5);
    expect(sessions[1].total_cost_usd).toBeCloseTo(11.2, 5);
    // Rows must not collide in React's key space.
    expect(sessions[0].id).not.toBe(sessions[1].id);
  });

  it('keeps same-day turns of one conversation in a single row', () => {
    const sessions = aggregateSessions(
      [
        mk({ source_file: `${PROJ}/oneday.jsonl`, project: 'p', ts: '2026-07-29T01:00:00Z', cost_usd: 1 }),
        mk({ source_file: `${PROJ}/oneday.jsonl`, project: 'p', ts: '2026-07-29T05:00:00Z', cost_usd: 2 }),
      ],
      null,
    );
    expect(sessions).toHaveLength(1);
    expect(sessions[0].turn_count).toBe(2);
    expect(sessions[0].total_cost_usd).toBeCloseTo(3, 5);
  });

  it('attributes a subagent to the day it ran, under that day’s parent row', () => {
    const sessions = aggregateSessions(
      [
        mk({ source_file: `${PROJ}/s.jsonl`, project: 'p', ts: '2026-07-26T03:00:00Z', cost_usd: 0.5 }),
        mk({ source_file: `${PROJ}/s/subagents/agent-aaa.jsonl`, project: 'p', ts: '2026-07-29T03:00:00Z', cost_usd: 2 }),
      ],
      null,
    );
    expect(sessions).toHaveLength(2);
    const today = sessions[0];
    expect(today.total_cost_usd).toBeCloseTo(2, 5);
    expect(today.subagents).toHaveLength(1);
    // The Jul 26 row must not inherit the Jul 29 subagent.
    expect(sessions[1].subagents).toHaveLength(0);
  });

  it('still preserves the grand total once rows are split by day', () => {
    const events = [
      mk({ source_file: `${PROJ}/a.jsonl`, project: 'p', ts: '2026-07-26T03:00:00Z', cost_usd: 0.3 }),
      mk({ source_file: `${PROJ}/a.jsonl`, project: 'p', ts: '2026-07-27T03:00:00Z', cost_usd: 0.7 }),
      mk({ source_file: `${PROJ}/a/subagents/agent-x.jsonl`, project: 'p', ts: '2026-07-27T04:00:00Z', cost_usd: 1.1 }),
      mk({ source_file: `-/headless.jsonl`, project: '-', ts: '2026-07-29T03:00:00Z', cost_usd: 0.12 }),
    ];
    const sessions = aggregateSessions(events, null);
    expect(sessions.reduce((t, s) => t + s.total_cost_usd, 0)).toBeCloseTo(
      events.reduce((t, e) => t + e.cost_usd, 0),
      5,
    );
  });
});

describe('compaction row keys', () => {
  // The Cost tab looks compactions up by the row id `<parentFile>#<day>`.
  // A compaction recorded against a subagent transcript must therefore fold
  // onto the parent's key, exactly like its events do — otherwise the marker
  // silently never renders.
  it('a compaction keys onto the same row id as the turns it sits between', () => {
    const events = [
      mk({ source_file: `${PROJ}/s.jsonl`, project: 'p', ts: '2026-07-29T01:00:00Z' }),
      mk({ source_file: `${PROJ}/s/subagents/agent-aaa.jsonl`, project: 'p', ts: '2026-07-29T02:00:00Z' }),
    ];
    const [row] = aggregateSessions(events, null);
    // Mirrors the component's `${parentKeyOf(c.source_file)}#${localDayKey(c.ts)}`.
    const parentKey = `${PROJ}/s.jsonl`;
    const day = localDayKeyOf('2026-07-29T03:17:00Z');
    expect(row.id).toBe(`${parentKey}#${day}`);
    expect(row.day).toBe(day);
  });
});

/** Local-day key, duplicated from SessionsTab so the test asserts the shape
 *  independently of the module-private helper. */
function localDayKeyOf(iso: string): string {
  const d = new Date(iso);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}

describe('modelLabel', () => {
  it('collapses Anthropic families to their tier name', () => {
    expect(modelLabel('claude-opus-4-7')).toBe('opus');
    expect(modelLabel('claude-sonnet-5')).toBe('sonnet');
    expect(modelLabel('claude-haiku-4-5-20251001')).toBe('haiku');
    expect(modelLabel('claude-fable-5')).toBe('fable');
  });
  it('surfaces the real name for third-party / relay models', () => {
    expect(modelLabel('glm-5.2')).toBe('glm-5.2');
    expect(modelLabel('k3')).toBe('k3');
    expect(modelLabel('MiniMax-M2.7-highspeed')).toBe('MiniMax-M2.7');
    expect(modelLabel('kimi-for-coding-highspeed')).toBe('kimi');
  });
});

describe('isHeadlessProject', () => {
  it('flags the no-cwd bucket ("-") and empty', () => {
    expect(isHeadlessProject('-')).toBe(true);
    expect(isHeadlessProject('')).toBe(true);
  });
  it('leaves real project names alone', () => {
    expect(isHeadlessProject('life-os')).toBe(false);
    expect(isHeadlessProject('claude-switchboard')).toBe(false);
  });
});
