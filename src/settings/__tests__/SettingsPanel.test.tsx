import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { SettingsPanel } from '../SettingsPanel';
import type { Settings } from '../../lib/generated/bindings';

const baseSettings: Settings = {
  polling_interval_secs: 300,
  stagger_gap_secs: 15,
  thresholds: [50, 70],
  theme: 'auto',
  launch_at_login: false,
  crash_reports: false,
  preferred_auth_source: null,
  terminal: null,
  payg_threshold: 85,
  notify_session_finished: true,
  notify_context_warning: true,
};

vi.mock('../../lib/store', () => ({
  useAppStore: (sel: (s: any) => any) =>
    sel({
      settings: baseSettings,
      setSettings: vi.fn(),
      usage: null,
      accounts: [],
    }),
}));

vi.mock('../../lib/theme', () => ({
  useThemeStore: (sel: (s: any) => any) =>
    sel({ themePreference: 'auto', setThemePreference: vi.fn() }),
}));

describe('SettingsPanel', () => {
  it('renders and updates the pay-as-you-go threshold slider', () => {
    render(<SettingsPanel />);

    const slider = screen.getByLabelText(/pay-as-you-go threshold/i) as HTMLInputElement;
    expect(slider).toBeInTheDocument();
    expect(screen.getByText('85%')).toBeInTheDocument();

    fireEvent.change(slider, { target: { value: '90' } });

    expect(screen.getByText('90%')).toBeInTheDocument();
  });

  it('renders and toggles the session-finished notification setting', () => {
    render(<SettingsPanel />);
    const toggle = screen.getByLabelText(/session finished/i) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
    fireEvent.click(toggle);
    expect(toggle.checked).toBe(false);
  });

  it('renders and toggles the context-warning notification setting', () => {
    render(<SettingsPanel />);
    const toggle = screen.getByLabelText(/context warnings/i) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
    fireEvent.click(toggle);
    expect(toggle.checked).toBe(false);
  });
});
