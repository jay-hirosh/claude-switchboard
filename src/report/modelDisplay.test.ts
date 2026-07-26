import { describe, it, expect } from 'vitest';
import { modelKey, shortName, MODEL_VARIANT } from './modelDisplay';

describe('modelKey', () => {
  it('classifies opus/sonnet/haiku regardless of case or version suffix', () => {
    expect(modelKey('claude-opus-4-7')).toBe('opus');
    expect(modelKey('Claude-Sonnet-4-6')).toBe('sonnet');
    expect(modelKey('claude-haiku-4-5')).toBe('haiku');
  });

  it('falls back to default for unrecognized models', () => {
    expect(modelKey('glm-5.1')).toBe('default');
  });
});

describe('shortName', () => {
  it('extracts a compact "tier version" label', () => {
    expect(shortName('claude-opus-4-7')).toBe('opus 4-7');
    expect(shortName('claude-sonnet-4-6')).toBe('sonnet 4-6');
  });

  it('returns the raw name when no tier pattern matches', () => {
    expect(shortName('glm-5.1')).toBe('glm-5.1');
  });
});

describe('MODEL_VARIANT', () => {
  it('maps known tiers to their badge variant', () => {
    expect(MODEL_VARIANT.opus).toBe('opus');
    expect(MODEL_VARIANT.sonnet).toBe('sonnet');
    expect(MODEL_VARIANT.haiku).toBe('haiku');
  });
});
