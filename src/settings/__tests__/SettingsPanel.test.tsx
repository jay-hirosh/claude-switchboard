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
  icon_style: 'dual',
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

const setAccentColor = vi.fn();
const setDensity = vi.fn();
const setReduceMotion = vi.fn();

vi.mock('../../lib/appearance', () => ({
  useAppearanceStore: (sel: (s: any) => any) =>
    sel({
      accentColor: 'terracotta',
      density: 'comfortable',
      reduceMotion: false,
      setAccentColor,
      setDensity,
      setReduceMotion,
    }),
}));

// StatuslineSettings (rendered inside SettingsPanel) is self-contained and
// fetches on mount via ipc.getStatuslineInstallState, matching ProvidersTab's
// pattern — tests that render it must mock '../../lib/ipc' the same way
// ProvidersTab.test.tsx does, or the real bindings hit
// `window.__TAURI_INTERNALS__` (undefined under jsdom) and reject unhandled.
vi.mock('../../lib/ipc', () => ({
  ipc: {
    getWarmupConsentGranted: vi.fn().mockResolvedValue(false),
    osSchedulerIsRegistered: vi.fn().mockResolvedValue(false),
    listAvailableTerminals: vi.fn().mockResolvedValue([]),
    getStatuslineInstallState: vi.fn().mockResolvedValue(null),
    revokeWarmupConsent: vi.fn().mockResolvedValue(undefined),
    osSchedulerRegister: vi.fn().mockResolvedValue(undefined),
    osSchedulerUnregister: vi.fn().mockResolvedValue(undefined),
    getSyncBackendUrl: vi.fn().mockResolvedValue(null),
    getSyncStatus: vi.fn().mockResolvedValue(null),
    setSyncBackendUrl: vi.fn().mockResolvedValue(undefined),
    bootstrapSyncAccount: vi.fn().mockResolvedValue(undefined),
    generatePairingCode: vi.fn().mockResolvedValue(''),
    joinSyncAccount: vi.fn().mockResolvedValue(undefined),
    syncNow: vi.fn().mockResolvedValue({ last_run_at: '', outcome: 'ok', pushed: 0, pulled: 0 }),
  },
}));

describe('SettingsPanel', () => {
  it('renders and updates the pay-as-you-go threshold slider', async () => {
    render(<SettingsPanel />);

    // findByLabelText (rather than getByLabelText) waits for and flushes the
    // StatuslineSettings mount fetch, so its pending state update doesn't
    // land outside act() after the test body returns.
    const slider = (await screen.findByLabelText(/pay-as-you-go threshold/i)) as HTMLInputElement;
    expect(slider).toBeInTheDocument();
    expect(screen.getByText('85%')).toBeInTheDocument();

    fireEvent.change(slider, { target: { value: '90' } });

    expect(screen.getByText('90%')).toBeInTheDocument();
  });

  it('renders and toggles the session-finished notification setting', async () => {
    render(<SettingsPanel />);
    const toggle = (await screen.findByLabelText(/session finished/i)) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
    fireEvent.click(toggle);
    expect(toggle.checked).toBe(false);
  });

  it('renders and toggles the context-warning notification setting', async () => {
    render(<SettingsPanel />);
    const toggle = (await screen.findByLabelText(/context warnings/i)) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
    fireEvent.click(toggle);
    expect(toggle.checked).toBe(false);
  });

  it('selects an accent color swatch', async () => {
    render(<SettingsPanel />);
    const teal = await screen.findByRole('button', { name: /teal/i });
    fireEvent.click(teal);
    expect(setAccentColor).toHaveBeenCalledWith('teal');
  });

  it('marks the active accent swatch as pressed', async () => {
    render(<SettingsPanel />);
    const terracotta = await screen.findByRole('button', { name: /terracotta/i });
    expect(terracotta).toHaveAttribute('aria-pressed', 'true');
    const teal = await screen.findByRole('button', { name: /teal/i });
    expect(teal).toHaveAttribute('aria-pressed', 'false');
  });

  it('toggles compact density', async () => {
    render(<SettingsPanel />);
    const toggle = (await screen.findByLabelText(/compact density/i)) as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    fireEvent.click(toggle);
    expect(setDensity).toHaveBeenCalledWith('compact');
  });

  it('toggles reduce motion', async () => {
    render(<SettingsPanel />);
    const toggle = (await screen.findByLabelText(/reduce motion/i)) as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    fireEvent.click(toggle);
    expect(setReduceMotion).toHaveBeenCalledWith(true);
  });

  it('renders and changes the tray icon style', async () => {
    render(<SettingsPanel />);
    const select = (await screen.findByLabelText(/icon style/i)) as HTMLSelectElement;
    expect(select.value).toBe('dual');
    fireEvent.change(select, { target: { value: 'minimal' } });
    expect(select.value).toBe('minimal');
  });
});
