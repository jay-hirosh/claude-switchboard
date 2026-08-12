import { useEffect, useMemo, useState } from 'react';
import { motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import { Banner } from '../components/ui/Banner';
import { IconButton } from '../components/ui/IconButton';
import { UpdateBanner } from '../components/UpdateBanner';
import { UsageSummary } from '../components/UsageSummary';
import { NowRunningSection } from './NowRunningSection';
import { SettingsPanel } from '../settings/SettingsPanel';
import { AccountsPanel } from '../accounts/AccountsPanel';
import { UnmanagedActiveBanner } from '../accounts/UnmanagedActiveBanner';
import { useAppStore } from '../lib/store';
import { useUpdateStore } from '../state/updateStore';
import { useActiveModel } from '../lib/useActiveModel';
import { ipc } from '../lib/ipc';
import { IconRefresh, IconSettings, ChevronRight, X, IconExpand } from '../lib/icons';
import { handleDragStart, closeWindow } from '../lib/window-chrome';

function formatRelativeTime(iso: string): string {
  const t = new Date(iso).getTime();
  if (!Number.isFinite(t)) return '';
  const diff = Date.now() - t;
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  return `${hours}h ago`;
}

function SwapToast() {
  const report = useAppStore((s) => s.pendingSwapReport);
  const email = useAppStore((s) =>
    report
      ? s.accounts.find((a) => a.slot === report.new_active_slot)?.email ?? null
      : null,
  );
  const consume = useAppStore((s) => s.consumeSwapReport);
  useEffect(() => {
    if (!report) return;
    const t = window.setTimeout(consume, 4000);
    return () => window.clearTimeout(t);
  }, [report, consume]);
  if (!report) return null;
  const cli = report.running.cli_processes;
  const code = report.running.vscode_with_extension.length;
  const hasRunning = cli > 0 || code > 0;
  return (
    <div className="absolute bottom-[40px] left-[var(--popover-pad)] right-[var(--popover-pad)] rounded-[var(--radius-sm)] bg-[var(--color-accent)] px-[var(--space-sm)] py-[var(--space-2xs)] text-[length:var(--text-micro)] text-white shadow-[0_4px_14px_rgba(0,0,0,0.18)]">
      <div className="truncate">
        ✓ Switched to {email ?? `slot ${report.new_active_slot}`}
      </div>
      {hasRunning && (
        <div className="opacity-85">
          {cli > 0 && `${cli} CLI session${cli > 1 ? 's' : ''}`}
          {cli > 0 && code > 0 && ' · '}
          {code > 0 && `${code} VS Code`}
          {' adopting in ~30s'}
        </div>
      )}
    </div>
  );
}

export function CompactPopover() {
  const usage = useAppStore((s) => s.usage);
  const thresholds = useAppStore((s) => s.settings?.thresholds ?? [75, 90]);
  const stale = useAppStore((s) => s.stale);
  const dismissBanner = useAppStore((s) => s.dismissBanner);
  const toggleViewMode = useAppStore((s) => s.toggleViewMode);
  const accountPlan = useAppStore(
    (s) => s.accounts.find((a) => a.is_active)?.subscription_type ?? null,
  );
  const liveSessions = useAppStore((s) => s.liveSessions);
  const activeModel = useActiveModel();
  const [view, setView] = useState<'home' | 'settings' | 'accounts'>('home');
  const [refreshing, setRefreshing] = useState(false);
  // Detail rows (Opus/Sonnet split, pay-as-you-go) collapse behind a
  // disclosure by default so the glance view is just the two hero numbers.
  const [detailsOpen, setDetailsOpen] = useState(false);
  const shownTick = useAppStore((s) => s.shownTick);

  // Keep the window height in sync with content: minimal only for the home
  // glance view with details collapsed; full compact height everywhere else
  // (settings/accounts need the room). Re-asserted on every popover show.
  const minimal = view === 'home' && !detailsOpen;
  useEffect(() => {
    ipc.resizeWindow(minimal ? 'compact-minimal' : 'compact').catch(() => {});
  }, [minimal, shownTick]);

  const accountEmail = usage?.account_email ?? null;
  const fetchedAt = usage?.snapshot.fetched_at;
  const updatedAgo = useMemo(
    () => (fetchedAt ? formatRelativeTime(fetchedAt) : ''),
    [fetchedAt],
  );

  async function handleRefresh() {
    if (refreshing) return;
    setRefreshing(true);
    try {
      await ipc.forceRefresh('active');
    } finally {
      setTimeout(() => setRefreshing(false), 420);
    }
  }

  if (view === 'settings') {
    return (
      <Shell>
        <Header title="Settings" onBack={() => setView('home')} />
        <div className="flex-1 overflow-y-auto px-[var(--popover-pad)] pb-[var(--space-md)] pt-[var(--space-xs)]">
          <SettingsPanel />
        </div>
      </Shell>
    );
  }

  if (view === 'accounts') {
    return (
      <Shell>
        <AccountsPanel onBack={() => setView('home')} />
      </Shell>
    );
  }

  if (!usage) {
    return <LoadingShell refreshing={refreshing} onRefresh={handleRefresh} onSettings={() => setView('settings')} />;
  }

  const warn = thresholds[0] ?? 75;
  const danger = thresholds[1] ?? 90;

  return (
    <Shell minimal={minimal}>
      <UpdateBanner />
      <ChromeBar
        live
        stale={stale}
        refreshing={refreshing}
        accountEmail={accountEmail}
        accountPlan={accountPlan}
        onRefresh={handleRefresh}
        onSettings={() => setView('settings')}
        onAccounts={() => setView('accounts')}
        onToggleView={toggleViewMode}
      />

      {/* Banners */}
      <div className="flex flex-col gap-[6px] px-[var(--popover-pad)]">
        <UnmanagedActiveBanner />
        {stale && (
          <Banner variant="stale" onDismiss={() => dismissBanner('stale')}>
            Data may be stale.
          </Banner>
        )}
      </div>

      <UsageSummary
        usage={usage}
        thresholds={[warn, danger]}
        activeModel={activeModel}
        collapsible
        detailsOpen={detailsOpen}
        onToggleDetails={() => setDetailsOpen((v) => !v)}
      />

      <NowRunningSection sessions={liveSessions} />

      <div
        style={{ marginTop: 'auto' }}
        className="flex items-center justify-between gap-2 px-[var(--popover-pad)] py-[var(--space-sm)] border-t border-[var(--color-rule)]"
      >
        <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          Updated {updatedAgo || '—'}
        </span>
        <VersionFooter />
      </div>
    </Shell>
  );
}


function LoadingShell({
  refreshing,
  onRefresh,
  onSettings,
}: {
  refreshing: boolean;
  onRefresh: () => void;
  onSettings: () => void;
}) {
  const refreshUsage = useAppStore((s) => s.refreshUsage);
  const [hint, setHint] = useState(false);

  useEffect(() => {
    ipc.forceRefresh('active').catch(() => { });

    const tick = setInterval(() => {
      refreshUsage().catch(() => { });
    }, 1000);

    const hintTimer = setTimeout(() => setHint(true), 3000);

    return () => {
      clearInterval(tick);
      clearTimeout(hintTimer);
    };
  }, [refreshUsage]);

  return (
    <Shell>
      <ChromeBar
        live={false}
        stale={false}
        refreshing={refreshing}
        accountEmail={null}
        onRefresh={onRefresh}
        onSettings={onSettings}
        onAccounts={() => { }}
      />
      <div className="flex flex-1 flex-col items-center justify-center gap-[var(--space-sm)] px-[var(--popover-pad)] text-center">
        <span className="text-[length:var(--text-label)] text-[color:var(--color-text-muted)]">
          Loading usage…
        </span>
        {hint && (
          <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] opacity-70">
            Taking longer than expected — tap the refresh icon.
          </span>
        )}
      </div>
    </Shell>
  );
}

function Shell({ children, minimal }: { children: React.ReactNode; minimal?: boolean }) {
  return (
    <div
      className={`relative flex h-full w-full flex-col ${minimal ? 'popover-minimal' : ''}`}
      style={{
        width: 'var(--popover-width)',
        height: minimal ? 'var(--popover-height-minimal)' : 'var(--popover-height)',
      }}
    >
      {children}
      <SwapToast />
    </div>
  );
}

function ChromeBar({
  live,
  stale,
  refreshing,
  accountEmail,
  accountPlan,
  onRefresh,
  onSettings,
  onAccounts,
  onToggleView,
}: {
  live: boolean;
  stale: boolean;
  refreshing: boolean;
  accountEmail: string | null;
  accountPlan?: string | null;
  onRefresh: () => void;
  onSettings: () => void;
  onAccounts: () => void;
  onToggleView?: () => void;
}) {
  return (
    <div
      onPointerDown={handleDragStart}
      className="flex items-center justify-between gap-[var(--space-sm)] px-[var(--popover-pad)] pt-[var(--space-md)] pb-[var(--space-sm)] cursor-default select-none"
    >
      <div className="flex items-center gap-[var(--space-xs)] min-w-0">
        <button
          type="button"
          onClick={onAccounts}
          className="text-[length:var(--text-body)] font-[var(--weight-medium)] text-[color:var(--color-text)] hover:text-[color:var(--color-accent)] truncate max-w-[220px]"
          title={accountEmail ?? ''}
        >
          {accountEmail ?? 'Sign in'}
        </button>
        {accountPlan && <PlanPill plan={accountPlan} />}
        <StatusDot live={live} stale={stale} />
      </div>
      <div className="flex items-center gap-[2px]">
        <IconButton label="Refresh" onClick={onRefresh}>
          <motion.span
            animate={refreshing ? { rotate: 360 } : { rotate: 0 }}
            transition={
              refreshing
                ? { duration: 0.7, ease: 'linear', repeat: Infinity }
                : { duration: 0.2 }
            }
            style={{ display: 'inline-flex' }}
          >
            <IconRefresh size={13} />
          </motion.span>
        </IconButton>
        <IconButton label="Settings" onClick={onSettings}>
          <IconSettings size={13} />
        </IconButton>
        {onToggleView && (
          <IconButton label="Expand details" onClick={onToggleView}>
            <IconExpand size={13} />
          </IconButton>
        )}
        <IconButton label="Close" onClick={closeWindow}>
          <X size={13} />
        </IconButton>
      </div>
    </div>
  );
}

function Header({ title, onBack }: { title: string; onBack: () => void }) {
  return (
    <div
      onPointerDown={handleDragStart}
      className="flex items-center justify-between gap-[var(--space-sm)] px-[var(--popover-pad)] pt-[var(--space-md)] pb-[var(--space-sm)] cursor-default select-none"
    >
      <button
        type="button"
        onClick={onBack}
        className="
          inline-flex items-center gap-[var(--space-2xs)]
          text-[length:var(--text-label)] text-[color:var(--color-text-secondary)] tracking-[var(--tracking-label)] uppercase
          transition-colors duration-[var(--duration-fast)]
          hover:text-[color:var(--color-text)]
          focus-visible:outline-2 focus-visible:outline-[var(--color-border-focus)] focus-visible:outline-offset-2 rounded
        "
      >
        <ChevronRight size={11} className="rotate-180" />
        Back
      </button>
      <span className="text-[length:var(--text-label)] font-[var(--weight-semibold)] text-[color:var(--color-text-secondary)] tracking-[var(--tracking-label)] uppercase">
        {title}
      </span>
      <IconButton label="Close" onClick={closeWindow}>
        <X size={13} />
      </IconButton>
    </div>
  );
}

function PlanPill({ plan }: { plan: string }) {
  const isMax = plan.toLowerCase().includes('max');
  return (
    <span
      className={`
        inline-flex items-center rounded-[var(--radius-pill)]
        px-[var(--space-xs)] py-[1px] shrink-0
        text-[length:var(--text-micro)] font-[var(--weight-semibold)]
        uppercase tracking-[var(--tracking-label)]
        ${isMax
          ? 'bg-[var(--color-accent-dim)] text-[color:var(--color-accent)]'
          : 'bg-[var(--color-track)] text-[color:var(--color-text-secondary)]'}
      `}
    >
      {plan}
    </span>
  );
}

function StatusDot({ live, stale }: { live: boolean; stale: boolean }) {
  if (stale) {
    return (
      <span
        title="Stale"
        className="inline-block h-[6px] w-[6px] rounded-full"
        style={{ background: 'var(--color-warn)' }}
      />
    );
  }
  if (!live) {
    return (
      <span
        title="Offline"
        className="inline-block h-[6px] w-[6px] rounded-full ring-1 ring-[var(--color-rule-strong)]"
      />
    );
  }
  return (
    <span className="relative inline-flex h-[6px] w-[6px] items-center justify-center" title="Live">
      <span
        className="absolute inline-block h-[6px] w-[6px] rounded-full opacity-60"
        style={{ background: 'var(--color-accent)', animation: 'pulse-dot 2.4s ease-in-out infinite' }}
      />
      <span
        className="relative inline-block h-[6px] w-[6px] rounded-full"
        style={{ background: 'var(--color-accent)' }}
      />
    </span>
  );
}

function VersionFooter() {
  const status = useUpdateStore((s) => s.status);
  const [transient, setTransient] = useState<null | 'checking' | 'up-to-date' | 'failed'>(null);

  useEffect(() => {
    if (status === 'checking') {
      setTransient('checking');
      return;
    }
    if (status === 'up-to-date') {
      setTransient('up-to-date');
      const t = setTimeout(() => setTransient(null), 3000);
      return () => clearTimeout(t);
    }
    if (status === 'failed') {
      setTransient('failed');
      const t = setTimeout(() => setTransient(null), 3000);
      return () => clearTimeout(t);
    }
    if (status === 'available' || status === 'downloading') {
      setTransient('checking');
      return;
    }
    setTransient(null);
  }, [status]);

  const label =
    transient === 'checking' ? 'Checking…'
      : transient === 'up-to-date' ? 'Up to date'
        : transient === 'failed' ? "Couldn't check"
          : 'Check for updates';

  const isChecking = transient === 'checking';

  const onClick = () => {
    if (isChecking) return;
    invoke('check_for_updates_now').catch(() => {/* error arrives via event */ });
  };

  return (
    <span className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)] select-none">
      v{__APP_VERSION__}{' · '}
      <button
        type="button"
        onClick={onClick}
        disabled={isChecking}
        className="underline-offset-2 hover:underline hover:text-[color:var(--color-accent)] transition-colors disabled:opacity-60"
      >
        {label}
      </button>
    </span>
  );
}
