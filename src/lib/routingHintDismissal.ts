function key(accountId: string): string {
  return `routing-hint-dismissed:${accountId}`;
}

/**
 * A dismissal is scoped to the 7D window it was dismissed in: the stored
 * value is the bucket's `resetsAt` at dismiss time, so once the window rolls
 * over to a new `resetsAt` the comparison naturally fails and the hint can
 * fire again — no explicit expiry bookkeeping needed.
 */
export function isRoutingHintDismissed(accountId: string, resetsAt: string | null): boolean {
  if (typeof localStorage === 'undefined') return false;
  return localStorage.getItem(key(accountId)) === (resetsAt ?? '');
}

export function dismissRoutingHint(accountId: string, resetsAt: string | null): void {
  try {
    localStorage.setItem(key(accountId), resetsAt ?? '');
  } catch {
    // Safari private mode / quota-exceeded — dismissal still applies for
    // this render, just isn't persisted across launches.
  }
}
