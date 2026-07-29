import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';

const ipcMock = vi.hoisted(() => ({
  listProviders: vi.fn(),
  listAvailableTerminals: vi.fn().mockResolvedValue(['ghostty']),
  listProviderPresets: vi.fn().mockResolvedValue([]),
  launchProviderSession: vi.fn().mockResolvedValue('/tmp/launch/a.sh'),
  getProviderLaunchCommand: vi.fn().mockResolvedValue('open -na Ghostty.app'),
  getDefaultProvider: vi.fn().mockResolvedValue(null),
  upsertProvider: vi.fn().mockResolvedValue(undefined),
  deleteProvider: vi.fn().mockResolvedValue(undefined),
  setDefaultProvider: vi.fn().mockResolvedValue({ status: 'applied' }),
  clearDefaultProvider: vi.fn().mockResolvedValue([]), // returns drift-skipped keys
  getSettings: vi.fn().mockResolvedValue({ terminal: null }),
}));
vi.mock('../../lib/ipc', () => ({ ipc: ipcMock }));

const dialogMock = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => dialogMock);

import { ProvidersTab } from '../ProvidersTab';

const official = {
  id: 'official',
  name: 'Anthropic (official)',
  kind: 'official',
  base_url: null,
  auth_token: null,
  env: {},
  extra_args: [],
  preset_id: null,
  sort_index: 0,
};
const glm = {
  id: 'p1',
  name: 'GLM',
  kind: 'third_party',
  base_url: 'https://api.z.ai/api/anthropic',
  auth_token: 'tok',
  env: { ANTHROPIC_MODEL: 'glm-5.2' },
  extra_args: [],
  preset_id: 'glm',
  sort_index: 1,
};

describe('ProvidersTab', () => {
  beforeEach(() => vi.clearAllMocks());

  it('lists providers returned by the backend', async () => {
    ipcMock.listProviders.mockResolvedValue([official, glm]);
    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText('GLM')).toBeTruthy());
    expect(screen.getByText('Anthropic (official)')).toBeTruthy();
  });

  it('shows an empty state when only the official provider exists', async () => {
    ipcMock.listProviders.mockResolvedValue([official]);
    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText(/add a provider/i)).toBeTruthy());
  });

  it('surfaces a backend error instead of rendering an empty list', async () => {
    ipcMock.listProviders.mockRejectedValue(new Error('db is locked'));
    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText(/db is locked/)).toBeTruthy());
  });

  it('launches with the folder chosen in the picker', async () => {
    ipcMock.listProviders.mockResolvedValue([official, glm]);
    dialogMock.open.mockResolvedValue('/Users/me/work');
    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText('GLM')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /launch glm/i }));
    await waitFor(() =>
      expect(ipcMock.launchProviderSession).toHaveBeenCalledWith(
        'p1',
        '/Users/me/work',
        'ghostty',
        null,
      ),
    );
  });

  it('does not launch when the folder picker is cancelled', async () => {
    ipcMock.listProviders.mockResolvedValue([official, glm]);
    dialogMock.open.mockResolvedValue(null);
    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText('GLM')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /launch glm/i }));
    await waitFor(() => expect(ipcMock.launchProviderSession).not.toHaveBeenCalled());
  });

  it('asks for confirmation before overwriting unmanaged settings keys', async () => {
    ipcMock.listProviders.mockResolvedValue([official, glm]);
    ipcMock.setDefaultProvider = vi
      .fn()
      .mockResolvedValueOnce({
        status: 'needs_confirmation',
        unmanaged_keys: ['ANTHROPIC_BASE_URL'],
      })
      .mockResolvedValueOnce({ status: 'applied' });
    ipcMock.clearDefaultProvider = vi.fn().mockResolvedValue([]);
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);

    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText('GLM')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /set glm as default/i }));

    await waitFor(() => expect(confirmSpy).toHaveBeenCalled());
    await waitFor(() => expect(ipcMock.setDefaultProvider).toHaveBeenCalledWith('p1', true));
    confirmSpy.mockRestore();
  });

  it('does not force the write when the user declines confirmation', async () => {
    ipcMock.listProviders.mockResolvedValue([official, glm]);
    ipcMock.setDefaultProvider = vi.fn().mockResolvedValue({
      status: 'needs_confirmation',
      unmanaged_keys: ['ANTHROPIC_BASE_URL'],
    });
    ipcMock.clearDefaultProvider = vi.fn().mockResolvedValue([]);
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);

    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText('GLM')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /set glm as default/i }));

    await waitFor(() => expect(confirmSpy).toHaveBeenCalled());
    expect(ipcMock.setDefaultProvider).toHaveBeenCalledTimes(1);
    confirmSpy.mockRestore();
  });
});
