import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { StatuslineInstallState, InstallStatuslineOutcome } from '../../lib/generated/bindings';

const ipcMock = vi.hoisted(() => ({
  getStatuslineInstallState: vi.fn(),
  installStatusline: vi.fn(),
  uninstallStatusline: vi.fn(),
}));
vi.mock('../../lib/ipc', () => ({ ipc: ipcMock }));

import { StatuslineSettings } from '../StatuslineSettings';

describe('StatuslineSettings', () => {
  beforeEach(() => {
    ipcMock.getStatuslineInstallState.mockReset();
    ipcMock.installStatusline.mockReset();
    ipcMock.uninstallStatusline.mockReset();
  });

  it('shows an Install button when not installed', async () => {
    ipcMock.getStatuslineInstallState.mockResolvedValue(null);
    render(<StatuslineSettings />);
    expect(await screen.findByRole('button', { name: /install/i })).toBeInTheDocument();
  });

  it('shows an Uninstall button when already installed', async () => {
    const state: StatuslineInstallState = {
      installed_command: '/usr/local/bin/switchboard statusline',
      installed_at: 1_700_000_000,
    };
    ipcMock.getStatuslineInstallState.mockResolvedValue(state);
    render(<StatuslineSettings />);
    expect(await screen.findByRole('button', { name: /uninstall/i })).toBeInTheDocument();
  });

  it('clicking Install applies directly when there is nothing to confirm', async () => {
    // Exactly 2 getStatuslineInstallState calls happen: the initial mount
    // fetch, then the reload() after handleInstall finishes. Queue exactly
    // those 2 values — a 3rd queued value would never be consumed and could
    // mask a queue-order mistake instead of catching one.
    ipcMock.getStatuslineInstallState
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce({
        installed_command: '/usr/local/bin/switchboard statusline',
        installed_at: 1_700_000_000,
      });
    const applied: InstallStatuslineOutcome = { status: 'applied' };
    ipcMock.installStatusline.mockResolvedValue(applied);

    render(<StatuslineSettings />);
    fireEvent.click(await screen.findByRole('button', { name: /install/i }));

    await waitFor(() => {
      expect(ipcMock.installStatusline).toHaveBeenCalledWith(false);
    });
    expect(await screen.findByRole('button', { name: /uninstall/i })).toBeInTheDocument();
  });

  it('clicking Install confirms before overwriting a foreign statusLine, and re-invokes with force on accept', async () => {
    // Same 2-call accounting as above: mount fetch, then post-install reload.
    ipcMock.getStatuslineInstallState
      .mockResolvedValueOnce(null)
      .mockResolvedValueOnce({
        installed_command: '/usr/local/bin/switchboard statusline',
        installed_at: 1_700_000_000,
      });
    const needsConfirmation: InstallStatuslineOutcome = {
      status: 'needs_confirmation',
      foreign_value: { type: 'command', command: 'bash x.sh' },
    };
    const applied: InstallStatuslineOutcome = { status: 'applied' };
    ipcMock.installStatusline
      .mockResolvedValueOnce(needsConfirmation)
      .mockResolvedValueOnce(applied);
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);

    render(<StatuslineSettings />);
    fireEvent.click(await screen.findByRole('button', { name: /install/i }));

    await waitFor(() => {
      expect(ipcMock.installStatusline).toHaveBeenNthCalledWith(2, true);
    });
    confirmSpy.mockRestore();
  });

  it('does not re-invoke when the user declines the confirmation', async () => {
    ipcMock.getStatuslineInstallState.mockResolvedValue(null);
    const needsConfirmation: InstallStatuslineOutcome = {
      status: 'needs_confirmation',
      foreign_value: { type: 'command', command: 'bash x.sh' },
    };
    ipcMock.installStatusline.mockResolvedValue(needsConfirmation);
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);

    render(<StatuslineSettings />);
    fireEvent.click(await screen.findByRole('button', { name: /install/i }));

    await waitFor(() => {
      expect(ipcMock.installStatusline).toHaveBeenCalledTimes(1);
    });
    confirmSpy.mockRestore();
  });

  it('clicking Uninstall applies directly and reverts to Install', async () => {
    // Same 2-call accounting as the install-success test: mount fetch (installed),
    // then the reload() after handleUninstall finishes (no longer installed).
    ipcMock.getStatuslineInstallState
      .mockResolvedValueOnce({
        installed_command: '/usr/local/bin/switchboard statusline',
        installed_at: 1_700_000_000,
      })
      .mockResolvedValueOnce(null);
    ipcMock.uninstallStatusline.mockResolvedValue(true);

    render(<StatuslineSettings />);
    fireEvent.click(await screen.findByRole('button', { name: /uninstall/i }));

    await waitFor(() => {
      expect(ipcMock.uninstallStatusline).toHaveBeenCalledTimes(1);
    });
    expect(await screen.findByRole('button', { name: /install/i })).toBeInTheDocument();
  });

  it('shows an error notice when uninstall fails, and clears the busy state', async () => {
    const state: StatuslineInstallState = {
      installed_command: '/usr/local/bin/switchboard statusline',
      installed_at: 1_700_000_000,
    };
    ipcMock.getStatuslineInstallState.mockResolvedValue(state);
    ipcMock.uninstallStatusline.mockRejectedValue(new Error('settings.json is not writable'));

    render(<StatuslineSettings />);
    const button = await screen.findByRole('button', { name: /uninstall/i });
    fireEvent.click(button);

    expect(await screen.findByRole('status')).toHaveTextContent('settings.json is not writable');
    expect(button).not.toBeDisabled();
  });

  it('shows a drift notice when uninstall is skipped because the file no longer matches what we wrote', async () => {
    const state: StatuslineInstallState = {
      installed_command: '/usr/local/bin/switchboard statusline',
      installed_at: 1_700_000_000,
    };
    ipcMock.getStatuslineInstallState.mockResolvedValue(state);
    ipcMock.uninstallStatusline.mockResolvedValue(false);

    render(<StatuslineSettings />);
    const button = await screen.findByRole('button', { name: /uninstall/i });
    fireEvent.click(button);

    expect(await screen.findByRole('status')).toHaveTextContent(
      "Left your own statusLine edit in place",
    );
    expect(button).not.toBeDisabled();
  });
});
