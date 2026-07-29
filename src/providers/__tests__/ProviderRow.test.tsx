import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import type { Provider } from '../../lib/generated/bindings';
import { ProviderRow } from '../ProviderRow';

function glm(): Provider {
  return {
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
}

function official(): Provider {
  return {
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
}

describe('ProviderRow', () => {
  it('shows the name, host and model', () => {
    render(<ProviderRow provider={glm()} onLaunch={vi.fn()} onEdit={vi.fn()} onDelete={vi.fn()} />);
    expect(screen.getByText('GLM')).toBeTruthy();
    expect(screen.getByText(/api\.z\.ai/)).toBeTruthy();
    expect(screen.getByText('glm-5.2')).toBeTruthy();
  });

  it('never renders the auth token', () => {
    const { container } = render(
      <ProviderRow provider={glm()} onLaunch={vi.fn()} onEdit={vi.fn()} onDelete={vi.fn()} />,
    );
    expect(container.textContent).not.toContain('tok');
  });

  it('calls onLaunch with the provider id', () => {
    const onLaunch = vi.fn();
    render(<ProviderRow provider={glm()} onLaunch={onLaunch} onEdit={vi.fn()} onDelete={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /launch/i }));
    expect(onLaunch).toHaveBeenCalledWith('p1');
  });

  it('offers no delete control for the official provider', () => {
    render(
      <ProviderRow provider={official()} onLaunch={vi.fn()} onEdit={vi.fn()} onDelete={vi.fn()} />,
    );
    expect(screen.queryByRole('button', { name: /delete/i })).toBeNull();
  });

  it('describes the official provider as using the active account', () => {
    render(
      <ProviderRow provider={official()} onLaunch={vi.fn()} onEdit={vi.fn()} onDelete={vi.fn()} />,
    );
    expect(screen.getByText(/active account/i)).toBeTruthy();
  });

  // Its credentials come from the active account, but its launch flags are the
  // user's — and there was previously no way to set them.
  it('offers an edit control for the official provider', () => {
    const onEdit = vi.fn();
    render(
      <ProviderRow provider={official()} onLaunch={vi.fn()} onEdit={onEdit} onDelete={vi.fn()} />,
    );
    fireEvent.click(screen.getByRole('button', { name: /edit/i }));
    expect(onEdit).toHaveBeenCalledWith('official');
  });
});
