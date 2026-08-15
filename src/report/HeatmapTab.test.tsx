import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { SessionEvent, DailyAccountBucket } from '../lib/types';

const ipcMock = vi.hoisted(() => ({
  getSessionHistory: vi.fn(),
  getDailyAccountBreakdown: vi.fn(),
}));
vi.mock('../lib/ipc', () => ({ ipc: ipcMock }));

vi.mock('../lib/store', async () => {
  const actual = await vi.importActual<typeof import('../lib/store')>('../lib/store');
  const state = {
    sessionDataVersion: 0,
    accounts: [
      { slot: 0, email: 'work@x.com', account_uuid: 'acc1', org_name: null, org_uuid: null, subscription_type: null, source: 'OAuth', is_active: true, cached_usage: null, last_error: null },
    ],
  };
  const useAppStore: any = (sel: any) => sel(state);
  useAppStore.getState = () => state;
  return { ...actual, useAppStore };
});

import { HeatmapTab } from './HeatmapTab';

describe('HeatmapTab — account attribution', () => {
  beforeEach(() => {
    ipcMock.getSessionHistory.mockClear();
    ipcMock.getDailyAccountBreakdown.mockClear();
  });

  it('shows the account split for a day in its hover tooltip', async () => {
    const today = new Date().toISOString().slice(0, 10);
    const events: SessionEvent[] = [
      {
        ts: new Date().toISOString(),
        project: 'p',
        model: 'm',
        input_tokens: 10,
        output_tokens: 5,
        cache_read_tokens: 0,
        cache_creation_5m_tokens: 0,
        cache_creation_1h_tokens: 0,
        cost_usd: 0.01,
        source_file: 'a.jsonl',
        source_line: 0,
        event_id: 'e1',
        account_uuid: 'acc1',
      },
    ];
    ipcMock.getSessionHistory.mockResolvedValue(events);
    const accountBuckets: DailyAccountBucket[] = [
      { date: today, accounts: [{ account_uuid: 'acc1', input_tokens: 10, output_tokens: 5, cost_usd: 0.01 }] },
    ];
    ipcMock.getDailyAccountBreakdown.mockResolvedValue(accountBuckets);

    render(<HeatmapTab />);

    const cell = await screen.findByTestId(`heatmap-cell-${today}`);
    fireEvent.mouseEnter(cell);

    expect(screen.getByText(/work 100%/)).toBeInTheDocument();
  });
});
