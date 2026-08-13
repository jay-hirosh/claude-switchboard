import { render, screen } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { ModelStats, CacheStats } from '../lib/types';

const CACHE: CacheStats = {
  total_cache_read_tokens: 8_300_000,
  total_cache_creation_tokens: 400_000,
  estimated_savings_usd: 12.5,
  hit_ratio: 0.72,
};

const ipcMock = vi.hoisted(() => ({
  getModelBreakdown: vi.fn(),
  getCacheStats: vi.fn(),
}));

vi.mock('../lib/ipc', () => ({ ipc: ipcMock }));

vi.mock('../lib/store', async () => {
  const actual = await vi.importActual<typeof import('../lib/store')>('../lib/store');
  const state = { sessionDataVersion: 0 };
  const useAppStore: any = (sel: any) => sel(state);
  useAppStore.getState = () => state;
  return { ...actual, useAppStore };
});

import { ModelsTab } from './ModelsTab';

describe('ModelsTab — cost-per-1K-tokens', () => {
  beforeEach(() => {
    ipcMock.getModelBreakdown.mockClear();
    ipcMock.getCacheStats.mockClear();
    ipcMock.getCacheStats.mockResolvedValue(CACHE);
  });

  it('shows a distinct per-1K rate for each model, independent of usage volume', async () => {
    const models: ModelStats[] = [
      // High volume, low per-token rate.
      { model: 'claude-sonnet-4-6', input_tokens: 8_000_000, output_tokens: 2_000_000, cache_read_tokens: 0, cache_creation_tokens: 0, cost_usd: 4.0 },
      // Low volume, high per-token rate — same total cost as sonnet above.
      { model: 'claude-opus-4-7', input_tokens: 8_000, output_tokens: 2_000, cache_read_tokens: 0, cache_creation_tokens: 0, cost_usd: 4.0 },
    ];
    ipcMock.getModelBreakdown.mockResolvedValue(models);

    render(<ModelsTab />);

    // sonnet: 4.0 / (10_000_000 / 1000) = $0.0004/1K -> floors to <$0.01/1K tokens
    await screen.findByText('<$0.01/1K tokens');
    // opus: 4.0 / (10_000 / 1000) = $0.40/1K tokens
    await screen.findByText('$0.40/1K tokens');
  });

  it('does not divide by zero for a model with no recorded tokens', async () => {
    const models: ModelStats[] = [
      { model: 'claude-haiku-4-5', input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0, cost_usd: 0 },
    ];
    ipcMock.getModelBreakdown.mockResolvedValue(models);

    render(<ModelsTab />);

    await screen.findByText('$0.00/1K tokens');
    expect(screen.queryByText(/NaN|Infinity/)).not.toBeInTheDocument();
  });
});
