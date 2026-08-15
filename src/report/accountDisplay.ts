import type { AccountListEntry } from '../lib/generated/bindings';

const PALETTE = [
  'var(--color-account-1)',
  'var(--color-account-2)',
  'var(--color-account-3)',
  'var(--color-account-4)',
] as const;

const UNKNOWN_COLOR = 'var(--color-text-muted)';

/** Deterministic color for an account, keyed by its stable `slot` number so
 *  the color doesn't shift as other accounts are added or removed. Cycles
 *  the 4-color palette for a 5th+ account rather than growing it. */
export function accountColor(slot: number): string {
  return PALETTE[slot % PALETTE.length];
}

/** Color for a row/entry's `account_uuid` (as returned by the account-aware
 *  report commands). `null` — no attribution, either pre-feature history or
 *  a gap with no managed account live — and an unmatched uuid (the account
 *  was since removed) both render muted. */
export function colorForAccount(accountUuid: string | null, accounts: AccountListEntry[]): string {
  if (!accountUuid) return UNKNOWN_COLOR;
  const account = accounts.find((a) => a.account_uuid === accountUuid);
  return account ? accountColor(account.slot) : UNKNOWN_COLOR;
}

/** Short display label for an account badge: the email's local-part, matching
 *  how tight-space UI elsewhere favors recognizable identity over the full
 *  address. */
export function labelForAccount(accountUuid: string | null, accounts: AccountListEntry[]): string {
  if (!accountUuid) return 'Unknown';
  const account = accounts.find((a) => a.account_uuid === accountUuid);
  return account ? account.email.split('@')[0] : 'Unknown';
}
