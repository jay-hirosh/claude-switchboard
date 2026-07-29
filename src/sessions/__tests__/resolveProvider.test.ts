import { describe, it, expect } from 'vitest';
import type { Provider } from '../../lib/generated/bindings';
import { norm, resolveProvider } from '../resolveProvider';

function provider(id: string, env: Record<string, string>, sortIndex = 1): Provider {
  return {
    id,
    name: id,
    kind: 'third_party',
    base_url: 'https://example.test',
    auth_token: 't',
    env,
    extra_args: [],
    preset_id: null,
    sort_index: sortIndex,
  };
}

const official: Provider = {
  id: 'official',
  name: 'Anthropic (official)',
  kind: 'official',
  base_url: null,
  auth_token: null,
  env: {},
  extra_args: [],
  preset_id: null,
  sort_index: 0,
};

describe('resolveProvider', () => {
  it('resolves a [1m]-suffixed provider config against a stripped session model', () => {
    // The C2 regression: Claude Code strips [1m] before writing the
    // transcript, so the SESSION side is already normalized and the PROVIDER
    // side still carries the suffix.
    const glm = provider('glm', { ANTHROPIC_MODEL: 'glm-5.2[1m]' });
    expect(resolveProvider('glm-5.2', [official, glm])).toEqual({
      kind: 'resolved',
      providerId: 'glm',
    });
  });

  it('resolves k3 the same way', () => {
    const kimi = provider('kimi', { ANTHROPIC_MODEL: 'k3[1m]' });
    expect(resolveProvider('k3', [official, kimi])).toEqual({
      kind: 'resolved',
      providerId: 'kimi',
    });
  });

  it('matches case-insensitively', () => {
    const mm = provider('minimax', { ANTHROPIC_MODEL: 'MiniMax-M2.7-highspeed' });
    expect(resolveProvider('minimax-m2.7-highspeed', [mm])).toEqual({
      kind: 'resolved',
      providerId: 'minimax',
    });
  });

  it('matches on non-ANTHROPIC_MODEL keys', () => {
    // 519 recorded events are kimi-for-coding-highspeed, configured as the
    // small/fast model rather than the primary one.
    const kimi = provider('kimi', {
      ANTHROPIC_MODEL: 'k3[1m]',
      ANTHROPIC_SMALL_FAST_MODEL: 'kimi-for-coding-highspeed',
    });
    expect(resolveProvider('kimi-for-coding-highspeed', [kimi])).toEqual({
      kind: 'resolved',
      providerId: 'kimi',
    });
  });

  it('breaks ties by sort_index then id', () => {
    const a = provider('bbb', { ANTHROPIC_MODEL: 'dup' }, 5);
    const b = provider('aaa', { ANTHROPIC_MODEL: 'dup' }, 2);
    expect(resolveProvider('dup', [a, b])).toEqual({ kind: 'resolved', providerId: 'aaa' });
  });

  it('falls back to official for claude-* ids', () => {
    expect(resolveProvider('claude-opus-5', [official])).toEqual({
      kind: 'resolved',
      providerId: 'official',
    });
  });

  it('does not silently reroute a deleted provider to official', () => {
    // A relay that echoed an Anthropic-style id. With its provider removed,
    // this must prompt rather than resume on Anthropic.
    expect(resolveProvider('claude-sonnet-4-5-thinking', [official])).toEqual({
      kind: 'unresolved',
    });
  });

  it('is unresolved for an unknown model and for no model at all', () => {
    expect(resolveProvider('mystery-9', [official])).toEqual({ kind: 'unresolved' });
    expect(resolveProvider(null, [official])).toEqual({ kind: 'unresolved' });
  });
});

describe('norm', () => {
  it('lowercases and strips a trailing [1m]', () => {
    expect(norm('GLM-5.2[1M]')).toBe('glm-5.2');
    expect(norm('k3')).toBe('k3');
  });
});
