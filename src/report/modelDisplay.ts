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
