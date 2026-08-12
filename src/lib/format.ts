export function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

export function formatCost(usd: number): string {
  return usd < 0.01 ? '<$0.01' : `$${usd.toFixed(2)}`;
}

/** "just now" / "3m ago" / "2h ago" for an ISO timestamp; '' when unparsable. */
export function formatRelativeTime(iso: string): string {
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return '';
  const diff = Date.now() - t;
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  return `${hours}h ago`;
}

/** Minutes between two ISO timestamps; 0 when unparsable or non-positive (a
 *  session with no usage can end before it "starts"). Kept separate from
 *  `formatDuration` so callers can sum several sessions' spans before
 *  formatting the total (e.g. a project's combined running time). */
export function durationMinutes(startIso: string, endIso: string): number {
  const a = new Date(startIso).getTime();
  const b = new Date(endIso).getTime();
  if (!Number.isFinite(a) || !Number.isFinite(b) || b <= a) return 0;
  return (b - a) / 60000;
}

/** "42m" / "1h23m" / "2d 3h" for a minute count; '' when zero or negative. */
export function formatDurationMinutes(totalMins: number): string {
  const mins = Math.round(totalMins);
  if (mins <= 0) return '';
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h${String(mins % 60).padStart(2, '0')}m`;
  const days = Math.floor(hours / 24);
  const remHours = hours % 24;
  return remHours > 0 ? `${days}d ${remHours}h` : `${days}d`;
}

/** "42m" / "1h23m" span between two ISO timestamps; '' when unparsable or
 *  non-positive (a session with no usage can end before it "starts"). */
export function formatDuration(startIso: string, endIso: string): string {
  return formatDurationMinutes(durationMinutes(startIso, endIso));
}
