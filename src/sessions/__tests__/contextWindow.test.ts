import { describe, it, expect } from 'vitest';
import { windowFor, contextReadout } from '../contextWindow';

describe('windowFor', () => {
  // Verified against the model registry in Claude Code v2.1.220: these are
  // exactly the ids carrying `context.native_1m: true`.
  it('reports 1M for models whose registry entry sets native_1m', () => {
    for (const m of [
      'claude-sonnet-5',
      'claude-opus-4-7',
      'claude-opus-4-8',
      'claude-opus-5',
      'claude-fable-5',
    ]) {
      expect(windowFor(m), m).toEqual({ total: 1_000_000, label: '1M' });
    }
  });

  // The regression this file exists for. The split runs *within* families:
  // Sonnet 5 is 1M but Sonnet 4.6 is 200K; Opus 4.7 is 1M but Opus 4.6 is
  // 200K. A prefix match on "sonnet"/"opus" mislabels every older release.
  it('reports 200K for the older releases of the same families', () => {
    for (const m of [
      'claude-sonnet-4-6',
      'claude-sonnet-4-5',
      'claude-opus-4-6',
      'claude-opus-4-5',
      'claude-haiku-4-5',
      'claude-haiku-4-5-20251001',
    ]) {
      expect(windowFor(m), m).toEqual({ total: 200_000, label: '200K' });
    }
  });

  // Claude Code strips the suffix before writing a transcript, but a model id
  // taken from a provider config still carries it.
  it('honours an explicit [1m] suffix', () => {
    expect(windowFor('claude-opus-5[1m]').total).toBe(1_000_000);
    expect(windowFor('claude-sonnet-4-6[1m]').total).toBe(1_000_000);
  });

  // Third-party windows are set per-provider. Guessing a denominator would
  // put a confidently wrong percentage beside a correct token count.
  it('returns no window for third-party and unrecognised models', () => {
    for (const m of ['glm-5.2', 'k3', 'kimi-for-coding-highspeed', 'claude-opus-9', null]) {
      expect(windowFor(m), String(m)).toEqual({ total: null, label: null });
    }
  });
});

describe('contextReadout', () => {
  it('shows peak against the window when one is known', () => {
    const r = contextReadout(495_644, 'claude-opus-5');
    expect(r.text).toBe('495.6K of 1M');
    expect(r.pct).toBe(50);
  });

  // The exact case the old peak-based inference got wrong: 15 local Fable 5
  // sessions never crossed 200K, so every one was labelled "of 200K" when the
  // real window was 1M.
  it('reports 1M for a native-1M model that never approached the window', () => {
    const r = contextReadout(155_837, 'claude-fable-5');
    expect(r.text).toBe('155.8K of 1M');
    expect(r.pct).toBe(16);
  });

  it('omits the denominator and the bar for a third-party model', () => {
    const r = contextReadout(466_521, 'glm-5.2');
    expect(r.text).toBe('466.5K');
    expect(r.pct).toBeNull();
  });

  it('never exceeds 100% when a session overruns its window', () => {
    expect(contextReadout(260_000, 'claude-sonnet-4-6').pct).toBe(100);
  });

  it('formats whole thousands without a trailing zero', () => {
    expect(contextReadout(57_000, 'claude-sonnet-4-6').text).toBe('57K of 200K');
    expect(contextReadout(1_000_000, 'claude-opus-5').text).toBe('1M of 1M');
  });
});
