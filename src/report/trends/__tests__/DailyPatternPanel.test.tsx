import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { AccountListEntry, HourCell, HourTotal } from '../../../lib/generated/bindings';
import type { DailyPatternReport, WarmupPlan } from '../../../lib/types';

const ipcMock = vi.hoisted(() => ({
  getDailyPattern: vi.fn(),
  getTodayPattern: vi.fn(),
  setAccountSchedule: vi.fn(),
}));

vi.mock('../../../lib/ipc', () => ({ ipc: ipcMock }));

const ACCOUNTS: AccountListEntry[] = vi.hoisted(() => [
  {
    slot: 1,
    email: 'active@x.com',
    account_uuid: 'uuid-active',
    org_name: null,
    org_uuid: null,
    subscription_type: 'pro',
    source: 'OAuth',
    is_active: true,
    cached_usage: null,
    last_error: null,
  },
  {
    slot: 2,
    email: 'other@x.com',
    account_uuid: 'uuid-other',
    org_name: null,
    org_uuid: null,
    subscription_type: 'pro',
    source: 'OAuth',
    is_active: false,
    cached_usage: null,
    last_error: null,
  },
]);

vi.mock('../../../lib/store', async () => {
  const actual = await vi.importActual<typeof import('../../../lib/store')>('../../../lib/store');
  const state = { sessionDataVersion: 0, accounts: ACCOUNTS };
  const useAppStore: any = (sel: any) => sel(state);
  useAppStore.getState = () => state;
  return { ...actual, useAppStore };
});

import { DailyPatternPanel } from '../DailyPatternPanel';

function cells(fill: (weekday: number, hour: number) => number): HourCell[] {
  const out: HourCell[] = [];
  for (let weekday = 0; weekday < 7; weekday++) {
    for (let hour = 0; hour < 24; hour++) {
      const tokens = fill(weekday, hour);
      out.push({ weekday, hour, tokens, cost_usd: 0, request_count: tokens > 0 ? 1 : 0 });
    }
  }
  return out;
}

function hourlyTotals(
  fill: (hour: number) => number,
  costFill: (hour: number) => number = () => 0,
): HourTotal[] {
  return Array.from({ length: 24 }, (_, hour) => ({
    hour,
    tokens: fill(hour),
    cost_usd: costFill(hour),
    request_count: fill(hour) > 0 ? 1 : 0,
  }));
}

const PLAN: WarmupPlan = {
  anchor: { hour: 8, minute: 0 },
  recommended_peak_share: 0.44,
  baseline_peak_share: 0.62,
  windows: [
    { start: { hour: 8, minute: 0 }, end: { hour: 13, minute: 0 }, share: 0.44 },
    { start: { hour: 13, minute: 0 }, end: { hour: 18, minute: 0 }, share: 0.3 },
    { start: { hour: 18, minute: 0 }, end: { hour: 23, minute: 0 }, share: 0.1 },
    { start: { hour: 23, minute: 0 }, end: { hour: 4, minute: 0 }, share: 0.16 },
  ],
};

function report(overrides: Partial<DailyPatternReport>): DailyPatternReport {
  return {
    cells: cells((_, hour) => (hour === 9 ? 1000 : 0)),
    hourly_totals: hourlyTotals(
      (hour) => (hour === 9 ? 1000 * 7 : 0),
      (hour) => (hour === 9 ? 2.5 : 0),
    ),
    active_days: 10,
    lookback_days: 30,
    plan: PLAN,
    ...overrides,
  };
}

describe('DailyPatternPanel — loading, error, empty', () => {
  beforeEach(() => {
    ipcMock.getDailyPattern.mockReset();
    ipcMock.getTodayPattern.mockReset();
    ipcMock.setAccountSchedule.mockReset().mockResolvedValue(undefined);
  });

  it('shows a loading state before data resolves', () => {
    ipcMock.getDailyPattern.mockReturnValue(new Promise(() => {}));
    render(<DailyPatternPanel />);
    expect(screen.getByText('Loading…')).toBeInTheDocument();
  });

  it('shows a retryable error state on failure', async () => {
    ipcMock.getDailyPattern.mockRejectedValue(new Error('db locked'));
    render(<DailyPatternPanel />);
    expect(await screen.findByText("Couldn't load daily pattern")).toBeInTheDocument();
    expect(screen.getByText('db locked')).toBeInTheDocument();

    ipcMock.getDailyPattern.mockResolvedValue(report({}));
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await screen.findByText(/active days/);
  });

  it('shows an empty state when there is no activity at all', async () => {
    ipcMock.getDailyPattern.mockResolvedValue(
      report({ cells: cells(() => 0), hourly_totals: hourlyTotals(() => 0), active_days: 0, plan: null }),
    );
    render(<DailyPatternPanel />);
    expect(await screen.findByText('No pattern data yet')).toBeInTheDocument();
  });
});

describe('DailyPatternPanel — grid and plan', () => {
  beforeEach(() => {
    ipcMock.getDailyPattern.mockReset();
    ipcMock.getTodayPattern.mockReset();
    ipcMock.setAccountSchedule.mockReset().mockResolvedValue(undefined);
  });

  it('renders all 168 hour × weekday cells', async () => {
    ipcMock.getDailyPattern.mockResolvedValue(report({}));
    render(<DailyPatternPanel />);
    await screen.findByText(/10 active days of 30/);
    expect(screen.getAllByTestId(/^pattern-cell-/)).toHaveLength(168);
  });

  it('shows the recommended anchor and both peak shares when a plan is present', async () => {
    ipcMock.getDailyPattern.mockResolvedValue(report({}));
    render(<DailyPatternPanel />);
    expect(await screen.findByText(/Anchor at 08:00/)).toBeInTheDocument();
    expect(screen.getByText(/62% to 44%/)).toBeInTheDocument();
  });

  it('still renders the grid but omits the plan card below the active-day floor', async () => {
    ipcMock.getDailyPattern.mockResolvedValue(report({ active_days: 3, plan: null }));
    render(<DailyPatternPanel />);
    await screen.findByText(/3 active days of 30/);
    expect(screen.getAllByTestId(/^pattern-cell-/)).toHaveLength(168);
    expect(screen.queryByText(/Anchor at/)).not.toBeInTheDocument();
    expect(screen.getByText(/Need 5 active days to recommend an anchor — 3 so far\./)).toBeInTheDocument();
  });

  it('cycles the lookback window and refetches', async () => {
    ipcMock.getDailyPattern.mockResolvedValue(report({}));
    render(<DailyPatternPanel />);
    await screen.findByText(/10 active days of 30/);
    expect(ipcMock.getDailyPattern).toHaveBeenCalledWith(30);

    fireEvent.click(screen.getByText('90d'));
    await waitFor(() => expect(ipcMock.getDailyPattern).toHaveBeenCalledWith(90));
  });

  it('renders both a tokens-by-hour and a cost-by-hour graph', async () => {
    ipcMock.getDailyPattern.mockResolvedValue(report({}));
    render(<DailyPatternPanel />);
    await screen.findByText(/10 active days of 30/);

    expect(screen.getByText('Tokens by hour of day')).toBeInTheDocument();
    expect(screen.getByText('Cost by hour of day')).toBeInTheDocument();
    expect(screen.getAllByTestId(/^hour-bar-tokens-/)).toHaveLength(24);
    expect(screen.getAllByTestId(/^hour-bar-cost-/)).toHaveLength(24);
    expect(screen.getByTestId('hour-bar-tokens-9')).toHaveAttribute(
      'title',
      expect.stringContaining('7.0K tokens'),
    );
    expect(screen.getByTestId('hour-bar-cost-9')).toHaveAttribute('title', expect.stringContaining('$2.50'));
  });
});

describe('DailyPatternPanel — Today lookback', () => {
  beforeEach(() => {
    ipcMock.getDailyPattern.mockReset();
    ipcMock.getTodayPattern.mockReset();
    ipcMock.setAccountSchedule.mockReset().mockResolvedValue(undefined);
  });

  it('switches to getTodayPattern when Today is selected, and back to getDailyPattern otherwise', async () => {
    ipcMock.getDailyPattern.mockResolvedValue(report({}));
    ipcMock.getTodayPattern.mockResolvedValue(
      report({ active_days: 1, lookback_days: 1, plan: null }),
    );
    render(<DailyPatternPanel />);
    await screen.findByText(/10 active days of 30/);
    expect(ipcMock.getTodayPattern).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText('Today'));
    await waitFor(() => expect(ipcMock.getTodayPattern).toHaveBeenCalled());
    expect(await screen.findByText('Active today')).toBeInTheDocument();

    fireEvent.click(screen.getByText('7d'));
    await waitFor(() => expect(ipcMock.getDailyPattern).toHaveBeenCalledWith(7));
  });

  it('shows "no activity yet today" when today has no events', async () => {
    ipcMock.getDailyPattern.mockResolvedValue(report({}));
    ipcMock.getTodayPattern.mockResolvedValue(
      report({ active_days: 0, lookback_days: 1, plan: null }),
    );
    render(<DailyPatternPanel />);
    await screen.findByText(/10 active days of 30/);

    fireEvent.click(screen.getByText('Today'));
    expect(await screen.findByText('No activity yet today')).toBeInTheDocument();
  });
});

describe('DailyPatternPanel — applying a schedule', () => {
  beforeEach(() => {
    ipcMock.getDailyPattern.mockReset().mockResolvedValue(report({}));
    ipcMock.setAccountSchedule.mockReset().mockResolvedValue(undefined);
  });

  it('defaults the account picker to the active account and applies the recommended anchor to it', async () => {
    render(<DailyPatternPanel />);
    await screen.findByText(/Anchor at 08:00/);

    fireEvent.click(screen.getByRole('button', { name: 'Apply' }));

    await waitFor(() =>
      expect(ipcMock.setAccountSchedule).toHaveBeenCalledWith('uuid-active', {
        type: 'Every5h',
        anchor: { hour: 8, minute: 0 },
      }),
    );
    expect(await screen.findByText('Applied')).toBeInTheDocument();
  });

  it('applies to whichever account is picked instead of the default', async () => {
    render(<DailyPatternPanel />);
    await screen.findByText(/Anchor at 08:00/);

    fireEvent.change(screen.getByLabelText('Account to apply the schedule to'), {
      target: { value: 'uuid-other' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Apply' }));

    await waitFor(() =>
      expect(ipcMock.setAccountSchedule).toHaveBeenCalledWith('uuid-other', {
        type: 'Every5h',
        anchor: { hour: 8, minute: 0 },
      }),
    );
  });

  it('shows a failure chip when applying the schedule rejects', async () => {
    ipcMock.setAccountSchedule.mockRejectedValue(new Error('offline'));
    render(<DailyPatternPanel />);
    await screen.findByText(/Anchor at 08:00/);

    fireEvent.click(screen.getByRole('button', { name: 'Apply' }));
    expect(await screen.findByText('Failed')).toBeInTheDocument();
  });
});
