/**
 * Context-window resolution, mirroring Claude Code's own logic.
 *
 * Claude Code resolves a window through three independent paths (function
 * `SZc` in the v2.1.220 binary):
 *
 *   1. a `[1m]` suffix on the model id,
 *   2. the 1M beta header on a `supports_1m_beta` model,
 *   3. `context.native_1m` in its model registry — **no suffix required**.
 *
 * Only path 3 survives into a transcript, and it is the one that matters:
 * Sonnet 5, Opus 4.7/4.8/5 and Fable 5 all carry `native_1m: true`, so they
 * run at 1M by default. Claude Code strips `[1m]` before writing the
 * transcript, so paths 1 and 2 leave no trace — but they only ever *raise*
 * a 200K model, and no 200K model in the registry has `native_1m`.
 *
 * The earlier implementation inferred the window from peak usage
 * (`peak > 200K ⇒ 1M`). That under-reports: it labelled 15 local Fable 5
 * sessions "of 200K" whose real window was 1M, simply because none of them
 * happened to exceed 200K.
 */

const STANDARD_WINDOW = 200_000;
const LONG_WINDOW = 1_000_000;

/**
 * Models whose registry entry sets `context.native_1m: true` — verified
 * against the model table in Claude Code v2.1.220.
 *
 * Deliberately an explicit list rather than a family prefix: the split runs
 * *within* families, not between them. Sonnet 5 is 1M while Sonnet 4.6 is
 * 200K; Opus 4.7 is 1M while Opus 4.6 is 200K. Matching on "opus" or
 * "sonnet" would mislabel every older release.
 */
// Mirrored in Rust at src-tauri/src/live_sessions.rs::NATIVE_1M_MODELS for
// the context-window-warning notification — keep both lists in sync.
const NATIVE_1M_MODELS = new Set([
  'claude-sonnet-5',
  'claude-opus-4-7',
  'claude-opus-4-8',
  'claude-opus-5',
  'claude-fable-5',
  'claude-mythos-5',
]);

/** Anthropic models known to be 200K. Everything else Anthropic-branded is
 *  unknown rather than assumed — a model released after this build should not
 *  be silently reported as 200K. */
const KNOWN_200K_PREFIXES = [
  'claude-3-5-',
  'claude-3-7-',
  'claude-sonnet-4-0',
  'claude-sonnet-4-5',
  'claude-sonnet-4-6',
  'claude-opus-4-0',
  'claude-opus-4-1',
  'claude-opus-4-5',
  'claude-opus-4-6',
  'claude-haiku-4-5',
];

export interface ContextWindow {
  /** Total window in tokens, or null when it cannot be established. */
  total: number | null;
  /** Display label for the window: "1M", "200K", or null. */
  label: string | null;
}

/**
 * The context window a session ran with, from its model id.
 *
 * Returns `total: null` for third-party models (GLM, Kimi, k3, …) and for
 * unrecognised Anthropic ids. Their windows are set per-provider — GLM is
 * configured to 1M by our own preset, but another provider could be
 * anything, and inventing a denominator would put a wrong percentage next to
 * a real number.
 */
export function windowFor(model: string | null | undefined): ContextWindow {
  if (!model) return { total: null, label: null };

  // Claude Code strips `[1m]` before writing a transcript, but a model id
  // reaching us from a provider config still carries it.
  const bare = model.replace(/\[1m\]$/i, '');

  if (NATIVE_1M_MODELS.has(bare) || /\[1m\]$/i.test(model)) {
    return { total: LONG_WINDOW, label: '1M' };
  }
  if (KNOWN_200K_PREFIXES.some((p) => bare.startsWith(p))) {
    return { total: STANDARD_WINDOW, label: '200K' };
  }
  return { total: null, label: null };
}

export interface ContextReadout {
  text: string;
  /** Fill fraction 0–100, or null when there is no known denominator. */
  pct: number | null;
  hint: string;
}

/**
 * Peak context as "495.6K of 1M", or bare "495.6K" when the window is
 * unknown. `peak` is the high-water mark of input + cache_read +
 * cache_creation across the session's assistant turns.
 */
export function contextReadout(peak: number, model: string | null): ContextReadout {
  const { total, label } = windowFor(model);
  if (total === null) {
    return {
      text: formatK(peak),
      pct: null,
      hint: 'Peak context. The window for this model is set by its provider and is not recorded in the transcript.',
    };
  }
  return {
    text: `${formatK(peak)} of ${label}`,
    pct: Math.min(100, Math.round((peak / total) * 100)),
    hint: `Peak context against this model's ${label} window.`,
  };
}

/** Local formatter rather than lib/format's `formatTokens`: this one keeps
 *  whole thousands clean ("57K" not "57.0K") while still showing a decimal
 *  where it carries information ("495.6K"). */
function formatK(n: number): string {
  if (n >= 1_000_000) {
    const m = n / 1_000_000;
    return `${Number.isInteger(m) ? m : m.toFixed(2)}M`;
  }
  if (n >= 1_000) {
    const k = n / 1_000;
    return `${Number.isInteger(k) ? k : k.toFixed(1)}K`;
  }
  return String(n);
}
