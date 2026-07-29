import type { Provider } from '../lib/generated/bindings';

export type Resolution = { kind: 'resolved'; providerId: string } | { kind: 'unresolved' };

const MODEL_KEYS = [
  'ANTHROPIC_MODEL',
  'ANTHROPIC_SMALL_FAST_MODEL',
  'ANTHROPIC_DEFAULT_OPUS_MODEL',
  'ANTHROPIC_DEFAULT_SONNET_MODEL',
  'ANTHROPIC_DEFAULT_HAIKU_MODEL',
  'ANTHROPIC_DEFAULT_FABLE_MODEL',
] as const;

/**
 * Applied to BOTH sides of the comparison. Claude Code strips the `[1m]`
 * context modifier before writing the transcript, so the session model
 * arrives pre-normalized while the provider config still carries it —
 * normalizing only one side silently fails to match GLM and k3.
 */
export function norm(s: string): string {
  return s.trim().toLowerCase().replace(/\[1m\]$/, '');
}

/** Anthropic-style ids a session may record when served by a relay. */
function looksAnthropic(model: string): boolean {
  return norm(model).startsWith('claude-');
}

export function resolveProvider(model: string | null, providers: Provider[]): Resolution {
  if (!model) return { kind: 'unresolved' };
  const needle = norm(model);

  // Deterministic ordering: two providers may declare the same model id.
  const ordered = [...providers].sort(
    (a, b) => a.sort_index - b.sort_index || a.id.localeCompare(b.id),
  );

  for (const p of ordered) {
    for (const key of MODEL_KEYS) {
      const configured = p.env[key];
      if (configured && norm(configured) === needle) {
        return { kind: 'resolved', providerId: p.id };
      }
    }
  }

  // Only a genuine Anthropic id falls back to official. A relay id that
  // merely looks Anthropic-style (claude-sonnet-4-5-thinking) reaches here
  // when its provider has been deleted, and must prompt rather than resume
  // silently on the wrong model.
  if (looksAnthropic(model) && !/-thinking$|^claude-\d/.test(needle)) {
    const off = providers.find((p) => p.kind === 'official');
    if (off) return { kind: 'resolved', providerId: off.id };
  }

  return { kind: 'unresolved' };
}
