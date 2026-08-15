import { render, screen } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { CacheStats, AccountCacheStats } from '../lib/types';

const ipcMock = vi.hoisted(() => ({
  getCacheStats: vi.fn(),
  getCacheStatsByAccount: vi.fn(),
}));
vi.mock('../lib/ipc', () => ({ ipc: ipcMock }));

vi.mock('../lib/store', async () => {
  const actual = await vi.importActual<typeof import('../lib/store')>('../lib/store');
  const state = {
    sessionDataVersion: 0,
    accounts: [
      { slot: 0, email: 'work@x.com', account_uuid: 'acc1', org_name: null, org_uuid: null, subscription_type: null, source: 'OAuth', is_active: true, cached_usage: null, last_error: null },
      { slot: 1, email: 'personal@x.com', account_uuid: 'acc2', org_name: null, org_uuid: null, subscription_type: null, source: 'OAuth', is_active: false, cached_usage: null, last_error: null },
    ],
  };
  const useAppStore: any = (sel: any) => sel(state);
  useAppStore.getState = () => state;
  return { ...actual, useAppStore };
});

import { CacheTab } from './CacheTab';

describe('CacheTab — per-account cards', () => {
  beforeEach(() => {
    ipcMock.getCacheStats.mockClear();
    ipcMock.getCacheStatsByAccount.mockClear();
  });

  it('shows one card per account alongside the total', async () => {
    const total: CacheStats = {
      total_cache_read_tokens: 110,
      total_cache_creation_tokens: 20,
      estimated_savings_usd: 1.5,
      hit_ratio: 0.85,
    };
    const byAccount: AccountCacheStats[] = [
      { account_uuid: 'acc1', total_cache_read_tokens: 100, total_cache_creation_tokens: 15, estimated_savings_usd: 1.3, hit_ratio: 0.87 },
      { account_uuid: 'acc2', total_cache_read_tokens: 10, total_cache_creation_tokens: 5, estimated_savings_usd: 0.2, hit_ratio: 0.67 },
    ];
    ipcMock.getCacheStats.mockResolvedValue(total);
    ipcMock.getCacheStatsByAccount.mockResolvedValue(byAccount);

    render(<CacheTab />);

    await screen.findByText('Total');
    expect(screen.getByText('work')).toBeInTheDocument();
    expect(screen.getByText('personal')).toBeInTheDocument();
  });
});
