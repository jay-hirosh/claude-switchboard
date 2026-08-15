import { render, screen } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { RepoStats } from '../lib/types';

const ipcMock = vi.hoisted(() => ({ getRepoBreakdown: vi.fn() }));
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

import { RepoTab } from './RepoTab';

describe('RepoTab — account attribution', () => {
  beforeEach(() => {
    ipcMock.getRepoBreakdown.mockClear();
  });

  it('shows a badge for each account that touched a repo', async () => {
    const repos: RepoStats[] = [
      {
        repo: 'switchboard',
        session_count: 2,
        total_tokens: 100,
        total_cost_usd: 1.0,
        projects: [{ project: 'switchboard', cwd: '/repo', session_count: 2, total_tokens: 100, total_cost_usd: 1.0, account_uuids: ['acc1', 'acc2'] }],
        account_uuids: ['acc1', 'acc2'],
      },
    ];
    ipcMock.getRepoBreakdown.mockResolvedValue(repos);

    render(<RepoTab />);

    await screen.findByText('switchboard');
    expect(screen.getByText('work')).toBeInTheDocument();
    expect(screen.getByText('personal')).toBeInTheDocument();
  });
});
