import { describe, it, expect } from 'vitest';
import type { AccountListEntry, CachedUsage } from '../lib/generated/bindings';
import { computeBestAccountUuid } from './bestAccount';

function cached(fiveHour: number | null, sevenDay: number | null): CachedUsage {
  return {
    snapshot: {
      five_hour: fiveHour === null ? null : { utilization: fiveHour, resets_at: null },
      seven_day: sevenDay === null ? null : { utilization: sevenDay, resets_at: null },
      seven_day_sonnet: null,
      seven_day_opus: null,
      extra_usage: null,
    },
    account_id: 'x',
    account_email: 'x@x.com',
    last_error: null,
    burn_rate: null,
    auth_source: 'OAuth',
  } as CachedUsage;
}

function acct(
  uuid: string,
  opts: {
    active?: boolean;
    fiveHour?: number;
    sevenDay?: number;
    lastError?: string;
    unpolled?: boolean;
  } = {},
): AccountListEntry {
  return {
    slot: 1,
    email: `${uuid}@x.com`,
    account_uuid: uuid,
    org_name: null,
    org_uuid: null,
    subscription_type: 'pro',
    source: 'OAuth',
    is_active: opts.active ?? false,
    cached_usage: opts.unpolled
      ? null
      : cached(opts.fiveHour ?? 0, opts.sevenDay ?? 0),
    last_error: opts.lastError ?? null,
  } as AccountListEntry;
}

describe('computeBestAccountUuid', () => {
  it('flags the account whose WORST bucket is lowest, not the lowest 5H', () => {
    // A (active): constraint 70. B: 5h is 20 but 7d 55 governs → 55.
    // C: lowest 5h overall (10) but 7d 65 governs → 65. Best is B.
    const accounts = [
      acct('A', { active: true, fiveHour: 70, sevenDay: 40 }),
      acct('B', { fiveHour: 20, sevenDay: 55 }),
      acct('C', { fiveHour: 10, sevenDay: 65 }),
    ];
    expect(computeBestAccountUuid(accounts)).toBe('B');
  });

  it('returns null when the active account is already best', () => {
    const accounts = [
      acct('A', { active: true, fiveHour: 10, sevenDay: 10 }),
      acct('B', { fiveHour: 50, sevenDay: 50 }),
    ];
    expect(computeBestAccountUuid(accounts)).toBeNull();
  });

  it('suppresses leads smaller than the margin, including exact ties', () => {
    const twoPointLead = [
      acct('A', { active: true, fiveHour: 50, sevenDay: 50 }),
      acct('B', { fiveHour: 48, sevenDay: 48 }),
    ];
    expect(computeBestAccountUuid(twoPointLead)).toBeNull();

    const tie = [
      acct('A', { active: true, fiveHour: 50, sevenDay: 50 }),
      acct('B', { fiveHour: 50, sevenDay: 50 }),
    ];
    expect(computeBestAccountUuid(tie)).toBeNull();
  });

  it('flags leads at or above the margin', () => {
    const accounts = [
      acct('A', { active: true, fiveHour: 50, sevenDay: 50 }),
      acct('B', { fiveHour: 47, sevenDay: 47 }),
    ];
    expect(computeBestAccountUuid(accounts)).toBe('B');
  });

  it('excludes errored accounts even when their numbers look best', () => {
    const accounts = [
      acct('A', { active: true, fiveHour: 60, sevenDay: 60 }),
      acct('B', { fiveHour: 5, sevenDay: 5, lastError: 'rate-limited (429)' }),
      acct('C', { fiveHour: 30, sevenDay: 30 }),
    ];
    expect(computeBestAccountUuid(accounts)).toBe('C');
  });

  it('excludes never-polled accounts (no cached_usage)', () => {
    const accounts = [
      acct('A', { active: true, fiveHour: 60, sevenDay: 60 }),
      acct('B', { unpolled: true }),
      acct('C', { fiveHour: 30, sevenDay: 30 }),
    ];
    expect(computeBestAccountUuid(accounts)).toBe('C');
  });

  it('skips the margin check when the active account itself is errored', () => {
    // Active is ineligible → no reliable baseline → best eligible wins
    // regardless of how its numbers compare to active's stale ones.
    const accounts = [
      acct('A', { active: true, fiveHour: 10, sevenDay: 10, lastError: 'auth_required' }),
      acct('B', { fiveHour: 90, sevenDay: 90 }),
    ];
    expect(computeBestAccountUuid(accounts)).toBe('B');
  });

  it('returns null with fewer than two managed accounts', () => {
    expect(computeBestAccountUuid([acct('A', { active: true })])).toBeNull();
    expect(computeBestAccountUuid([])).toBeNull();
  });

  it('returns null when every account is ineligible', () => {
    const accounts = [
      acct('A', { active: true, lastError: 'auth_required' }),
      acct('B', { unpolled: true }),
    ];
    expect(computeBestAccountUuid(accounts)).toBeNull();
  });

  it('treats a missing bucket as 0 for that bucket', () => {
    // B reports only 7d (5h null → 0); constraint is 20.
    const accounts = [
      acct('A', { active: true, fiveHour: 60, sevenDay: 60 }),
      { ...acct('B'), cached_usage: cached(null, 20) },
    ];
    expect(computeBestAccountUuid(accounts)).toBe('B');
  });
});
