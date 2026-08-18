import { render, screen } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { SessionEvent, DailyPatternReport } from '../lib/types';

const ipcMock = vi.hoisted(() => ({
  getSessionHistory: vi.fn(),
  getTodayPattern: vi.fn(),
  getTodayRepoBreakdown: vi.fn(),
  getTodayCacheStats: vi.fn(),
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

import { TodayTab } from './TodayTab';

const EMPTY_PATTERN: DailyPatternReport = {
  cells: [],
  hourly_totals: Array.from({ length: 24 }, (_, hour) => ({ hour, tokens: 0, cost_usd: 0, request_count: 0 })),
  active_days: 0,
  lookback_days: 1,
  plan: null,
};

let idCounter = 0;
function ev(over: Partial<SessionEvent> & { ts: string; source_file: string; cost_usd: number }): SessionEvent {
  idCounter += 1;
  return {
    project: 'life-os',
    model: 'claude-sonnet-5',
    input_tokens: 100,
    output_tokens: 50,
    cache_read_tokens: 0,
    cache_creation_5m_tokens: 0,
    cache_creation_1h_tokens: 0,
    source_line: 0,
    event_id: `id-${idCounter}`,
    account_uuid: null,
    ...over,
  };
}

describe('TodayTab', () => {
  beforeEach(() => {
    ipcMock.getSessionHistory.mockReset();
    ipcMock.getTodayPattern.mockReset();
    ipcMock.getTodayRepoBreakdown.mockReset();
    ipcMock.getTodayCacheStats.mockReset();
    ipcMock.getTodayPattern.mockResolvedValue(EMPTY_PATTERN);
    ipcMock.getTodayRepoBreakdown.mockResolvedValue([]);
    ipcMock.getTodayCacheStats.mockResolvedValue({
      total_cache_read_tokens: 0,
      total_cache_creation_tokens: 0,
      estimated_savings_usd: 0,
      hit_ratio: 0,
    });
  });

  it('shows an empty state when nothing happened today', async () => {
    ipcMock.getSessionHistory.mockResolvedValue([]);
    render(<TodayTab />);
    expect(await screen.findByText('No activity yet today')).toBeInTheDocument();
  });

  it('sums only today\'s events into the headline row and excludes yesterday', async () => {
    const now = new Date();
    const todayIso = now.toISOString();
    const yesterdayIso = new Date(now.getTime() - 26 * 60 * 60 * 1000).toISOString();
    ipcMock.getSessionHistory.mockResolvedValue([
      ev({ ts: todayIso, source_file: 'proj/today-a.jsonl', cost_usd: 0.5, input_tokens: 100, output_tokens: 50 }),
      ev({ ts: todayIso, source_file: 'proj/today-b.jsonl', cost_usd: 0.25, input_tokens: 40, output_tokens: 10 }),
      ev({ ts: yesterdayIso, source_file: 'proj/yesterday.jsonl', cost_usd: 9.99, input_tokens: 1000, output_tokens: 1000 }),
    ]);

    render(<TodayTab />);

    expect(await screen.findByTestId('today-cost')).toHaveTextContent('$0.75');
    expect(screen.getByTestId('today-tokens')).toHaveTextContent('200'); // 150 + 50, headline tokens = input+output
    expect(screen.getByTestId('today-sessions')).toHaveTextContent('2');
    expect(screen.getAllByTestId('today-session-row')).toHaveLength(2);
  });

  it('folds today\'s events by model in the model section', async () => {
    const todayIso = new Date().toISOString();
    ipcMock.getSessionHistory.mockResolvedValue([
      ev({ ts: todayIso, source_file: 'proj/a.jsonl', model: 'claude-opus-5', cost_usd: 1.0, input_tokens: 10, output_tokens: 5 }),
      ev({ ts: todayIso, source_file: 'proj/b.jsonl', model: 'claude-sonnet-5', cost_usd: 0.2, input_tokens: 8, output_tokens: 2 }),
    ]);

    render(<TodayTab />);

    await screen.findByTestId('today-cost');
    expect(screen.getByText('opus')).toBeInTheDocument();
    expect(screen.getByText('sonnet')).toBeInTheDocument();
  });

  it('renders a repo card from getTodayRepoBreakdown', async () => {
    const todayIso = new Date().toISOString();
    ipcMock.getSessionHistory.mockResolvedValue([
      ev({ ts: todayIso, source_file: 'proj/a.jsonl', cost_usd: 1.0 }),
    ]);
    ipcMock.getTodayRepoBreakdown.mockResolvedValue([
      {
        repo: 'claude-switchboard',
        session_count: 1,
        total_tokens: 150,
        total_cost_usd: 1.0,
        projects: [],
        account_uuids: [null],
      },
    ]);

    render(<TodayTab />);

    expect(await screen.findByText('claude-switchboard')).toBeInTheDocument();
  });

  it('renders cache stats from getTodayCacheStats', async () => {
    const todayIso = new Date().toISOString();
    ipcMock.getSessionHistory.mockResolvedValue([
      ev({ ts: todayIso, source_file: 'proj/a.jsonl', cost_usd: 1.0 }),
    ]);
    ipcMock.getTodayCacheStats.mockResolvedValue({
      total_cache_read_tokens: 900,
      total_cache_creation_tokens: 100,
      estimated_savings_usd: 2.5,
      hit_ratio: 0.9,
    });

    render(<TodayTab />);

    expect(await screen.findByText('90%')).toBeInTheDocument();
  });
});
