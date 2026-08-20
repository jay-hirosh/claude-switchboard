/** Local calendar day of `iso` as `YYYY-MM-DD`.
 *
 * Built from the Date's local getters rather than `toISOString()` (which is
 * UTC) or a locale format (which varies by machine), because this string is
 * the grouping key for a row's cost. It has to agree with the backend's
 * `get_daily_trends`, which buckets on `chrono::Local` — otherwise the Cost
 * tab and the Trends bar for the same day would disagree near midnight. */
export function localDayKey(iso: string): string {
  const d = new Date(iso);
  const month = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${d.getFullYear()}-${month}-${day}`;
}

/** Local day key 6 calendar days before today — the start of the Dashboard
 * tab's "This Week" rolling window, matching the backend's `week_range_utc`
 * (today plus the 6 preceding local calendar days). */
export function weekStartDayKey(): string {
  const d = new Date();
  d.setDate(d.getDate() - 6);
  return localDayKey(d.toISOString());
}

export function formatDayLabel(dayKey: string): string {
  const today = localDayKey(new Date().toISOString());
  if (dayKey === today) return 'Today';
  const yesterday = new Date();
  yesterday.setDate(yesterday.getDate() - 1);
  if (dayKey === localDayKey(yesterday.toISOString())) return 'Yesterday';
  // Parse as local midnight — `new Date('2026-07-26')` would be parsed as UTC
  // and could render the previous day west of Greenwich.
  const [y, m, d] = dayKey.split('-').map(Number);
  return new Date(y, m - 1, d).toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
  });
}
