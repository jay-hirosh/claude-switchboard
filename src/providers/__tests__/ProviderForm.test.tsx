import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';

const ipcMock = vi.hoisted(() => ({
  listProviderPresets: vi.fn().mockResolvedValue([
    {
      id: 'glm',
      name: 'GLM (z.ai)',
      base_url: 'https://api.z.ai/api/anthropic',
      website: 'https://z.ai',
      env: { ANTHROPIC_MODEL: 'glm-5.2', CLAUDE_CODE_MAX_CONTEXT_TOKENS: '1000000' },
    },
  ]),
  listProviders: vi.fn().mockResolvedValue([]),
  upsertProvider: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('../../lib/ipc', () => ({ ipc: ipcMock }));

import { ProviderForm } from '../ProviderForm';

describe('ProviderForm', () => {
  beforeEach(() => vi.clearAllMocks());

  it('prefills base URL and model when a preset is chosen', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/preset/i)).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/preset/i), { target: { value: 'glm' } });
    await waitFor(() =>
      expect((screen.getByLabelText(/base url/i) as HTMLInputElement).value).toBe(
        'https://api.z.ai/api/anthropic',
      ),
    );
    expect((screen.getByLabelText(/^model/i) as HTMLInputElement).value).toBe('glm-5.2');
  });

  it('carries the preset context-window knobs into the saved provider', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/preset/i)).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/preset/i), { target: { value: 'glm' } });
    fireEvent.change(screen.getByLabelText(/api key/i), { target: { value: 'sk-test' } });
    await waitFor(() =>
      expect((screen.getByLabelText(/base url/i) as HTMLInputElement).value).toBeTruthy(),
    );
    fireEvent.click(screen.getByRole('button', { name: /save/i }));
    await waitFor(() => expect(ipcMock.upsertProvider).toHaveBeenCalled());
    const saved = ipcMock.upsertProvider.mock.calls[0][0];
    expect(saved.env.CLAUDE_CODE_MAX_CONTEXT_TOKENS).toBe('1000000');
    expect(saved.auth_token).toBe('sk-test');
    expect(saved.kind).toBe('third_party');
  });

  it('splits extra CLI arguments into separate argv entries', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/preset/i)).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/preset/i), { target: { value: 'glm' } });
    fireEvent.change(screen.getByLabelText(/api key/i), { target: { value: 'sk-test' } });
    fireEvent.change(screen.getByLabelText(/extra cli arguments/i), {
      target: { value: '--dangerously-skip-permissions  --verbose' },
    });
    await waitFor(() =>
      expect((screen.getByLabelText(/base url/i) as HTMLInputElement).value).toBeTruthy(),
    );
    fireEvent.click(screen.getByRole('button', { name: /save/i }));
    await waitFor(() => expect(ipcMock.upsertProvider).toHaveBeenCalled());
    expect(ipcMock.upsertProvider.mock.calls[0][0].extra_args).toEqual([
      '--dangerously-skip-permissions',
      '--verbose',
    ]);
  });

  it('refuses to save without a name and a base URL', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/preset/i)).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /save/i }));
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    expect(ipcMock.upsertProvider).not.toHaveBeenCalled();
  });

  it('rejects a base URL that is not http(s)', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/name/i)).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/name/i), { target: { value: 'X' } });
    fireEvent.change(screen.getByLabelText(/base url/i), { target: { value: 'ftp://nope' } });
    fireEvent.click(screen.getByRole('button', { name: /save/i }));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/https/i));
    expect(ipcMock.upsertProvider).not.toHaveBeenCalled();
  });
});
