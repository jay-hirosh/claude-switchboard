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
