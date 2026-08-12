import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { SessionSummary } from '../../lib/generated/bindings';

const ipcMock = vi.hoisted(() => ({
  listResumableSessions: vi.fn(),
  listProviders: vi.fn().mockResolvedValue([]),
  listAvailableTerminals: vi.fn().mockResolvedValue(['ghostty']),
  launchProviderSession: vi.fn().mockResolvedValue('/tmp/a.sh'),
  getSettings: vi.fn().mockResolvedValue({ terminal: null }),
  vscodeTabAvailable: vi.fn().mockResolvedValue(true),
}));
vi.mock('../../lib/ipc', () => ({ ipc: ipcMock }));

import { SessionsBrowserTab } from '../SessionsBrowserTab';

function s(over: Partial<SessionSummary>): SessionSummary {
  return {
    session_id: 'id-1',
    cwd: '/w/alpha',
    project_name: 'alpha',
    git_branch: 'main',
    title: 'Alpha work',
    recap: null,
    asked: 'do alpha',
    left_off: null,
    touched_files: [],
    touched_overflow: 0,
    model: 'claude-opus-5',
    peak_context_tokens: 120_000,
    turns: 3,
    started_at: '2026-07-29T10:00:00Z',
    ended_at: '2026-07-29T11:00:00Z',
    total_tokens: 12_345,
    total_cost_usd: 1.23,
    permission_mode: null,
    cwd_exists: true,
    ...over,
  };
}

describe('SessionsBrowserTab', () => {
  beforeEach(() => vi.clearAllMocks());

  it('groups sessions under their project', async () => {
    ipcMock.listResumableSessions.mockResolvedValue([
      s({ session_id: 'a', project_name: 'alpha', title: 'Alpha work' }),
      s({ session_id: 'b', project_name: 'beta', cwd: '/w/beta', title: 'Beta work' }),
    ]);
    render(<SessionsBrowserTab />);
    await waitFor(() => expect(screen.getByRole('heading', { name: 'alpha' })).toBeTruthy());
    expect(screen.getByRole('heading', { name: 'beta' })).toBeTruthy();

    // Groups collapse by default — expand one to confirm its row renders.
    fireEvent.click(screen.getByRole('heading', { name: 'alpha' }));
    await waitFor(() => expect(screen.getByText('Alpha work')).toBeTruthy());
  });

  it('filters on search and flattens the grouping', async () => {
    ipcMock.listResumableSessions.mockResolvedValue([
      s({ session_id: 'a', project_name: 'alpha', title: 'Alpha work' }),
      s({ session_id: 'b', project_name: 'beta', cwd: '/w/beta', title: 'Beta work' }),
    ]);
    render(<SessionsBrowserTab />);
    await waitFor(() => expect(screen.getByRole('heading', { name: 'alpha' })).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/search/i), { target: { value: 'beta' } });
    await waitFor(() => expect(screen.queryByText('Alpha work')).toBeNull());
    expect(screen.getByText('Beta work')).toBeTruthy();
    expect(screen.queryByRole('heading', { name: 'beta' })).toBeNull();
  });

  it('searches left_off and touched files, not just the title', async () => {
    ipcMock.listResumableSessions.mockResolvedValue([
      s({ session_id: 'a', title: 'Opaque title', left_off: 'the migration broke' }),
      s({
        session_id: 'b',
        title: 'Other',
        touched_files: ['walker.rs'],
        cwd: '/w/beta',
        project_name: 'beta',
      }),
    ]);
    render(<SessionsBrowserTab />);
    await waitFor(() => expect(screen.getByRole('heading', { name: 'alpha' })).toBeTruthy());

    fireEvent.change(screen.getByLabelText(/search/i), { target: { value: 'migration' } });
    await waitFor(() => expect(screen.getByText('Opaque title')).toBeTruthy());

    fireEvent.change(screen.getByLabelText(/search/i), { target: { value: 'walker' } });
    await waitFor(() => expect(screen.getByText('Other')).toBeTruthy());
  });

  it('expands only one row at a time', async () => {
    ipcMock.listResumableSessions.mockResolvedValue([
      s({ session_id: 'a', title: 'First', asked: 'first ask' }),
      s({ session_id: 'b', title: 'Second', asked: 'second ask' }),
    ]);
    render(<SessionsBrowserTab />);
    await waitFor(() => expect(screen.getByRole('heading', { name: 'alpha' })).toBeTruthy());
    fireEvent.click(screen.getByRole('heading', { name: 'alpha' }));
    await waitFor(() => expect(screen.getByText('First')).toBeTruthy());
    fireEvent.click(screen.getByText('First'));
    await waitFor(() => expect(screen.getByText('first ask')).toBeTruthy());
    fireEvent.click(screen.getByText('Second'));
    await waitFor(() => expect(screen.getByText('second ask')).toBeTruthy());
    expect(screen.queryByText('first ask')).toBeNull();
  });

  it('distinguishes empty-corpus from no-match', async () => {
    ipcMock.listResumableSessions.mockResolvedValue([]);
    const { rerender } = render(<SessionsBrowserTab />);
    await waitFor(() => expect(screen.getByText(/no sessions yet/i)).toBeTruthy());

    ipcMock.listResumableSessions.mockResolvedValue([s({})]);
    rerender(<SessionsBrowserTab key="2" />);
    await waitFor(() => expect(screen.getByRole('heading', { name: 'alpha' })).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/search/i), { target: { value: 'zzzz' } });
    await waitFor(() => expect(screen.getByText(/no sessions match/i)).toBeTruthy());
  });
});
