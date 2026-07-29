import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';

const ipcMock = vi.hoisted(() => ({
  listProviders: vi.fn(),
  listAvailableTerminals: vi.fn().mockResolvedValue(['ghostty']),
  getSettings: vi.fn().mockResolvedValue({ terminal: null }),
  launchProviderSession: vi.fn().mockResolvedValue('/tmp/a.sh'),
}));
vi.mock('../../lib/ipc', () => ({ ipc: ipcMock }));

import { useResume } from '../useResume';

const glmProvider = {
  id: 'glm',
  name: 'GLM',
  kind: 'third_party',
  base_url: 'https://api.z.ai/api/anthropic',
  auth_token: 't',
  env: { ANTHROPIC_MODEL: 'glm-5.2[1m]' },
  extra_args: [],
  preset_id: 'glm',
  sort_index: 1,
};

const session = {
  session_id: 'sess-1',
  cwd: '/w/proj',
  project_name: 'proj',
  git_branch: 'main',
  title: 'T',
  recap: null,
  asked: 'a',
  left_off: null,
  touched_files: [],
  touched_overflow: 0,
  model: 'glm-5.2',
  turns: 2,
  started_at: '',
  ended_at: '',
  permission_mode: 'bypassPermissions',
  cwd_exists: true,
};

function Harness({ s }: { s: unknown }) {
  const { resume, dialog } = useResume();
  return (
    <>
      <button onClick={() => resume(s as never)}>go</button>
      {dialog}
    </>
  );
}

describe('useResume', () => {
  beforeEach(() => vi.clearAllMocks());

  // The permission mode rides along, so the continued session behaves the way
  // the one it continues did.
  it('launches directly when the model resolves, with the session cwd and mode', async () => {
    ipcMock.listProviders.mockResolvedValue([glmProvider]);
    render(<Harness s={session} />);
    fireEvent.click(screen.getByText('go'));
    await waitFor(() =>
      expect(ipcMock.launchProviderSession).toHaveBeenCalledWith(
        'glm',
        '/w/proj',
        'ghostty',
        'sess-1',
        'bypassPermissions',
      ),
    );
  });

  it('passes no mode when the session recorded none', async () => {
    ipcMock.listProviders.mockResolvedValue([glmProvider]);
    render(<Harness s={{ ...session, permission_mode: null }} />);
    fireEvent.click(screen.getByText('go'));
    await waitFor(() =>
      expect(ipcMock.launchProviderSession).toHaveBeenCalledWith(
        'glm',
        '/w/proj',
        'ghostty',
        'sess-1',
        null,
      ),
    );
  });

  it('prompts instead of launching when the model does not resolve', async () => {
    ipcMock.listProviders.mockResolvedValue([glmProvider]);
    render(<Harness s={{ ...session, model: 'mystery-9' }} />);
    fireEvent.click(screen.getByText('go'));
    await waitFor(() => expect(screen.getByRole('dialog')).toBeTruthy());
    expect(ipcMock.launchProviderSession).not.toHaveBeenCalled();
  });

  it('prompts when no model was recorded', async () => {
    ipcMock.listProviders.mockResolvedValue([glmProvider]);
    render(<Harness s={{ ...session, model: null }} />);
    fireEvent.click(screen.getByText('go'));
    await waitFor(() => expect(screen.getByRole('dialog')).toBeTruthy());
    expect(ipcMock.launchProviderSession).not.toHaveBeenCalled();
  });

  it('warns about cross-model resume in the picker', async () => {
    ipcMock.listProviders.mockResolvedValue([glmProvider]);
    render(<Harness s={{ ...session, model: 'mystery-9' }} />);
    fireEvent.click(screen.getByText('go'));
    await waitFor(() => expect(screen.getByRole('dialog')).toBeTruthy());
    expect(screen.getByText(/thinking/i)).toBeTruthy();
  });

  it('launches with the chosen provider after confirmation', async () => {
    ipcMock.listProviders.mockResolvedValue([glmProvider]);
    render(<Harness s={{ ...session, model: 'mystery-9' }} />);
    fireEvent.click(screen.getByText('go'));
    await waitFor(() => expect(screen.getByRole('dialog')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /^resume$/i }));
    await waitFor(() =>
      expect(ipcMock.launchProviderSession).toHaveBeenCalledWith(
        'glm',
        '/w/proj',
        'ghostty',
        'sess-1',
        'bypassPermissions',
      ),
    );
  });
});
