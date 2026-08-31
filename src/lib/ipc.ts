import { commands, type Result } from './generated/bindings';
import type { Settings } from './types';

async function unwrap<T>(r: Result<T, string>): Promise<T> {
  if (r.status === 'error') throw new Error(r.error);
  return r.data;
}

export const ipc = {
  getCurrentUsage: () => commands.getCurrentUsage().then(unwrap),
  getDeviceId: () => commands.getDeviceId().then(unwrap),
  getPricing: () => commands.getPricing().then(unwrap),
  getSessionHistory: (days: number, localOnly: boolean) =>
    commands.getSessionHistory(days, localOnly).then(unwrap),
  getLiveSessions: () => commands.getLiveSessions().then(unwrap),
  getCompactions: (days: number, localOnly: boolean) =>
    commands.getCompactions(days, localOnly).then(unwrap),
  getDailyTrends: (days: number, localOnly: boolean) =>
    commands.getDailyTrends(days, localOnly).then(unwrap),
  getDailyPattern: (days: number, localOnly: boolean) =>
    commands.getDailyPattern(days, localOnly).then(unwrap),
  getTodayPattern: (localOnly: boolean) => commands.getTodayPattern(localOnly).then(unwrap),
  getYesterdayPattern: (localOnly: boolean) => commands.getYesterdayPattern(localOnly).then(unwrap),
  getWeekPattern: (localOnly: boolean) => commands.getWeekPattern(localOnly).then(unwrap),
  getDatePattern: (date: string, localOnly: boolean) =>
    commands.getDatePattern(date, localOnly).then(unwrap),
  exportTrendsCsv: (path: string, days: number, localOnly: boolean) =>
    commands.exportTrendsCsv(path, days, localOnly).then(unwrap),
  getLimitHitHistory: (days: number) => commands.getLimitHitHistory(days).then(unwrap),
  getModelBreakdown: (days: number, localOnly: boolean) =>
    commands.getModelBreakdown(days, localOnly).then(unwrap),
  getDailyModelBreakdown: (days: number, localOnly: boolean) =>
    commands.getDailyModelBreakdown(days, localOnly).then(unwrap),
  getProjectBreakdown: (days: number, localOnly: boolean) =>
    commands.getProjectBreakdown(days, localOnly).then(unwrap),
  getRepoBreakdown: () => commands.getRepoBreakdown().then(unwrap),
  getTodayRepoBreakdown: (localOnly: boolean) => commands.getTodayRepoBreakdown(localOnly).then(unwrap),
  getYesterdayRepoBreakdown: (localOnly: boolean) =>
    commands.getYesterdayRepoBreakdown(localOnly).then(unwrap),
  getWeekRepoBreakdown: (localOnly: boolean) => commands.getWeekRepoBreakdown(localOnly).then(unwrap),
  getDateRepoBreakdown: (date: string, localOnly: boolean) =>
    commands.getDateRepoBreakdown(date, localOnly).then(unwrap),
  getCacheStats: (days: number, localOnly: boolean) => commands.getCacheStats(days, localOnly).then(unwrap),
  getTodayCacheStats: (localOnly: boolean) => commands.getTodayCacheStats(localOnly).then(unwrap),
  getYesterdayCacheStats: (localOnly: boolean) => commands.getYesterdayCacheStats(localOnly).then(unwrap),
  getWeekCacheStats: (localOnly: boolean) => commands.getWeekCacheStats(localOnly).then(unwrap),
  getDateCacheStats: (date: string, localOnly: boolean) =>
    commands.getDateCacheStats(date, localOnly).then(unwrap),
  getDailyAccountBreakdown: (days: number, localOnly: boolean) =>
    commands.getDailyAccountBreakdown(days, localOnly).then(unwrap),
  getCacheStatsByAccount: (days: number, localOnly: boolean) =>
    commands.getCacheStatsByAccount(days, localOnly).then(unwrap),

  startOauthFlow: (longLived: boolean = false) =>
    commands.startOauthFlow(longLived).then(unwrap),
  hasClaudeCodeCreds: () => commands.hasClaudeCodeCreds().then(unwrap),

  listAccounts: () => commands.listAccounts().then(unwrap),
  addAccountFromClaudeCode: () => commands.addAccountFromClaudeCode().then(unwrap),
  removeAccount: (slot: number) => commands.removeAccount(slot).then(unwrap),
  swapToAccount: (slot: number) => commands.swapToAccount(slot).then(unwrap),
  detectRunningClaudeCode: () => commands.detectRunningClaudeCode().then(unwrap),
  refreshAccount: (slot: number) => commands.refreshAccount(slot).then(unwrap),

  getSettings: () => commands.getSettings().then(unwrap),
  updateSettings: (s: Settings) => commands.updateSettings(s).then(unwrap),

  resizeWindow: (mode: 'compact' | 'compact-minimal' | 'expanded', extraHeight = 0) =>
    commands.resizeWindow(mode, extraHeight).then(unwrap),
  toggleFullscreen: () => commands.toggleFullscreen().then(unwrap),
  printWebview: () => commands.printWebview().then(unwrap),
  forceRefresh: (scope: 'active' | 'all') => commands.forceRefresh(scope).then(unwrap),

  // Warmup pillar
  getWarmupState: (accountId: string) => commands.getWarmupState(accountId).then(unwrap),
  getWarmupSuggestion: () => commands.getWarmupSuggestion().then(unwrap),
  getStatuslineInstallState: () => commands.getStatuslineInstallState().then(unwrap),
  installStatusline: (force: boolean) => commands.installStatusline(force).then(unwrap),
  uninstallStatusline: () => commands.uninstallStatusline().then(unwrap),
  setWarmupEnabled: (accountId: string, enabled: boolean) =>
    commands.setWarmupEnabled(accountId, enabled).then(unwrap),
  setAccountSchedule: (accountId: string, schedule: import('./generated/bindings').Schedule) =>
    commands.setAccountSchedule(accountId, schedule).then(unwrap),
  warmupAccountNow: (accountId: string) => commands.warmupAccountNow(accountId).then(unwrap),
  grantWarmupConsent: () => commands.grantWarmupConsent().then(unwrap),
  revokeWarmupConsent: () => commands.revokeWarmupConsent().then(unwrap),
  getWarmupConsentGranted: () => commands.getWarmupConsentGranted().then(unwrap),
  osSchedulerRegister: () => commands.osSchedulerRegister().then(unwrap),
  osSchedulerUnregister: () => commands.osSchedulerUnregister().then(unwrap),
  osSchedulerIsRegistered: () => commands.osSchedulerIsRegistered().then(unwrap),

  // Providers pillar
  listProviders: () => commands.listProviders().then(unwrap),
  upsertProvider: (p: import('./generated/bindings').Provider) =>
    commands.upsertProvider(p).then(unwrap),
  deleteProvider: (id: string) => commands.deleteProvider(id).then(unwrap),
  listProviderPresets: () => commands.listProviderPresets().then(unwrap),
  listOllamaModels: (baseUrl: string) => commands.listOllamaModels(baseUrl).then(unwrap),
  listAvailableTerminals: () => commands.listAvailableTerminals().then(unwrap),
  launchProviderSession: (
    providerId: string,
    cwd: string,
    terminal: import('./generated/bindings').Terminal,
    resumeSessionId: string | null = null,
    permissionMode: string | null = null,
    /** Omitted means a terminal, matching every call written before tabs existed. */
    surface: import('./generated/bindings').LaunchSurface | null = null,
  ) =>
    commands
      .launchProviderSession(
        providerId,
        cwd,
        terminal,
        resumeSessionId,
        permissionMode,
        surface,
      )
      .then(unwrap),
  /** Both halves present: the `code` CLI, and the Claude Code extension. */
  vscodeTabAvailable: () => commands.vscodeTabAvailable().then(unwrap),
  getProviderLaunchCommand: (
    providerId: string,
    cwd: string,
    terminal: import('./generated/bindings').Terminal,
  ) => commands.getProviderLaunchCommand(providerId, cwd, terminal).then(unwrap),
  getDefaultProvider: () => commands.getDefaultProvider().then(unwrap),
  setDefaultProvider: (providerId: string, force: boolean) =>
    commands.setDefaultProvider(providerId, force).then(unwrap),
  /** Resolves to the keys left untouched because the user edited them (spec §4.2). */
  clearDefaultProvider: () => commands.clearDefaultProvider().then(unwrap),

  // Session browser
  listResumableSessions: () => commands.listResumableSessions().then(unwrap),

  // Sync
  getSyncStatus: () => commands.getSyncStatus().then(unwrap),
  getSyncBackendUrl: () => commands.getSyncBackendUrl().then(unwrap),
  setSyncBackendUrl: (url: string) => commands.setSyncBackendUrl(url).then(unwrap),
  bootstrapSyncAccount: (deviceName: string) => commands.bootstrapSyncAccount(deviceName).then(unwrap),
  generatePairingCode: () => commands.generatePairingCode().then(unwrap),
  joinSyncAccount: (pairingCode: string, deviceName: string) =>
    commands.joinSyncAccount(pairingCode, deviceName).then(unwrap),
  syncNow: () => commands.syncNow().then(unwrap),
};
