import { useState, useEffect } from 'react';
import { Card } from '../components/ui/Card';
import { Toggle } from '../components/ui/Toggle';
import { Slider } from '../components/ui/Slider';
import { Badge } from '../components/ui/Badge';
import { Button } from '../components/ui/Button';
import { Select } from '../components/ui/Select';
import type { Terminal } from '../lib/generated/bindings';
import { useAppStore } from '../lib/store';
import { useThemeStore, type ThemePreference } from '../lib/theme';
import type { Settings } from '../lib/types';
import {
  enable as enableAutostart,
  disable as disableAutostart,
  isEnabled as isAutostartEnabled,
} from '@tauri-apps/plugin-autostart';
import { ipc } from '../lib/ipc';
import { WarmupSettings } from './WarmupSettings';

const POLL_MIN_SECS = 60;
const POLL_MAX_SECS = 1800;

/// Display names for the `Terminal` enum. The backend's `label()` is not
/// exposed over IPC, so the mapping is duplicated here — keep in sync with
/// `providers::launcher::Terminal::label`.
const TERMINAL_LABELS: Record<Terminal, string> = {
  ghostty: 'Ghostty',
  terminal_app: 'Terminal.app',
  iterm_2: 'iTerm2',
  kitty: 'kitty',
  wez_term: 'WezTerm',
  windows_terminal: 'Windows Terminal',
  power_shell: 'PowerShell',
};

export function SettingsPanel() {
  const settings = useAppStore((s) => s.settings);
  const setSettings = useAppStore((s) => s.setSettings);
  const usage = useAppStore((s) => s.usage);
  const accounts = useAppStore((s) => s.accounts);
  const themePreference = useThemeStore((s) => s.themePreference);
  const setThemePreference = useThemeStore((s) => s.setThemePreference);
  const [local, setLocal] = useState<Settings | null>(() => settings);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [savedOk, setSavedOk] = useState(false);
  const [consentGranted, setConsentGranted] = useState(false);
  const [osRegistered, setOsRegistered] = useState(false);
  const [availableTerminals, setAvailableTerminals] = useState<Terminal[]>([]);

  useEffect(() => {
    ipc.getWarmupConsentGranted().then(setConsentGranted).catch(() => {});
    ipc.osSchedulerIsRegistered().then(setOsRegistered).catch(() => {});
    ipc.listAvailableTerminals().then(setAvailableTerminals).catch(() => {});
  }, []);

  const handleRevoke = async () => {
    await ipc.revokeWarmupConsent();
    setConsentGranted(false);
  };

  const handleRegisterOs = async () => {
    await ipc.osSchedulerRegister();
    setOsRegistered(true);
  };

  const handleUnregisterOs = async () => {
    await ipc.osSchedulerUnregister();
    setOsRegistered(false);
  };

  if (!local) return <p className="text-[color:var(--color-text-muted)]">Loading...</p>;

  const clamp = (n: number, min: number, max: number) => Math.min(max, Math.max(min, n));
  const pollingMinutes = Math.max(1, Math.round(local.polling_interval_secs / 60));

  function update<K extends keyof Settings>(key: K, value: Settings[K]) {
    setLocal((prev) => (prev ? { ...prev, [key]: value } : prev));
  }

  async function save() {
    if (!local) return;
    setSaving(true);
    setSaveError(null);
    setSavedOk(false);
    try {
      const next: Settings = {
        ...local,
        polling_interval_secs: clamp(local.polling_interval_secs, POLL_MIN_SECS, POLL_MAX_SECS),
      };
      await setSettings(next);
      try {
        // Only toggle the OS autostart entry when the desired state
        // actually differs from the current one. On Windows, calling
        // disable() against a registry value that doesn't exist returns
        // ERROR_FILE_NOT_FOUND (os error 2) — which would surface as
        // "Saved, but autostart toggle failed" on every Save against
        // a never-enabled state.
        const currentlyEnabled = await isAutostartEnabled();
        if (next.launch_at_login && !currentlyEnabled) {
          await enableAutostart();
        } else if (!next.launch_at_login && currentlyEnabled) {
          await disableAutostart();
        }
      } catch (e) {
        // Autostart toggle is best-effort: surface but don't fail the whole save.
        const msg = e instanceof Error ? e.message : String(e);
        setSaveError(`Saved, but autostart toggle failed: ${msg}`);
        return;
      }
      setSavedOk(true);
      setTimeout(() => setSavedOk(false), 2000);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setSaveError(`Save failed: ${msg}`);
    } finally {
      setSaving(false);
    }
  }

  const authSourceLabel = (src: string) => src === 'ClaudeCode' ? 'Claude Code' : src;
  const activeAccount = accounts.find((a) => a.is_active);
  const accountStatus = usage
    ? { connected: true, email: usage.account_email, source: authSourceLabel(usage.auth_source) }
    : activeAccount
      ? { connected: true, email: activeAccount.email, source: activeAccount.cached_usage ? authSourceLabel(activeAccount.cached_usage.auth_source) : 'Claude Code' }
      : { connected: false, email: null, source: null };

  return (
    <div className="flex flex-col gap-[var(--space-lg)]">
      {/* Appearance */}
      <section className="flex flex-col gap-[var(--space-sm)]">
        <h2 className="text-[length:var(--text-label)] font-[var(--weight-semibold)] text-[color:var(--color-text-muted)] uppercase tracking-[0.04em] px-[var(--space-2xs)]">
          Appearance
        </h2>
        <Card className="p-[var(--space-md)] flex flex-col gap-[var(--space-xs)]">
          {(['light', 'dark', 'auto'] as ThemePreference[]).map((opt) => (
            <label
              key={opt}
              className="flex items-center gap-[var(--space-sm)] cursor-pointer py-[var(--space-2xs)]"
            >
              <input
                type="radio"
                name="theme-preference"
                value={opt}
                checked={themePreference === opt}
                onChange={() => setThemePreference(opt)}
                className="accent-[color:var(--color-accent)]"
              />
              <span className="text-[length:var(--text-body)] text-[color:var(--color-text)]">
                {opt === 'light' && 'Light'}
                {opt === 'dark' && 'Dark'}
                {opt === 'auto' && 'Auto (follow system)'}
              </span>
            </label>
          ))}
        </Card>
      </section>

      {/* General */}
      <section className="flex flex-col gap-[var(--space-sm)]">
        <h2 className="text-[length:var(--text-label)] font-[var(--weight-semibold)] text-[color:var(--color-text-muted)] uppercase tracking-[0.04em] px-[var(--space-2xs)]">
          General
        </h2>
        <Card className="p-[var(--space-md)] flex flex-col">
          <Toggle
            label="Launch at login"
            description="Start monitoring when you log in"
            checked={local.launch_at_login}
            onChange={(e) => update('launch_at_login', e.target.checked)}
          />
        </Card>
      </section>

      {/* Polling */}
      <section className="flex flex-col gap-[var(--space-sm)]">
        <h2 className="text-[length:var(--text-label)] font-[var(--weight-semibold)] text-[color:var(--color-text-muted)] uppercase tracking-[0.04em] px-[var(--space-2xs)]">
          Polling
        </h2>
        <Card className="p-[var(--space-md)] flex flex-col gap-[var(--space-md)]">
          <div>
            <Slider
              label="Poll interval"
              min={1}
              max={30}
              step={1}
              value={pollingMinutes}
              onChange={(e) => update('polling_interval_secs', Number(e.target.value) * 60)}
              formatValue={(v) => `${v}m`}
            />
            {pollingMinutes <= 2 && (
              <p className="text-[length:var(--text-micro)] text-[color:var(--color-warn)] mt-[var(--space-xs)]">
                Frequent polling may cause rate limiting
              </p>
            )}
          </div>
          <div>
            <Slider
              label="Stagger gap"
              min={5}
              max={120}
              step={5}
              value={local.stagger_gap_secs}
              onChange={(e) => update('stagger_gap_secs', Number(e.target.value))}
              formatValue={(v) => `${v}s`}
            />
            <p className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] mt-[var(--space-xs)]">
              Spacing between consecutive account polls in one round.
            </p>
            {accounts.length > 1 &&
              accounts.length * local.stagger_gap_secs > local.polling_interval_secs && (
                <p className="text-[length:var(--text-micro)] text-[color:var(--color-warn)] mt-[var(--space-xs)]">
                  {accounts.length} accounts × {local.stagger_gap_secs}s won't fit in{' '}
                  {Math.round(local.polling_interval_secs / 60)}m — gap will compress to{' '}
                  {Math.floor(local.polling_interval_secs / accounts.length)}s per slot.
                </p>
              )}
          </div>
        </Card>
      </section>

      {/* Thresholds */}
      <section className="flex flex-col gap-[var(--space-sm)]">
        <h2 className="text-[length:var(--text-label)] font-[var(--weight-semibold)] text-[color:var(--color-text-muted)] uppercase tracking-[0.04em] px-[var(--space-2xs)]">
          Notifications
        </h2>
        <Card className="p-[var(--space-md)] flex flex-col gap-[var(--space-md)]">
          {local.thresholds.map((t, i) => (
            <Slider
              key={i}
              label={`Threshold ${i + 1}`}
              min={25}
              max={95}
              step={5}
              value={t}
              onChange={(e) => {
                const v = Number(e.target.value);
                const next = [...local.thresholds];
                next[i] = v;
                update('thresholds', next);
              }}
              formatValue={(v) => `${v}%`}
            />
          ))}
          <Slider
            label="Pay-as-you-go threshold"
            min={25}
            max={95}
            step={5}
            value={local.payg_threshold}
            onChange={(e) => update('payg_threshold', Number(e.target.value))}
            formatValue={(v) => `${v}%`}
          />
          <p className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] px-[var(--space-2xs)]">
            Credits alert fires at this level — separate from the rate-limit thresholds above.
          </p>
          <Toggle
            label="Session finished"
            description="Notify when a session that ran 10+ minutes goes quiet."
            checked={local.notify_session_finished}
            onChange={(e) => update('notify_session_finished', e.target.checked)}
          />
          <div className="flex items-center gap-[var(--space-sm)] px-[var(--space-2xs)]">
            <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
              Notifications fire once per bucket reset cycle
            </span>
          </div>
        </Card>
      </section>

      {/* Terminal */}
      <section className="flex flex-col gap-[var(--space-sm)]">
        <h2 className="text-[length:var(--text-label)] font-[var(--weight-semibold)] text-[color:var(--color-text-muted)] uppercase tracking-[0.04em] px-[var(--space-2xs)]">
          Terminal
        </h2>
        <Card className="p-[var(--space-md)]">
          <Select
            label="Terminal"
            value={local.terminal ?? ''}
            onChange={(e) => update('terminal', (e.target.value || null) as Settings['terminal'])}
            options={[
              { value: '', label: 'System default' },
              ...availableTerminals.map((t) => ({ value: t, label: TERMINAL_LABELS[t] ?? t })),
            ]}
          />
          <p className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] mt-[var(--space-xs)]">
            Used when launching or resuming a session from the Providers and Sessions tabs. Only
            terminals installed on this machine are listed.
          </p>
        </Card>
      </section>

      {/* Account */}
      <section className="flex flex-col gap-[var(--space-sm)]">
        <h2 className="text-[length:var(--text-label)] font-[var(--weight-semibold)] text-[color:var(--color-text-muted)] uppercase tracking-[0.04em] px-[var(--space-2xs)]">
          Account
        </h2>
        <Card className="p-[var(--space-md)]">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-[var(--space-sm)]">
              <span className="text-[length:var(--text-body)] text-[color:var(--color-text)]">
                {accountStatus.connected ? (accountStatus.email ?? 'Connected') : 'Not signed in'}
              </span>
              {accountStatus.source && <Badge variant="live">{accountStatus.source}</Badge>}
            </div>
            {!accountStatus.connected && (
              <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
                Manage in Accounts
              </span>
            )}
          </div>
        </Card>
      </section>

      {/* Warm-up */}
      <section className="flex flex-col gap-[var(--space-sm)]">
        <h2 className="text-[length:var(--text-label)] font-[var(--weight-semibold)] text-[color:var(--color-text-muted)] uppercase tracking-[0.04em] px-[var(--space-2xs)]">
          Warm-up
        </h2>
        <Card className="p-[var(--space-md)]">
          <WarmupSettings
            consentGranted={consentGranted}
            osSchedulerRegistered={osRegistered}
            onRevoke={handleRevoke}
            onRegisterOs={handleRegisterOs}
            onUnregisterOs={handleUnregisterOs}
          />
        </Card>
      </section>

      {/* Save */}
      <div className="flex flex-col gap-[var(--space-xs)] px-[var(--space-2xs)]">
        {saveError && (
          <span className="text-[length:var(--text-micro)] text-[color:var(--color-danger)]">{saveError}</span>
        )}
        <div className="flex items-center justify-end gap-[var(--space-sm)]">
          {savedOk && (
            <span
              className="text-[length:var(--text-label)] font-[var(--weight-medium)] text-[color:var(--color-accent)]"
              style={{ animation: 'fadeIn 150ms ease-out' }}
            >
              ✓ Settings saved
            </span>
          )}
          <Button variant="primary" onClick={save} disabled={saving}>
            {saving ? 'Saving…' : 'Save'}
          </Button>
        </div>
      </div>
    </div>
  );
}
