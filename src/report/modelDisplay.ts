export const MODEL_VARIANT: Record<string, 'opus' | 'sonnet' | 'haiku' | 'default'> = {
  opus: 'opus',
  sonnet: 'sonnet',
  haiku: 'haiku',
};

export function modelKey(name: string): string {
  const lower = name.toLowerCase();
  if (lower.includes('opus')) return 'opus';
  if (lower.includes('sonnet')) return 'sonnet';
  if (lower.includes('haiku')) return 'haiku';
  return 'default';
}

export function shortName(model: string): string {
  const m = model.match(/(opus|sonnet|haiku)-(\d+(?:-\d+)?)/i);
  return m ? `${m[1]} ${m[2]}` : model;
}

/** Badge text for a model. Anthropic families collapse to their tier name
 * (opus/sonnet/haiku/fable); third-party / relay models keep a cleaned
 * version of their real name so the badge says "glm-5.2" or "k3" instead of
 * a generic "default" — the family color variant is still driven by
 * `modelKey` (which stays "default", i.e. neutral, for non-Anthropic). */
export function modelLabel(name: string): string {
  const lower = name.toLowerCase();
  if (lower.includes('opus')) return 'opus';
  if (lower.includes('sonnet')) return 'sonnet';
  if (lower.includes('haiku')) return 'haiku';
  if (lower.includes('fable')) return 'fable';
  return name
    .replace(/-(highspeed|streaming)$/i, '')
    .replace(/-\d{8}$/, '')
    .replace(/-for-coding$/i, '');
}
