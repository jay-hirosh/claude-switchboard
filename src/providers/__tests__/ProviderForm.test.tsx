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
    {
      id: 'ollama',
      name: 'Ollama (local)',
      base_url: 'http://localhost:11434',
      website: 'https://ollama.com/library',
      env: {
        ANTHROPIC_AUTH_TOKEN: 'ollama',
        ANTHROPIC_MODEL: 'llama3.2',
        ANTHROPIC_SMALL_FAST_MODEL: 'llama3.2',
        CLAUDE_CODE_MAX_CONTEXT_TOKENS: '32768',
      },
    },
  ]),
  listProviders: vi.fn().mockResolvedValue([]),
  upsertProvider: vi.fn().mockResolvedValue(undefined),
  listOllamaModels: vi.fn().mockResolvedValue(['gpt-oss:20b', 'llama3.2:latest']),
}));
vi.mock('../../lib/ipc', () => ({ ipc: ipcMock }));

import { ProviderForm } from '../ProviderForm';

describe('ProviderForm', () => {
  beforeEach(() => vi.clearAllMocks());

  it('prefills base URL and model when a preset is chosen', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    // The <select> renders before its options load, so waiting on the select
    // alone lets fireEvent.change fire against an option that does not exist
    // yet — it silently no-ops and the preset is never applied.
    await waitFor(() => expect(screen.getByRole('option', { name: /GLM/ })).toBeTruthy());
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
    // The <select> renders before its options load, so waiting on the select
    // alone lets fireEvent.change fire against an option that does not exist
    // yet — it silently no-ops and the preset is never applied.
    await waitFor(() => expect(screen.getByRole('option', { name: /GLM/ })).toBeTruthy());
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
    // The <select> renders before its options load, so waiting on the select
    // alone lets fireEvent.change fire against an option that does not exist
    // yet — it silently no-ops and the preset is never applied.
    await waitFor(() => expect(screen.getByRole('option', { name: /GLM/ })).toBeTruthy());
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
    // The <select> renders before its options load, so waiting on the select
    // alone lets fireEvent.change fire against an option that does not exist
    // yet — it silently no-ops and the preset is never applied.
    await waitFor(() => expect(screen.getByRole('option', { name: /GLM/ })).toBeTruthy());
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
    // The <select> renders before its options load, so waiting on the select
    // alone lets fireEvent.change fire against an option that does not exist
    // yet — it silently no-ops and the preset is never applied.
    await waitFor(() => expect(screen.getByRole('option', { name: /GLM/ })).toBeTruthy());
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
    // The <select> renders before its options load, so waiting on the select
    // alone lets fireEvent.change fire against an option that does not exist
    // yet — it silently no-ops and the preset is never applied.
    await waitFor(() => expect(screen.getByRole('option', { name: /GLM/ })).toBeTruthy());
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

  describe('Ollama model discovery', () => {
    it('offers installed models as suggestions once the local server responds', async () => {
      render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
      await waitFor(() => expect(screen.getByRole('option', { name: /Ollama/ })).toBeTruthy());
      fireEvent.change(screen.getByLabelText(/preset/i), { target: { value: 'ollama' } });

      await waitFor(() => expect(ipcMock.listOllamaModels).toHaveBeenCalledWith('http://localhost:11434'));
      await waitFor(() => {
        const options = Array.from(document.querySelectorAll('#ollama-model-list option')).map(
          (o) => (o as HTMLOptionElement).value,
        );
        expect(options).toEqual(['gpt-oss:20b', 'llama3.2:latest']);
      });

      // Still a plain, typable text input — not a closed dropdown.
      const modelField = screen.getByLabelText(/^model/i) as HTMLInputElement;
      fireEvent.change(modelField, { target: { value: 'not-yet-pulled' } });
      expect(modelField.value).toBe('not-yet-pulled');
    });

    it('falls back to a plain field with a hint when Ollama is unreachable', async () => {
      ipcMock.listOllamaModels.mockRejectedValueOnce(new Error('connection refused'));
      render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
      await waitFor(() => expect(screen.getByRole('option', { name: /Ollama/ })).toBeTruthy());
      fireEvent.change(screen.getByLabelText(/preset/i), { target: { value: 'ollama' } });

      await waitFor(() =>
        expect(screen.getByText(/couldn.t reach ollama/i)).toBeTruthy(),
      );
      const modelField = screen.getByLabelText(/^model/i) as HTMLInputElement;
      fireEvent.change(modelField, { target: { value: 'llama3.2' } });
      expect(modelField.value).toBe('llama3.2');
    });

    it('does not offer Ollama suggestions for other presets', async () => {
      render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
      await waitFor(() => expect(screen.getByRole('option', { name: /GLM/ })).toBeTruthy());
      fireEvent.change(screen.getByLabelText(/preset/i), { target: { value: 'glm' } });
      await waitFor(() =>
        expect((screen.getByLabelText(/^model/i) as HTMLInputElement).value).toBe('glm-5.2'),
      );
      expect(ipcMock.listOllamaModels).not.toHaveBeenCalled();
      expect(document.querySelector('#ollama-model-list')).toBeNull();
    });

    it('loads suggestions when opening an existing Ollama provider for edit', async () => {
      ipcMock.listProviders.mockResolvedValue([
        {
          id: 'p-ollama',
          name: 'Ollama (local)',
          kind: 'third_party',
          base_url: 'http://localhost:11500',
          auth_token: 'ollama',
          env: { ANTHROPIC_MODEL: 'llama3.2' },
          extra_args: [],
          preset_id: 'ollama',
          sort_index: 1,
        },
      ]);
      render(<ProviderForm providerId="p-ollama" onClose={vi.fn()} onSaved={vi.fn()} />);
      await waitFor(() =>
        expect(ipcMock.listOllamaModels).toHaveBeenCalledWith('http://localhost:11500'),
      );
    });
  });

  describe('the official provider', () => {
    const official = {
      id: 'official',
      name: 'Anthropic (official)',
      kind: 'official' as const,
      base_url: null,
      auth_token: null,
      env: {},
      extra_args: [],
      preset_id: null,
      sort_index: 0,
    };

    // Its endpoint, credentials and model all come from the active account —
    // resolved_env is empty for it — so those controls would silently do
    // nothing. Only the launch flags are real.
    it('shows only the flags, not the inert credential fields', async () => {
      ipcMock.listProviders.mockResolvedValue([official]);
      render(
        <ProviderForm
          providerId="official"
          providerKind="official"
          onClose={vi.fn()}
          onSaved={vi.fn()}
        />,
      );
      await waitFor(() => expect(screen.getByLabelText(/extra cli arguments/i)).toBeTruthy());
      expect(screen.queryByLabelText(/base url/i)).toBeNull();
      expect(screen.queryByLabelText(/api key/i)).toBeNull();
      expect(screen.queryByLabelText(/^preset$/i)).toBeNull();
      expect(screen.queryByLabelText(/^model$/i)).toBeNull();
    });

    it('saves flags without demanding a name or base URL', async () => {
      ipcMock.listProviders.mockResolvedValue([official]);
      render(
        <ProviderForm
          providerId="official"
          providerKind="official"
          onClose={vi.fn()}
          onSaved={vi.fn()}
        />,
      );
      await waitFor(() => expect(screen.getByLabelText(/extra cli arguments/i)).toBeTruthy());
      fireEvent.change(screen.getByLabelText(/extra cli arguments/i), {
        target: { value: '--dangerously-skip-permissions' },
      });
      fireEvent.click(screen.getByRole('button', { name: /save/i }));
      await waitFor(() => expect(ipcMock.upsertProvider).toHaveBeenCalled());

      const saved = ipcMock.upsertProvider.mock.calls[0][0];
      expect(saved.extra_args).toEqual(['--dangerously-skip-permissions']);
      // Saving must not rewrite the official row into a custom endpoint.
      expect(saved.kind).toBe('official');
      expect(saved.base_url).toBeNull();
      expect(saved.auth_token).toBeNull();
      expect(saved.id).toBe('official');
    });
  });
});
