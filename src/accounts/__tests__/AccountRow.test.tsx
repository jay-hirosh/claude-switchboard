import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import type { AccountListEntry, CachedUsage } from '../../lib/generated/bindings';

const ipcMock = vi.hoisted(() => ({
  getWarmupState: vi.fn().mockResolvedValue({ warmup_enabled: false, schedule: { type: 'Off' } }),
  getWarmupConsentGranted: vi.fn().mockResolvedValue(true),
  setWarmupEnabled: vi.fn().mockResolvedValue(undefined),
  setAccountSchedule: vi.fn().mockResolvedValue(undefined),
  warmupAccountNow: vi.fn().mockResolvedValue('Success'),
  grantWarmupConsent: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../lib/ipc', () => ({ ipc: ipcMock }));

import { AccountRow } from '../AccountRow';

function cachedUsage(util5h: number | null, util7d: number | null): CachedUsage {
  return {
    snapshot: {
      five_hour:
        util5h === null
          ? null
          : { utilization: util5h, resets_at: new Date(Date.now() + 3600_000).toISOString() },
      seven_day:
        util7d === null
          ? null
          : { utilization: util7d, resets_at: new Date(Date.now() + 86400_000).toISOString() },
      seven_day_sonnet: null,
      seven_day_opus: null,
      extra_usage: null,
      fetched_at: new Date(Date.now() - 3 * 60_000).toISOString(),
    },
    account_id: 'uuid-1',
    account_email: 'a@x.com',
    last_error: 'rate-limited (429)',
    burn_rate: null,
    auth_source: 'OAuth',
  } as CachedUsage;
}

function entry(overrides: Partial<AccountListEntry>): AccountListEntry {
  return {
    slot: 1,
    email: 'a@x.com',
    account_uuid: 'uuid-1',
    org_name: null,
    org_uuid: null,
    subscription_type: 'pro',
    source: 'OAuth',
    is_active: true,
    cached_usage: null,
    last_error: null,
    ...overrides,
  } as AccountListEntry;
}

describe('AccountRow error rendering', () => {
  it('shows last-good meters with a stale hint on transient errors', () => {
    render(
      <AccountRow
        entry={entry({ last_error: 'rate-limited (429)', cached_usage: cachedUsage(42, 63) })}
        thresholds={[75, 90]}
      />,
    );
    // Last-known-good data stays visible instead of being hidden by the error.
    expect(screen.getByText('42%')).toBeTruthy();
    expect(screen.getByText('63%')).toBeTruthy();
    expect(screen.queryByText('usage unavailable')).toBeNull();
    // …and a hint explains the data is stale.
    expect(screen.getByText(/rate-limited/i)).toBeTruthy();
    expect(screen.getByText(/ago/)).toBeTruthy();
  });

  it('shows "usage unavailable" only when there is no usable snapshot', () => {
    render(
      <AccountRow
        entry={entry({ last_error: 'rate-limited (429)', cached_usage: cachedUsage(null, null) })}
        thresholds={[75, 90]}
      />,
    );
    expect(screen.getByText('usage unavailable')).toBeTruthy();
  });

  it('auth_required keeps the re-authenticate action even when data exists', () => {
    const cached = { ...cachedUsage(42, 63), last_error: 'auth_required' };
    render(
      <AccountRow
        entry={entry({ last_error: 'auth_required', cached_usage: cached })}
        thresholds={[75, 90]}
        onReauth={() => {}}
      />,
    );
    expect(screen.getByText(/token expired/i)).toBeTruthy();
    expect(screen.getByRole('button', { name: /re-authenticate/i })).toBeTruthy();
    expect(screen.queryByText('42%')).toBeNull();
  });

  it('renders meters normally when there is no error', () => {
    render(
      <AccountRow
        entry={entry({ last_error: null, cached_usage: cachedUsage(42, 63) })}
        thresholds={[75, 90]}
      />,
    );
    expect(screen.getByText('42%')).toBeTruthy();
    expect(screen.queryByText(/rate-limited/i)).toBeNull();
  });
});

describe('AccountRow warm-up section', () => {
  it('is collapsed by default — controls hidden, disclosure line visible', () => {
    render(<AccountRow entry={entry({})} thresholds={[75, 90]} />);
    expect(screen.getByRole('button', { name: /warm-up/i })).toBeTruthy();
    expect(screen.queryByRole('button', { name: /warm up now/i })).toBeNull();
    expect(screen.queryByRole('switch')).toBeNull();
  });

  it('expands on click to reveal the warm-up controls', async () => {
    ipcMock.getWarmupState.mockResolvedValueOnce({
      warmup_enabled: true,
      schedule: { type: 'Every5h', anchor: { hour: 6, minute: 0 } },
    });
    render(<AccountRow entry={entry({})} thresholds={[75, 90]} />);
    fireEvent.click(screen.getByRole('button', { name: /warm-up/i }));
    expect(screen.getByRole('switch')).toBeTruthy();
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /warm up now/i })).toBeTruthy(),
    );
  });

  it('collapses again on a second click', async () => {
    render(<AccountRow entry={entry({})} thresholds={[75, 90]} />);
    const disclosure = screen.getByRole('button', { name: /warm-up/i });
    fireEvent.click(disclosure);
    expect(screen.getByRole('switch')).toBeTruthy();
    fireEvent.click(disclosure);
    expect(screen.queryByRole('switch')).toBeNull();
  });

  it('summarizes the schedule on the collapsed line', async () => {
    ipcMock.getWarmupState.mockResolvedValueOnce({
      warmup_enabled: true,
      schedule: { type: 'Every5h', anchor: { hour: 6, minute: 0 } },
    });
    render(<AccountRow entry={entry({})} thresholds={[75, 90]} />);
    expect(await screen.findByText('Every 5h')).toBeTruthy();
    // Summary lives on the collapsed line — controls stay hidden.
    expect(screen.queryByRole('switch')).toBeNull();
  });
});

describe('AccountRow best-available badge', () => {
  it('renders the pill when isBest is set', () => {
    render(
      <AccountRow
        entry={entry({ is_active: false })}
        thresholds={[75, 90]}
        isBest
      />,
    );
    expect(screen.getByText('Best available')).toBeTruthy();
  });

  it('renders no pill when isBest is omitted', () => {
    render(<AccountRow entry={entry({ is_active: false })} thresholds={[75, 90]} />);
    expect(screen.queryByText('Best available')).toBeNull();
  });
});
