import { render, screen, fireEvent, within } from '@testing-library/react';
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
    // A single-project repo isn't expandable, so these are the repo header's
    // badges and nothing else.
    expect(screen.getByText('work')).toBeInTheDocument();
    expect(screen.getByText('personal')).toBeInTheDocument();
  });

  it('renders an "Unknown" badge for unattributed sessions instead of dropping them', async () => {
    const repos: RepoStats[] = [
      {
        repo: 'switchboard',
        session_count: 11,
        total_tokens: 100,
        total_cost_usd: 1.0,
        // 10 pre-feature sessions plus one under `acc1`: showing only "work"
        // would read as "100% work account" when it's mostly unknown.
        projects: [{ project: 'switchboard', cwd: '/repo', session_count: 11, total_tokens: 100, total_cost_usd: 1.0, account_uuids: [null, 'acc1'] }],
        account_uuids: [null, 'acc1'],
      },
    ];
    ipcMock.getRepoBreakdown.mockResolvedValue(repos);

    render(<RepoTab />);

    await screen.findByText('switchboard');
    expect(screen.getByText('Unknown')).toBeInTheDocument();
    expect(screen.getByText('work')).toBeInTheDocument();
  });

  it('badges each project row too, not just the repo header', async () => {
    const repos: RepoStats[] = [
      {
        repo: 'switchboard',
        session_count: 2,
        total_tokens: 100,
        total_cost_usd: 2.0,
        projects: [
          { project: 'app', cwd: '/repo/app', session_count: 1, total_tokens: 50, total_cost_usd: 1.0, account_uuids: ['acc1'] },
          { project: 'core', cwd: '/repo/core', session_count: 1, total_tokens: 50, total_cost_usd: 1.0, account_uuids: [null] },
        ],
        account_uuids: ['acc1', null],
      },
    ];
    ipcMock.getRepoBreakdown.mockResolvedValue(repos);

    render(<RepoTab />);

    // Collapsed: only the repo header's badges are mounted.
    const header = await screen.findByRole('button', { expanded: false });
    expect(screen.getAllByText('work')).toHaveLength(1);
    expect(screen.getAllByText('Unknown')).toHaveLength(1);

    fireEvent.click(header);

    // Expanded: each project row carries its own badge, on top of the header's.
    // The cwd span's enclosing div is the project row's identity column, which
    // is exactly where that row's badges live.
    const appRow = screen.getByText('/repo/app').closest('div')!;
    expect(within(appRow).getByText('work')).toBeInTheDocument();
    const coreRow = screen.getByText('/repo/core').closest('div')!;
    expect(within(coreRow).getByText('Unknown')).toBeInTheDocument();

    expect(screen.getAllByText('work')).toHaveLength(2);
    expect(screen.getAllByText('Unknown')).toHaveLength(2);
  });
});
