import { render, screen, fireEvent, cleanup } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { ModelRoutingHint } from './ModelRoutingHint';
import { dismissRoutingHint } from '../lib/routingHintDismissal';
import type { ModelRoutingHint as ModelRoutingHintData } from '../lib/modelRoutingHint';

function hint(overrides: Partial<ModelRoutingHintData> = {}): ModelRoutingHintData {
  return {
    busier: 'opus',
    busierPct: 82,
    quieterPct: 31,
    resetsAt: '2026-08-20T00:00:00Z',
    ...overrides,
  };
}

describe('ModelRoutingHint', () => {
  beforeEach(() => {
    localStorage.clear();
  });
  afterEach(() => {
    cleanup();
  });

  it('renders the busier/quieter split and a /model suggestion for the quieter model', () => {
    render(<ModelRoutingHint accountId="acct-1" hint={hint()} />);
    expect(screen.getByText(/Opus 7D at 82%/)).toBeTruthy();
    expect(screen.getByText(/Sonnet at 31%/)).toBeTruthy();
    expect(screen.getByText(/\/model sonnet/)).toBeTruthy();
  });

  it('hides itself and persists the dismissal when the dismiss button is clicked', () => {
    render(<ModelRoutingHint accountId="acct-1" hint={hint()} />);
    fireEvent.click(screen.getByRole('button', { name: /dismiss/i }));
    expect(screen.queryByText(/Opus 7D at 82%/)).toBeNull();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('does not render when already dismissed for the current resetsAt', () => {
    dismissRoutingHint('acct-1', '2026-08-20T00:00:00Z');
    render(<ModelRoutingHint accountId="acct-1" hint={hint()} />);
    expect(screen.queryByText(/Opus 7D/)).toBeNull();
  });

  it('renders again once the resetsAt has moved past a prior dismissal', () => {
    dismissRoutingHint('acct-1', '2026-08-20T00:00:00Z');
    render(<ModelRoutingHint accountId="acct-1" hint={hint({ resetsAt: '2026-08-27T00:00:00Z' })} />);
    expect(screen.getByText(/Opus 7D at 82%/)).toBeTruthy();
  });
});
