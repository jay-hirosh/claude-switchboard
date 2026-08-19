import { create } from 'zustand';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { UnlistenFn } from '@tauri-apps/api/event';
import { ipc } from './ipc';
import { subscribe, type AppEvent } from './events';
import type { AccountListEntry, CachedUsage, LiveSessionInfo, Settings, SwapReport } from './generated/bindings';

let _unlisteners: UnlistenFn[] = [];

interface AccountAuthState {
  failingSlots: Set<number>;
  dismissedUnmanaged: Set<string>;
}

interface AppStore {
  usage: CachedUsage | null;
  settings: Settings | null;
  accounts: AccountListEntry[];
  activeSlot: number | null;
  unmanagedActive: { email: string; account_uuid: string } | null;
  authState: AccountAuthState;
  requiresSetup: boolean;
  stale: boolean;
  dbReset: boolean;
  sessionDataVersion: number;
  liveSessions: LiveSessionInfo[];
  viewMode: 'compact' | 'expanded';
  /** Native OS fullscreen state of the expanded window. Re-synced from
   *  resize events (see setFullscreen) since the OS can exit fullscreen
   *  on its own, without going through toggleFullscreen(). */
  isFullscreen: boolean;
  /** Bumped on every popover_shown so views can re-assert size/state. */
  shownTick: number;
  pendingSwapReport: SwapReport | null;
  modalStack: string[];

  init: () => Promise<void>;
  cleanup: () => void;
  refreshSettings: () => Promise<void>;
  setSettings: (s: Settings) => Promise<void>;
  refreshUsage: () => Promise<void>;
  refreshAccounts: () => Promise<void>;
  dismissBanner: (
    kind: 'requiresSetup' | 'stale' | 'dbReset' | 'unmanagedActive',
  ) => void;
  toggleViewMode: () => void;
  toggleFullscreen: () => Promise<void>;
  setFullscreen: (fullscreen: boolean) => void;
  setPendingSwapReport: (report: SwapReport) => void;
  consumeSwapReport: () => void;
  pushModal: (id: string) => void;
  popModal: (id: string) => void;
  isTopmost: (id: string) => boolean;
  resetModalStack: () => void;
}

export const useAppStore = create<AppStore>((set, _get) => ({
  usage: null,
  settings: null,
  accounts: [],
  activeSlot: null,
  unmanagedActive: null,
  authState: { failingSlots: new Set(), dismissedUnmanaged: new Set() },
  requiresSetup: false,
  stale: false,
  dbReset: false,
  sessionDataVersion: 0,
  liveSessions: [],
  viewMode: 'compact',
  isFullscreen: false,
  shownTick: 0,
  pendingSwapReport: null,
  modalStack: [],

  async init() {
    if (_unlisteners.length > 0) {
      _unlisteners.forEach((fn) => fn());
      _unlisteners = [];
    }

    const [usage, settings, accounts, liveSessions] = await Promise.all([
      ipc.getCurrentUsage(),
      ipc.getSettings(),
      ipc.listAccounts().catch(() => []),
      ipc.getLiveSessions().catch(() => []),
    ]);
    const active = accounts.find((a) => a.is_active)?.slot ?? null;
    set({ usage, settings, accounts, activeSlot: active, liveSessions });

    _unlisteners = await subscribe((e: AppEvent) => {
      switch (e.type) {
        case 'usage_updated': {
          const { slot, cached } = e.payload;
          set((s) => {
            const next = s.accounts.map((a) =>
              a.slot === slot
                ? { ...a, cached_usage: cached, last_error: cached.last_error }
                : a,
            );
            const isActive = s.activeSlot === slot;
            return {
              accounts: next,
              usage: isActive ? cached : s.usage,
              stale: isActive ? cached.last_error != null : s.stale,
            };
          });
          break;
        }
        case 'accounts_changed':
          set({
            accounts: e.payload,
            activeSlot: e.payload.find((a) => a.is_active)?.slot ?? null,
          });
          break;
        case 'auth_required_for_slot':
          set((s) => {
            const failing = new Set(s.authState.failingSlots);
            failing.add(e.payload.slot);
            return {
              authState: { ...s.authState, failingSlots: failing },
            };
          });
          break;
        case 'unmanaged_active_account':
          set((s) =>
            s.authState.dismissedUnmanaged.has(e.payload.account_uuid)
              ? {}
              : { unmanagedActive: e.payload },
          );
          break;
        case 'requires_setup':
          set({ requiresSetup: true });
          break;
        case 'migrated_accounts':
          ipc.listAccounts().then((accounts) => {
            set({
              accounts,
              activeSlot: accounts.find((a) => a.is_active)?.slot ?? null,
            });
          });
          break;
        case 'swap_completed':
          set({ pendingSwapReport: e.payload });
          ipc.listAccounts().then((accounts) => {
            set({
              accounts,
              activeSlot: accounts.find((a) => a.is_active)?.slot ?? null,
            });
          });
          break;
        case 'session_ingested':
          set((s) => ({ sessionDataVersion: s.sessionDataVersion + 1 }));
          break;
        case 'live_sessions_changed':
          set({ liveSessions: e.payload });
          break;
        case 'stale_data':
          set({ stale: true });
          break;
        case 'db_reset':
          set({ dbReset: true });
          break;
        case 'watcher_error':
          console.error('[watcher_error]', e.payload);
          break;
        case 'popover_hidden':
          set({ viewMode: 'compact', isFullscreen: false });
          ipc.resizeWindow('compact').catch(() => {});
          break;
        case 'popover_shown':
          set((s) => ({ shownTick: s.shownTick + 1 }));
          document.body.dataset.appearing = 'true';
          window.setTimeout(() => {
            delete document.body.dataset.appearing;
          }, 240);
          break;
      }
    });

    try {
      const win = getCurrentWindow();
      const focusUnlisten = await win.onFocusChanged(({ payload: focused }) => {
        if (!focused) return;
        ipc.getCurrentUsage().then((u) => {
          if (u) set({ usage: u, stale: false });
        }).catch(() => {});
      });
      _unlisteners.push(focusUnlisten);
    } catch {
      // Outside Tauri.
    }
  },

  cleanup() {
    _unlisteners.forEach((fn) => fn());
    _unlisteners = [];
  },

  async refreshSettings() {
    const s = await ipc.getSettings();
    set({ settings: s });
  },

  async setSettings(s) {
    await ipc.updateSettings(s);
    set({ settings: s });
  },

  async refreshUsage() {
    const u = await ipc.getCurrentUsage();
    if (u) set({ usage: u, stale: false });
  },

  async refreshAccounts() {
    const accounts = await ipc.listAccounts();
    set({
      accounts,
      activeSlot: accounts.find((a) => a.is_active)?.slot ?? null,
    });
  },

  dismissBanner(kind) {
    switch (kind) {
      case 'requiresSetup':
        set({ requiresSetup: false });
        break;
      case 'stale':
        set({ stale: false });
        break;
      case 'dbReset':
        set({ dbReset: false });
        break;
      case 'unmanagedActive':
        set((s) => {
          if (!s.unmanagedActive) return {};
          const dismissed = new Set(s.authState.dismissedUnmanaged);
          dismissed.add(s.unmanagedActive.account_uuid);
          return {
            unmanagedActive: null,
            authState: { ...s.authState, dismissedUnmanaged: dismissed },
          };
        });
        break;
    }
  },

  toggleViewMode() {
    const next = _get().viewMode === 'compact' ? 'expanded' : 'compact';
    set({ viewMode: next });
    ipc.resizeWindow(next).catch(() => {});
  },

  async toggleFullscreen() {
    const next = await ipc.toggleFullscreen();
    set({ isFullscreen: next });
  },

  setFullscreen(fullscreen) {
    set({ isFullscreen: fullscreen });
  },

  setPendingSwapReport(report) {
    set({ pendingSwapReport: report });
  },
  consumeSwapReport() {
    set({ pendingSwapReport: null });
  },

  pushModal(id) {
    set((s) => (s.modalStack.includes(id) ? {} : { modalStack: [...s.modalStack, id] }));
  },
  popModal(id) {
    set((s) => {
      const i = s.modalStack.lastIndexOf(id);
      if (i === -1) return {};
      return { modalStack: [...s.modalStack.slice(0, i), ...s.modalStack.slice(i + 1)] };
    });
  },
  isTopmost(id) {
    const stack = _get().modalStack;
    return stack.length > 0 && stack[stack.length - 1] === id;
  },
  resetModalStack() {
    set({ modalStack: [] });
  },
}));
