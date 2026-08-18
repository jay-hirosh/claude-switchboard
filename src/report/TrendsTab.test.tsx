import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import type { DailyBucket, DailyModelBucket } from '../lib/types';

// This suite exercises UsageTrends directly (the former TrendsTab body,
// moved to report/trends/ when Trends grew a "Daily pattern" sub-tab —
// see DailyPatternPanel.test.tsx). TrendsTab itself is now just the
// sub-tab shell around UsageTrends and DailyPatternPanel.

const TRENDS: DailyBucket[] = [
  { date: '2026-07-24', input_tokens: 8_000_000, output_tokens: 4_400_000, cost_usd: 4.82, request_count: 12 },
  { date: '2026-07-25', input_tokens: 2_000_000, output_tokens: 1_000_000, cost_usd: 1.1, request_count: 5 },
];

const BREAKDOWN: DailyModelBucket[] = [
  {
    date: '2026-07-24',
    models: [
      { model: 'claude-sonnet-4-6', input_tokens: 6_000_000, output_tokens: 2_100_000, cache_read_tokens: 6_200_000, cache_creation_tokens: 300_000, cost_usd: 2.94, by_account: [] },
      { model: 'claude-opus-4-7', input_tokens: 1_700_000, output_tokens: 1_700_000, cache_read_tokens: 2_100_000, cache_creation_tokens: 100_000, cost_usd: 1.61, by_account: [] },
      { model: 'claude-haiku-4-5', input_tokens: 300_000, output_tokens: 600_000, cache_read_tokens: 50_000, cache_creation_tokens: 10_000, cost_usd: 0.27, by_account: [] },
    ],
  },
  {
    date: '2026-07-25',
    models: [
      { model: 'claude-sonnet-4-6', input_tokens: 2_000_000, output_tokens: 1_000_000, cache_read_tokens: 500_000, cache_creation_tokens: 20_000, cost_usd: 1.1, by_account: [] },
    ],
  },
];

const ipcMock = vi.hoisted(() => ({
  getDailyTrends: vi.fn(),
  getDailyModelBreakdown: vi.fn(),
  getDailyAccountBreakdown: vi.fn(),
  exportTrendsCsv: vi.fn(),
}));

vi.mock('../lib/ipc', () => ({ ipc: ipcMock }));

const dialogMock = vi.hoisted(() => ({ save: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => dialogMock);

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

import { UsageTrends } from './trends/UsageTrends';

describe('UsageTrends — day breakdown panel', () => {
  beforeEach(() => {
    ipcMock.getDailyTrends.mockClear();
    ipcMock.getDailyModelBreakdown.mockClear();
    ipcMock.getDailyTrends.mockResolvedValue(TRENDS);
    ipcMock.getDailyModelBreakdown.mockResolvedValue(BREAKDOWN);
  });

  it('reveals the per-model breakdown for a day on click, sorted by cost descending', async () => {
    render(<UsageTrends />);
    const bar = await screen.findByRole('button', { name: /Jul 24/i });
    fireEvent.click(bar);

    const panel = await screen.findByTestId('day-breakdown-panel');
    const badges = within(panel).getAllByText(/^(sonnet|opus|haiku) \d/);
    expect(badges.map((b) => b.textContent)).toEqual(['sonnet 4-6', 'opus 4-7', 'haiku 4-5']);

    const sonnetFill = within(panel).getByTestId('model-fill-claude-sonnet-4-6');
    // day total = 8_000_000 + 4_400_000 = 12_400_000; sonnet total = 6_000_000 + 2_100_000 = 8_100_000
    // 8_100_000 / 12_400_000 * 100 = 65.32258064516129 — NOT the 30-day aggregate ModelsTab uses
    expect(parseFloat(sonnetFill.style.width)).toBeCloseTo(65.32, 1);
  });

  it('collapses the panel when the same bar is clicked again', async () => {
    render(<UsageTrends />);
    const bar = await screen.findByRole('button', { name: /Jul 24/i });
    fireEvent.click(bar);
    await screen.findByTestId('day-breakdown-panel');

    fireEvent.click(bar);
    await waitFor(() => expect(screen.queryByTestId('day-breakdown-panel')).not.toBeInTheDocument());
  });

  it('switches the panel to a different day when a different bar is clicked', async () => {
    render(<UsageTrends />);
    const bar24 = await screen.findByRole('button', { name: /Jul 24/i });
    fireEvent.click(bar24);
    let panel = await screen.findByTestId('day-breakdown-panel');
    expect(within(panel).getByText('opus 4-7')).toBeInTheDocument();

    const bar25 = screen.getByRole('button', { name: /Jul 25/i });
    fireEvent.click(bar25);
    await waitFor(() => {
      panel = screen.getByTestId('day-breakdown-panel');
      expect(within(panel).queryByText('opus 4-7')).not.toBeInTheDocument();
    });
    expect(within(panel).getByText('sonnet 4-6')).toBeInTheDocument();

    expect(ipcMock.getDailyTrends).toHaveBeenCalledTimes(1);
    expect(ipcMock.getDailyModelBreakdown).toHaveBeenCalledTimes(1);
  });

  it('clears the selection when the range toggle changes', async () => {
    render(<UsageTrends />);
    const bar = await screen.findByRole('button', { name: /Jul 24/i });
    fireEvent.click(bar);
    await screen.findByTestId('day-breakdown-panel');

    fireEvent.click(screen.getByText('7d'));
    await waitFor(() => expect(screen.queryByTestId('day-breakdown-panel')).not.toBeInTheDocument());
  });
});

describe('UsageTrends — period comparison strip', () => {
  // "now" = 2026-08-14 (Fri). Current week: Aug 8–14. Prior week: Aug 1–7.
  // Current month: Aug 1–14. Prior month: all of July.
  const PERIOD_TRENDS: DailyBucket[] = [
    { date: '2026-08-01', input_tokens: 1000, output_tokens: 0, cost_usd: 1, request_count: 1 },
    { date: '2026-08-14', input_tokens: 1000, output_tokens: 0, cost_usd: 2, request_count: 1 },
    { date: '2026-07-15', input_tokens: 1000, output_tokens: 0, cost_usd: 5, request_count: 1 },
  ];

  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date(2026, 7, 14));
    ipcMock.getDailyTrends.mockClear();
    ipcMock.getDailyModelBreakdown.mockClear();
    ipcMock.getDailyTrends.mockResolvedValue(PERIOD_TRENDS);
    ipcMock.getDailyModelBreakdown.mockResolvedValue([]);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows week-over-week deltas by default', async () => {
    render(<UsageTrends />);
    // Cost: current week (Aug 8–14) = 2, prior week (Aug 1–7) = 1 → doubled.
    await screen.findByText('▲ +100%');
  });

  it('switches to month-over-month deltas when Month is clicked', async () => {
    render(<UsageTrends />);
    await screen.findByText('▲ +100%');

    fireEvent.click(screen.getByRole('button', { name: 'Month' }));

    // Cost: current month (Aug 1–14) = 1 + 2 = 3, prior month (all of July) = 5 → down 40%.
    await screen.findByText('▼ -40%');
  });

  it('shows "new" for every metric when there is no prior-period data at all', async () => {
    ipcMock.getDailyTrends.mockResolvedValue([
      { date: '2026-08-14', input_tokens: 1000, output_tokens: 0, cost_usd: 2, request_count: 1 },
    ]);

    render(<UsageTrends />);
    const newLabels = await screen.findAllByText('new');
    expect(newLabels).toHaveLength(3); // cost, tokens, requests all lack prior data
  });
});

describe('UsageTrends — monthly cost pacing', () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    // Aug 10, 2026 — 10th day of a 31-day month.
    vi.setSystemTime(new Date(2026, 7, 10));
    ipcMock.getDailyTrends.mockClear();
    ipcMock.getDailyModelBreakdown.mockClear();
    ipcMock.getDailyModelBreakdown.mockResolvedValue([]);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('shows spent-so-far and the whole-dollar projected total for the month', async () => {
    ipcMock.getDailyTrends.mockResolvedValue([
      { date: '2026-08-01', input_tokens: 0, output_tokens: 0, cost_usd: 20, request_count: 1 },
      { date: '2026-08-10', input_tokens: 0, output_tokens: 0, cost_usd: 30, request_count: 1 },
    ]);

    render(<UsageTrends />);
    // spent so far: 20 + 30 = 50. rate = 50/10 = 5/day; projected = 5 * 31 = 155.
    await screen.findByText('$50.00 this month');
    await screen.findByText('→ ~$155 this month');
  });
});

describe('UsageTrends — anomaly flagging', () => {
  beforeEach(() => {
    ipcMock.getDailyTrends.mockClear();
    ipcMock.getDailyModelBreakdown.mockClear();
    ipcMock.getDailyModelBreakdown.mockResolvedValue([]);
  });

  it('marks a day whose cost spikes well above its trailing baseline, with the ratio in its tooltip', async () => {
    const baseline = Array.from({ length: 7 }, (_, i) => ({
      date: `2026-07-2${i}`,
      input_tokens: 1000,
      output_tokens: 0,
      cost_usd: 1,
      request_count: 1,
    }));
    const spike = { date: '2026-07-27', input_tokens: 1000, output_tokens: 0, cost_usd: 4, request_count: 1 };
    ipcMock.getDailyTrends.mockResolvedValue([...baseline, spike]);

    render(<UsageTrends />);
    const dot = await screen.findByTestId('day-anomaly-2026-07-27');
    expect(dot).toBeInTheDocument();
    // No dot on a normal baseline day.
    expect(screen.queryByTestId('day-anomaly-2026-07-20')).not.toBeInTheDocument();
    expect(screen.getByText('4.0x your recent average')).toBeInTheDocument();
  });

  it('does not mark any day when nothing exceeds the baseline', async () => {
    const steady = Array.from({ length: 8 }, (_, i) => ({
      date: `2026-07-2${i}`,
      input_tokens: 1000,
      output_tokens: 0,
      cost_usd: 1,
      request_count: 1,
    }));
    ipcMock.getDailyTrends.mockResolvedValue(steady);

    render(<UsageTrends />);
    await screen.findByTestId(`day-bar-${steady[0].date}`);
    expect(screen.queryByText(/your recent average/)).not.toBeInTheDocument();
  });
});

describe('UsageTrends — CSV export', () => {
  beforeEach(() => {
    ipcMock.getDailyTrends.mockClear();
    ipcMock.getDailyModelBreakdown.mockClear();
    ipcMock.exportTrendsCsv.mockClear();
    dialogMock.save.mockClear();
    ipcMock.getDailyTrends.mockResolvedValue(TRENDS);
    ipcMock.getDailyModelBreakdown.mockResolvedValue(BREAKDOWN);
  });

  it('exports to the chosen path when the user picks one', async () => {
    dialogMock.save.mockResolvedValue('/Users/me/switchboard-trends.csv');
    render(<UsageTrends />);
    await screen.findByRole('button', { name: /Jul 24/i });

    fireEvent.click(screen.getByRole('button', { name: /export csv/i }));

    await waitFor(() =>
      expect(ipcMock.exportTrendsCsv).toHaveBeenCalledWith('/Users/me/switchboard-trends.csv', 62),
    );
  });

  it('does not export when the user cancels the save dialog', async () => {
    dialogMock.save.mockResolvedValue(null);
    render(<UsageTrends />);
    await screen.findByRole('button', { name: /Jul 24/i });

    fireEvent.click(screen.getByRole('button', { name: /export csv/i }));

    await waitFor(() => expect(dialogMock.save).toHaveBeenCalled());
    expect(ipcMock.exportTrendsCsv).not.toHaveBeenCalled();
  });
});

describe('UsageTrends — color by account', () => {
  beforeEach(() => {
    ipcMock.getDailyTrends.mockClear();
    ipcMock.getDailyModelBreakdown.mockClear();
    ipcMock.getDailyAccountBreakdown.mockClear();
  });

  it('colors day bars by account when the Account toggle is selected', async () => {
    ipcMock.getDailyTrends.mockResolvedValue([
      { date: '2026-08-01', input_tokens: 100, output_tokens: 20, cost_usd: 1.0, request_count: 3 },
    ]);
    ipcMock.getDailyModelBreakdown.mockResolvedValue([]);
    ipcMock.getDailyAccountBreakdown.mockResolvedValue([
      {
        date: '2026-08-01',
        accounts: [
          { account_uuid: 'acc1', input_tokens: 80, output_tokens: 15, cost_usd: 0.8 },
          { account_uuid: 'acc2', input_tokens: 20, output_tokens: 5, cost_usd: 0.2 },
        ],
      },
    ]);

    render(<UsageTrends />);
    await screen.findByTestId('day-bar-2026-08-01');

    fireEvent.click(screen.getByRole('button', { name: 'Account' }));

    expect(screen.getByTestId('day-bar-2026-08-01-acc1')).toBeInTheDocument();
    expect(screen.getByTestId('day-bar-2026-08-01-acc2')).toBeInTheDocument();
  });
});
