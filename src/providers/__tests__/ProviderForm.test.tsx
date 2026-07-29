import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';

const ipcMock = vi.hoisted(() => ({
  listProviderPresets: vi.fn().mockResolvedValue([
    {
      id: 'glm',
      name: 'GLM (z.ai)',
      base_url: 'https://api.z.ai/api/anthropic',
      website: 'https://z.ai',
      env: {
        ANTHROPIC_MODEL: 'glm-5.2',
        ANTHROPIC_SMALL_FAST_MODEL: 'glm-5-turbo',
        CLAUDE_CODE_MAX_CONTEXT_TOKENS: '1000000',
      },
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

  // The flag used to exist only as an input placeholder — it looked prefilled
  // but nothing had typed it, nothing would save it, and there was no way to
  // accept it. The chip has to put real text into the field.
  it('adds a common flag to the field when its chip is clicked', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/preset/i)).toBeTruthy());

    const field = screen.getByLabelText(/extra cli arguments/i) as HTMLInputElement;
    expect(field.value).toBe('');

    fireEvent.click(screen.getByRole('button', { name: /--dangerously-skip-permissions/ }));
    await waitFor(() => expect(field.value).toBe('--dangerously-skip-permissions'));

    fireEvent.click(screen.getByRole('button', { name: /--continue/ }));
    await waitFor(() =>
      expect(field.value).toBe('--dangerously-skip-permissions --continue'),
    );
  });

  it('removes a flag when its chip is clicked again, leaving the rest intact', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/preset/i)).toBeTruthy());

    const field = screen.getByLabelText(/extra cli arguments/i) as HTMLInputElement;
    fireEvent.change(field, {
      target: { value: '--dangerously-skip-permissions --verbose' },
    });

    const chip = screen.getByRole('button', { name: /--dangerously-skip-permissions/ });
    expect(chip.getAttribute('aria-pressed')).toBe('true');

    fireEvent.click(chip);
    await waitFor(() => expect(field.value).toBe('--verbose'));
  });

  it('saves a flag added by chip', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/preset/i)).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/preset/i), { target: { value: 'glm' } });
    fireEvent.click(screen.getByRole('button', { name: /--dangerously-skip-permissions/ }));
    await waitFor(() =>
      expect((screen.getByLabelText(/base url/i) as HTMLInputElement).value).toBeTruthy(),
    );
    fireEvent.click(screen.getByRole('button', { name: /^save$/i }));
    await waitFor(() => expect(ipcMock.upsertProvider).toHaveBeenCalled());
    expect(ipcMock.upsertProvider.mock.calls[0][0].extra_args).toEqual([
      '--dangerously-skip-permissions',
    ]);
  });

  it('prefills the quick model from the preset', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/preset/i)).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/preset/i), { target: { value: 'glm' } });
    await waitFor(() =>
      expect((screen.getByLabelText(/quick model/i) as HTMLInputElement).value).toBe(
        'glm-5-turbo',
      ),
    );
  });

  // Before this, the form wrote ANTHROPIC_MODEL alone. A custom provider got no
  // aliases at all, so `/model opus` resolved to a first-party Anthropic id and
  // the third-party endpoint rejected it.
  it('expands both model fields into the aliases Claude Code reads', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/name/i)).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/name/i), { target: { value: 'Custom' } });
    fireEvent.change(screen.getByLabelText(/base url/i), {
      target: { value: 'https://api.example.com/anthropic' },
    });
    fireEvent.change(screen.getByLabelText(/^model/i), { target: { value: 'big-1' } });
    fireEvent.change(screen.getByLabelText(/quick model/i), { target: { value: 'fast-1' } });
    fireEvent.click(screen.getByRole('button', { name: /^save$/i }));

    await waitFor(() => expect(ipcMock.upsertProvider).toHaveBeenCalled());
    expect(ipcMock.upsertProvider.mock.calls[0][0].env).toEqual({
      ANTHROPIC_MODEL: 'big-1',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'big-1',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'big-1',
      ANTHROPIC_DEFAULT_FABLE_MODEL: 'big-1',
      ANTHROPIC_SMALL_FAST_MODEL: 'fast-1',
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'fast-1',
    });
  });

  it('rewrites a preset’s aliases when the model is edited', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/preset/i)).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/preset/i), { target: { value: 'glm' } });
    await waitFor(() =>
      expect((screen.getByLabelText(/^model/i) as HTMLInputElement).value).toBe('glm-5.2'),
    );
    fireEvent.change(screen.getByLabelText(/^model/i), { target: { value: 'glm-6' } });
    fireEvent.click(screen.getByRole('button', { name: /^save$/i }));

    await waitFor(() => expect(ipcMock.upsertProvider).toHaveBeenCalled());
    const { env } = ipcMock.upsertProvider.mock.calls[0][0];
    expect(env.ANTHROPIC_DEFAULT_OPUS_MODEL).toBe('glm-6');
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe('glm-6');
    // Unrelated preset knobs survive the rewrite.
    expect(env.CLAUDE_CODE_MAX_CONTEXT_TOKENS).toBe('1000000');
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
