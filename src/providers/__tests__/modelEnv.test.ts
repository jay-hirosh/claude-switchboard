import { describe, it, expect } from 'vitest';
import { applyModelEnv } from '../modelEnv';

describe('applyModelEnv', () => {
  it('points every big-model alias at the main model', () => {
    const env = applyModelEnv({}, 'glm-5.2', '');
    expect(env).toEqual({
      ANTHROPIC_MODEL: 'glm-5.2',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'glm-5.2',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'glm-5.2',
      ANTHROPIC_DEFAULT_FABLE_MODEL: 'glm-5.2',
    });
  });

  it('points the quick aliases at the quick model', () => {
    const env = applyModelEnv({}, 'k3', 'kimi-for-coding-highspeed');
    expect(env['ANTHROPIC_SMALL_FAST_MODEL']).toBe('kimi-for-coding-highspeed');
    expect(env['ANTHROPIC_DEFAULT_HAIKU_MODEL']).toBe('kimi-for-coding-highspeed');
    // The quick model must not leak into the big-model aliases.
    expect(env['ANTHROPIC_DEFAULT_OPUS_MODEL']).toBe('k3');
  });

  // The regression this exists for: a provider seeded from a preset kept the
  // preset's alias values forever, so changing Model moved ANTHROPIC_MODEL and
  // left `/model opus` pointing at the model the user had just replaced.
  it('rewrites stale aliases left over from a preset', () => {
    const seeded = {
      ANTHROPIC_MODEL: 'glm-5.2',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'glm-5.2',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'glm-5.2',
      ANTHROPIC_DEFAULT_FABLE_MODEL: 'glm-5.2',
      ANTHROPIC_SMALL_FAST_MODEL: 'glm-5-turbo',
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'glm-5-turbo',
    };
    const env = applyModelEnv(seeded, 'glm-6', 'glm-6-turbo');
    expect(env['ANTHROPIC_DEFAULT_OPUS_MODEL']).toBe('glm-6');
    expect(env['ANTHROPIC_DEFAULT_SONNET_MODEL']).toBe('glm-6');
    expect(env['ANTHROPIC_DEFAULT_FABLE_MODEL']).toBe('glm-6');
    expect(env['ANTHROPIC_DEFAULT_HAIKU_MODEL']).toBe('glm-6-turbo');
  });

  // A blank field means "don't pin one". Keeping the previous provider's value
  // would be worse than leaving the question unanswered.
  it('clears the aliases a blank field owns', () => {
    const env = applyModelEnv(
      { ANTHROPIC_SMALL_FAST_MODEL: 'old-turbo', ANTHROPIC_DEFAULT_HAIKU_MODEL: 'old-turbo' },
      'glm-5.2',
      '',
    );
    expect(env).not.toHaveProperty('ANTHROPIC_SMALL_FAST_MODEL');
    expect(env).not.toHaveProperty('ANTHROPIC_DEFAULT_HAIKU_MODEL');
  });

  it('passes every unrelated entry through untouched', () => {
    const env = applyModelEnv(
      { CLAUDE_CODE_MAX_CONTEXT_TOKENS: '1000000', API_TIMEOUT_MS: '3000000' },
      'glm-5.2',
      'glm-5-turbo',
    );
    expect(env['CLAUDE_CODE_MAX_CONTEXT_TOKENS']).toBe('1000000');
    expect(env['API_TIMEOUT_MS']).toBe('3000000');
  });

  it('trims whitespace and does not treat a spaces-only field as a value', () => {
    const env = applyModelEnv({}, '  glm-5.2  ', '   ');
    expect(env['ANTHROPIC_MODEL']).toBe('glm-5.2');
    expect(env).not.toHaveProperty('ANTHROPIC_SMALL_FAST_MODEL');
  });

  it('does not mutate the env it was given', () => {
    const original = { ANTHROPIC_MODEL: 'glm-5.2' };
    applyModelEnv(original, 'glm-6', 'glm-6-turbo');
    expect(original).toEqual({ ANTHROPIC_MODEL: 'glm-5.2' });
  });
});
