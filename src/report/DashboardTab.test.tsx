import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { SessionEvent, DailyPatternReport } from '../lib/types';

const ipcMock = vi.hoisted(() => ({
  getSessionHistory: vi.fn(),
  getTodayPattern: vi.fn(),
  getTodayRepoBreakdown: vi.fn(),
  getTodayCacheStats: vi.fn(),
  getYesterdayPattern: vi.fn(),
  getYesterdayRepoBreakdown: vi.fn(),
  getYesterdayCacheStats: vi.fn(),
  getWeekPattern: vi.fn(),
  getWeekRepoBreakdown: vi.fn(),
  getWeekCacheStats: vi.fn(),
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

import { DashboardTab } from './DashboardTab';

const EMPTY_PATTERN: DailyPatternReport = {
  cells: [],
  hourly_totals: Array.from({ length: 24 }, (_, hour) => ({ hour, tokens: 0, cost_usd: 0, request_count: 0 })),
  active_days: 0,
  lookback_days: 1,
  plan: null,
};

const EMPTY_CACHE = {
  total_cache_read_tokens: 0,
  total_cache_creation_tokens: 0,
  estimated_savings_usd: 0,
  hit_ratio: 0,
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

describe('DashboardTab', () => {
  beforeEach(() => {
    ipcMock.getSessionHistory.mockReset();
    ipcMock.getTodayPattern.mockReset().mockResolvedValue(EMPTY_PATTERN);
    ipcMock.getTodayRepoBreakdown.mockReset().mockResolvedValue([]);
    ipcMock.getTodayCacheStats.mockReset().mockResolvedValue(EMPTY_CACHE);
    ipcMock.getYesterdayPattern.mockReset().mockResolvedValue(EMPTY_PATTERN);
    ipcMock.getYesterdayRepoBreakdown.mockReset().mockResolvedValue([]);
    ipcMock.getYesterdayCacheStats.mockReset().mockResolvedValue(EMPTY_CACHE);
    ipcMock.getWeekPattern.mockReset().mockResolvedValue(EMPTY_PATTERN);
    ipcMock.getWeekRepoBreakdown.mockReset().mockResolvedValue([]);
    ipcMock.getWeekCacheStats.mockReset().mockResolvedValue(EMPTY_CACHE);
  });

  it('defaults to the Today period and switches to Yesterday/This Week on demand', async () => {
    const now = new Date();
    const todayIso = now.toISOString();
    const yesterdayIso = new Date(now.getTime() - 26 * 60 * 60 * 1000).toISOString();
    ipcMock.getSessionHistory.mockResolvedValue([
      ev({ ts: todayIso, source_file: 'proj/today-a.jsonl', cost_usd: 0.5 }),
      ev({ ts: yesterdayIso, source_file: 'proj/yesterday.jsonl', cost_usd: 1.0 }),
    ]);
    render(<DashboardTab />);

    expect(await screen.findByTestId('today-cost')).toBeInTheDocument();
    expect(screen.queryByTestId('yesterday-cost')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('period-switch-yesterday'));

    expect(screen.queryByTestId('today-cost')).not.toBeInTheDocument();
    expect(screen.getByTestId('yesterday-cost')).toHaveTextContent('$1.00');
  });

  it('shows an inline empty message when the selected period has no activity', async () => {
    const todayIso = new Date().toISOString();
    ipcMock.getSessionHistory.mockResolvedValue([
      ev({ ts: todayIso, source_file: 'proj/today-a.jsonl', cost_usd: 0.5 }),
    ]);
    render(<DashboardTab />);
    await screen.findByTestId('today-cost');

    fireEvent.click(screen.getByTestId('period-switch-yesterday'));

    expect(screen.getByText('No activity yesterday')).toBeInTheDocument();
  });

  it('sums each period\'s own events into its headline row', async () => {
    const now = new Date();
    const todayIso = now.toISOString();
    const yesterdayIso = new Date(now.getTime() - 26 * 60 * 60 * 1000).toISOString();
    const lastWeekButOneIso = new Date(now.getTime() - 3 * 24 * 60 * 60 * 1000).toISOString();
    ipcMock.getSessionHistory.mockResolvedValue([
      ev({ ts: todayIso, source_file: 'proj/today-a.jsonl', cost_usd: 0.5, input_tokens: 100, output_tokens: 50 }),
      ev({ ts: yesterdayIso, source_file: 'proj/yesterday.jsonl', cost_usd: 1.0, input_tokens: 200, output_tokens: 100 }),
      ev({ ts: lastWeekButOneIso, source_file: 'proj/3-days-ago.jsonl', cost_usd: 2.0, input_tokens: 300, output_tokens: 100 }),
    ]);

    render(<DashboardTab />);

    expect(await screen.findByTestId('today-cost')).toHaveTextContent('$0.50');

    fireEvent.click(screen.getByTestId('period-switch-yesterday'));
    expect(screen.getByTestId('yesterday-cost')).toHaveTextContent('$1.00');

    fireEvent.click(screen.getByTestId('period-switch-week'));
    // This Week includes today + yesterday + the 3-days-ago event.
    expect(screen.getByTestId('week-cost')).toHaveTextContent('$3.50');
  });

  it('collapses the sessions list by default, expands on toggle, and always renders it for print', async () => {
    const todayIso = new Date().toISOString();
    ipcMock.getSessionHistory.mockResolvedValue([
      ev({ ts: todayIso, source_file: 'proj/a.jsonl', cost_usd: 0.5 }),
    ]);

    render(<DashboardTab />);
    await screen.findByTestId('today-cost');

    const toggle = screen.getByTestId('today-sessions-toggle');
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    // Always mounted (not conditionally unmounted) so a print stylesheet can
    // force it visible regardless of the on-screen collapsed state.
    expect(screen.getByTestId('today-session-row')).toBeInTheDocument();
    expect(screen.getByTestId('today-sessions-body')).toHaveClass('hidden');

    fireEvent.click(toggle);

    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByTestId('today-sessions-body')).not.toHaveClass('hidden');
  });

  it('exports the visible period to PDF via the native print dialog', async () => {
    const todayIso = new Date().toISOString();
    ipcMock.getSessionHistory.mockResolvedValue([
      ev({ ts: todayIso, source_file: 'proj/a.jsonl', cost_usd: 0.5 }),
    ]);
    const printSpy = vi.spyOn(window, 'print').mockImplementation(() => {});

    render(<DashboardTab />);
    await screen.findByTestId('today-cost');

    fireEvent.click(screen.getByLabelText('Export PDF'));

    expect(printSpy).toHaveBeenCalledTimes(1);
    printSpy.mockRestore();
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

    render(<DashboardTab />);

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

    render(<DashboardTab />);

    expect(await screen.findByText('90%')).toBeInTheDocument();
  });
});
