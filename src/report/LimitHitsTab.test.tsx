import { render, screen } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { LimitHitReport } from '../lib/generated/bindings';

const REPORT: LimitHitReport = {
  accounts: [
    {
      account_id: 'acc-1',
      email: 'work@example.com',
      five_hour_hits: 2,
      seven_day_hits: 1,
      hourly_distribution: Array.from({ length: 24 }, (_, h) => (h === 9 ? 2 : h === 14 ? 1 : 0)),
      top_projects: [
        { project: 'switchboard', cost_usd: 12.5 },
        { project: 'other-repo', cost_usd: 3.1 },
      ],
    },
  ],
};

const EMPTY_REPORT: LimitHitReport = { accounts: [] };

const ipcMock = vi.hoisted(() => ({ getLimitHitHistory: vi.fn() }));
vi.mock('../lib/ipc', () => ({ ipc: ipcMock }));

vi.mock('../lib/store', async () => {
  const actual = await vi.importActual<typeof import('../lib/store')>('../lib/store');
  const state = { sessionDataVersion: 0 };
  const useAppStore: any = (sel: any) => sel(state);
  useAppStore.getState = () => state;
  return { ...actual, useAppStore };
});

import { LimitHitsTab } from './LimitHitsTab';

describe('LimitHitsTab', () => {
  beforeEach(() => {
    ipcMock.getLimitHitHistory.mockClear();
  });

  it('shows an empty state when no account has any hits', async () => {
    ipcMock.getLimitHitHistory.mockResolvedValue(EMPTY_REPORT);
    render(<LimitHitsTab />);
    expect(await screen.findByText(/no limit hits yet/i)).toBeTruthy();
  });

  it('renders hit counts and top projects for accounts with history', async () => {
    ipcMock.getLimitHitHistory.mockResolvedValue(REPORT);
    render(<LimitHitsTab />);
    expect(await screen.findByText('work@example.com')).toBeTruthy();
    expect(screen.getByText(/2 × 5H/)).toBeTruthy();
    expect(screen.getByText(/1 × 7D/)).toBeTruthy();
    expect(screen.getByText('switchboard')).toBeTruthy();
    expect(screen.getByText('$12.50')).toBeTruthy();
  });
});
