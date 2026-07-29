/**
 * Expanding the two model fields into the env Claude Code actually reads.
 *
 * One model name is never enough. Claude Code resolves the model in use and
 * the `/model <alias>` shortcuts from separate variables, verified against the
 * v2.1.220 binary:
 *
 *   - the quick ("small, fast") model comes from `ANTHROPIC_SMALL_FAST_MODEL`
 *     and falls back to the haiku alias only when that is unset;
 *   - `/model haiku|sonnet|opus|fable` each resolve their own
 *     `ANTHROPIC_DEFAULT_*_MODEL`, and fall back to a *first-party* Anthropic
 *     id when unset.
 *
 * That fallback is why every alias has to be written, not just the ones a user
 * expects to type. Left unset against a third-party endpoint, `/model opus`
 * asks z.ai for `claude-opus-…` and the request fails. It is also why editing
 * the Model field has to rewrite the aliases: before this, a provider seeded
 * from a preset kept the preset's alias values forever, so changing the model
 * moved `ANTHROPIC_MODEL` and left `/model opus` pointing at the old one.
 */

/** Aliases that should name the provider's main model. */
export const BIG_MODEL_KEYS = [
  'ANTHROPIC_MODEL',
  'ANTHROPIC_DEFAULT_OPUS_MODEL',
  'ANTHROPIC_DEFAULT_SONNET_MODEL',
  'ANTHROPIC_DEFAULT_FABLE_MODEL',
] as const;

/** Aliases that should name the provider's quick model. */
export const QUICK_MODEL_KEYS = [
  'ANTHROPIC_SMALL_FAST_MODEL',
  'ANTHROPIC_DEFAULT_HAIKU_MODEL',
] as const;

export const MODEL_ENV_KEYS: readonly string[] = [...BIG_MODEL_KEYS, ...QUICK_MODEL_KEYS];

/**
 * `env` with the model aliases rewritten from the two form fields.
 *
 * An empty field clears its aliases rather than leaving stale ones behind —
 * a blank Quick model means "don't pin one", and pinning the previous
 * provider's value would be worse than not answering.
 *
 * Every other entry in `env` is passed through untouched: presets carry
 * context-window and timeout knobs that this function has no business
 * rewriting.
 */
export function applyModelEnv(
  env: Record<string, string>,
  model: string,
  quickModel: string,
): Record<string, string> {
  const out = { ...env };
  for (const [keys, value] of [
    [BIG_MODEL_KEYS, model.trim()],
    [QUICK_MODEL_KEYS, quickModel.trim()],
  ] as const) {
    for (const key of keys) {
      if (value) out[key] = value;
      else delete out[key];
    }
  }
  return out;
}
