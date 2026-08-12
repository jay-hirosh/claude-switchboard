import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useAppStore } from '../lib/store';
import { ipc } from '../lib/ipc';
import { computeBestAccountUuid } from './bestAccount';
import type {
  AccountListEntry,
  RunningClaudeCode,
} from '../lib/generated/bindings';

interface PendingSwap {
  target: AccountListEntry;
  running: RunningClaudeCode;
}

export function useAccountManagement() {
  const accounts = useAppStore((s) => s.accounts);
  const refreshAccounts = useAppStore((s) => s.refreshAccounts);
  const setPendingSwapReport = useAppStore((s) => s.setPendingSwapReport);

  const [chooserOpen, setChooserOpen] = useState(false);
  const [pending, setPending] = useState<PendingSwap | null>(null);
  const [swappingSlot, setSwappingSlot] = useState<number | null>(null);
  const [confirmError, setConfirmError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [reauthSlot, setReauthSlot] = useState<number | null>(null);

  // Stable-listener pattern: the listener mounts once when *any* reauth
  // becomes pending and stays mounted until none are. The dep is the
  // boolean `reauthPending`, not `reauthSlot` itself — switching slots
  // (e.g. user starts slot 2, then quickly slot 3) does NOT re-mount the
  // listener and so cannot race the pending listen() promise. The handler
  // reads the current slot from a ref.
  const reauthSlotRef = useRef<number | null>(null);
  useEffect(() => { reauthSlotRef.current = reauthSlot; }, [reauthSlot]);

  const reauthPending = reauthSlot !== null;
  useEffect(() => {
    if (!reauthPending) return;
    let unlistenComplete: (() => void) | undefined;
    let unlistenError: (() => void) | undefined;
    listen<number>('oauth_complete', () => {
      if (reauthSlotRef.current !== null) {
        refreshAccounts().catch(() => {});
        setReauthSlot(null);
      }
    }).then((f) => { unlistenComplete = f; });
    listen<string>('oauth_error', () => {
      if (reauthSlotRef.current !== null) setReauthSlot(null);
    }).then((f) => { unlistenError = f; });
    return () => {
      unlistenComplete?.();
      unlistenError?.();
    };
  }, [reauthPending, refreshAccounts]);

  const orgGroups = useMemo(() => {
    const map = new Map<string, AccountListEntry>();
    for (const a of accounts) {
      if (a.org_uuid && !map.has(a.org_uuid)) map.set(a.org_uuid, a);
    }
    return map;
  }, [accounts]);

  const currentActive = useMemo(
    () => accounts.find((a) => a.is_active) ?? null,
    [accounts],
  );

  const bestAccountUuid = useMemo(() => computeBestAccountUuid(accounts), [accounts]);

  const openChooser = useCallback(() => setChooserOpen(true), []);
  const closeChooser = useCallback(() => setChooserOpen(false), []);

  const requestSwap = useCallback(
    async (entry: AccountListEntry) => {
      if (entry.is_active || swappingSlot !== null) return;
      setConfirmError(null);
      let running: RunningClaudeCode = { cli_processes: 0, vscode_with_extension: [] };
      try {
        running = await ipc.detectRunningClaudeCode();
      } catch {
        // Best-effort detection.
      }
      setPending({ target: entry, running });
    },
    [swappingSlot],
  );

  const confirmSwap = useCallback(async () => {
    if (!pending || swappingSlot !== null) return;
    setConfirmError(null);
    setSwappingSlot(pending.target.slot);
    try {
      const report = await ipc.swapToAccount(pending.target.slot);
      setPendingSwapReport(report);
      await refreshAccounts();
      setPending(null);
    } catch (e) {
      setConfirmError(e instanceof Error ? e.message : 'Swap failed');
    } finally {
      setSwappingSlot(null);
    }
  }, [pending, swappingSlot, refreshAccounts, setPendingSwapReport]);

  const cancelSwap = useCallback(() => {
    setPending(null);
    setConfirmError(null);
  }, []);

  const handleReauth = useCallback(async (entry: AccountListEntry) => {
    if (reauthSlot !== null) return;
    setReauthSlot(entry.slot);
    try {
      const url = await ipc.startOauthFlow(false);
      await openUrl(url);
    } catch {
      setReauthSlot(null);
    }
  }, [reauthSlot]);

  const handleRemove = useCallback(async (entry: AccountListEntry) => {
    await ipc.removeAccount(entry.slot);
    await refreshAccounts();
  }, [refreshAccounts]);

  const handleRefreshAll = useCallback(async () => {
    if (refreshing) return;
    setRefreshing(true);
    // Rows live-update individually as each slot's `usage_updated` event
    // lands, so the spinner only needs to outlast the wait for the FIRST
    // fresh result — not the whole staggered round. (This used to spin
    // (n−1)×30s+2s, which read as "stuck".) A 10s cap bounds the spin when
    // every slot is in 429 backoff and no event ever arrives.
    const cap = setTimeout(() => setRefreshing(false), 10_000);
    let unlisten: (() => void) | undefined;
    let stopped = false;
    const stop = () => {
      if (stopped) return;
      stopped = true;
      clearTimeout(cap);
      unlisten?.();
      setRefreshing(false);
    };
    unlisten = await listen('usage_updated', stop).catch(() => undefined);
    try {
      await ipc.forceRefresh('all');
    } catch {
      // Loop logs failures.
      stop();
    }
  }, [refreshing]);

  return {
    accounts,
    currentActive,
    bestAccountUuid,
    orgGroups,
    pending,
    swappingSlot,
    confirmError,
    refreshing,
    reauthSlot,
    chooserOpen,
    requestSwap,
    confirmSwap,
    cancelSwap,
    handleReauth,
    handleRemove,
    handleRefreshAll,
    openChooser,
    closeChooser,
  };
}
