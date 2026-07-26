import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { DailyBucket, DailyModelBucket } from '../lib/types';

const TRENDS: DailyBucket[] = [
  { date: '2026-07-24', input_tokens: 8_000_000, output_tokens: 4_400_000, cost_usd: 4.82 },
  { date: '2026-07-25', input_tokens: 2_000_000, output_tokens: 1_000_000, cost_usd: 1.1 },
];

const BREAKDOWN: DailyModelBucket[] = [
  {
    date: '2026-07-24',
    models: [
      { model: 'claude-sonnet-4-6', input_tokens: 6_000_000, output_tokens: 2_100_000, cache_read_tokens: 6_200_000, cache_creation_tokens: 300_000, cost_usd: 2.94 },
      { model: 'claude-opus-4-7', input_tokens: 1_700_000, output_tokens: 1_700_000, cache_read_tokens: 2_100_000, cache_creation_tokens: 100_000, cost_usd: 1.61 },
      { model: 'claude-haiku-4-5', input_tokens: 300_000, output_tokens: 600_000, cache_read_tokens: 50_000, cache_creation_tokens: 10_000, cost_usd: 0.27 },
    ],
  },
  {
    date: '2026-07-25',
    models: [
      { model: 'claude-sonnet-4-6', input_tokens: 2_000_000, output_tokens: 1_000_000, cache_read_tokens: 500_000, cache_creation_tokens: 20_000, cost_usd: 1.1 },
    ],
  },
];

const ipcMock = vi.hoisted(() => ({
  getDailyTrends: vi.fn(),
  getDailyModelBreakdown: vi.fn(),
}));

vi.mock('../lib/ipc', () => ({ ipc: ipcMock }));

vi.mock('../lib/store', async () => {
  const actual = await vi.importActual<typeof import('../lib/store')>('../lib/store');
  const state = { sessionDataVersion: 0 };
  const useAppStore: any = (sel: any) => sel(state);
  useAppStore.getState = () => state;
  return { ...actual, useAppStore };
});

import { TrendsTab } from './TrendsTab';

describe('TrendsTab — day breakdown panel', () => {
  beforeEach(() => {
    ipcMock.getDailyTrends.mockClear();
    ipcMock.getDailyModelBreakdown.mockClear();
    ipcMock.getDailyTrends.mockResolvedValue(TRENDS);
    ipcMock.getDailyModelBreakdown.mockResolvedValue(BREAKDOWN);
  });

  it('reveals the per-model breakdown for a day on click, sorted by cost descending', async () => {
    render(<TrendsTab />);
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
    render(<TrendsTab />);
    const bar = await screen.findByRole('button', { name: /Jul 24/i });
    fireEvent.click(bar);
    await screen.findByTestId('day-breakdown-panel');

    fireEvent.click(bar);
    await waitFor(() => expect(screen.queryByTestId('day-breakdown-panel')).not.toBeInTheDocument());
  });

  it('switches the panel to a different day when a different bar is clicked', async () => {
    render(<TrendsTab />);
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
    render(<TrendsTab />);
    const bar = await screen.findByRole('button', { name: /Jul 24/i });
    fireEvent.click(bar);
    await screen.findByTestId('day-breakdown-panel');

    fireEvent.click(screen.getByText('7d'));
    await waitFor(() => expect(screen.queryByTestId('day-breakdown-panel')).not.toBeInTheDocument());
  });
});
