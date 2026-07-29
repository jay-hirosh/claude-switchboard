import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { DefaultProviderBanner } from '../DefaultProviderBanner';

describe('DefaultProviderBanner', () => {
  it('names the provider and warns about shell overrides', () => {
    render(<DefaultProviderBanner providerName="GLM" onClear={vi.fn()} />);
    expect(screen.getByText(/GLM/)).toBeTruthy();
    expect(screen.getByText(/overrides/i)).toBeTruthy();
  });

  it('warns that running sessions are unaffected', () => {
    render(<DefaultProviderBanner providerName="GLM" onClear={vi.fn()} />);
    expect(screen.getByText(/already running/i)).toBeTruthy();
  });

  it('calls onClear when the user turns the default off', () => {
    const onClear = vi.fn();
    render(<DefaultProviderBanner providerName="GLM" onClear={onClear} />);
    fireEvent.click(screen.getByRole('button', { name: /turn off/i }));
    expect(onClear).toHaveBeenCalled();
  });
});
