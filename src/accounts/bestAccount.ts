import type { AccountListEntry } from '../lib/generated/bindings';

/**
 * Picks the managed account with the most remaining headroom, or null when
 * no account is worth flagging.
 *
 * "Headroom" is judged by the WORST of the two buckets
 * (max of 5H% and 7D%) — whichever bucket is closer to blocking you is the
 * one that matters for "can I use this account right now."
 *
 * Suppression rules (all return null):
 *  - fewer than two managed accounts;
 *  - no eligible account (eligible = polled at least once and not errored);
 *  - the best account is already the active one;
 *  - the best account's lead over the active one is under `marginPct`
 *    points — unless the active account is itself ineligible (errored),
 *    in which case there is no trustworthy baseline and the best eligible
 *    account is flagged regardless of margin.
 */
export function computeBestAccountUuid(
  accounts: AccountListEntry[],
  marginPct = 3,
): string | null {
  if (accounts.length < 2) return null;

  const constraint = (a: AccountListEntry) => {
    const s = a.cached_usage?.snapshot;
    return Math.max(s?.five_hour?.utilization ?? 0, s?.seven_day?.utilization ?? 0);
  };
  const eligible = accounts.filter((a) => !a.last_error && a.cached_usage);
  if (eligible.length === 0) return null;

  const best = eligible.reduce((a, b) => (constraint(a) <= constraint(b) ? a : b));
  if (best.is_active) return null;

  const active = accounts.find((a) => a.is_active);
  const activeEligible =
    active && eligible.find((a) => a.account_uuid === active.account_uuid);
  if (activeEligible && constraint(activeEligible) - constraint(best) < marginPct) {
    return null;
  }

  return best.account_uuid;
}
