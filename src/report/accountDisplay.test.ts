import { describe, it, expect } from 'vitest';
import { accountColor, colorForAccount, labelForAccount } from './accountDisplay';
import type { AccountListEntry } from '../lib/generated/bindings';

function account(overrides: Partial<AccountListEntry> = {}): AccountListEntry {
  return {
    slot: 0,
    email: 'work@example.com',
    account_uuid: 'uuid-1',
    org_name: null,
    org_uuid: null,
    subscription_type: null,
    source: 'OAuth',
    is_active: false,
    cached_usage: null,
    last_error: null,
    ...overrides,
  };
}

describe('accountColor', () => {
  it('is deterministic per slot and cycles the palette', () => {
    expect(accountColor(0)).toBe('var(--color-account-1)');
    expect(accountColor(4)).toBe(accountColor(0));
  });
});

describe('colorForAccount', () => {
  it('resolves the color of the account matching accountUuid', () => {
    const accounts = [account({ slot: 2, account_uuid: 'uuid-a' })];
    expect(colorForAccount('uuid-a', accounts)).toBe(accountColor(2));
  });

  it('falls back to the muted color for null or unmatched uuids', () => {
    const accounts = [account({ slot: 0, account_uuid: 'uuid-a' })];
    expect(colorForAccount(null, accounts)).toBe('var(--color-text-muted)');
    expect(colorForAccount('uuid-missing', accounts)).toBe('var(--color-text-muted)');
  });
});

describe('labelForAccount', () => {
  it('shows the email local-part for a matched account', () => {
    const accounts = [account({ account_uuid: 'uuid-a', email: 'jay@work.com' })];
    expect(labelForAccount('uuid-a', accounts)).toBe('jay');
  });

  it('shows "Unknown" for null or unmatched uuids', () => {
    const accounts = [account({ account_uuid: 'uuid-a' })];
    expect(labelForAccount(null, accounts)).toBe('Unknown');
    expect(labelForAccount('uuid-missing', accounts)).toBe('Unknown');
  });
});
