import { commands, type Result } from './generated/bindings';
import type { Settings } from './types';

async function unwrap<T>(r: Result<T, string>): Promise<T> {
  if (r.status === 'error') throw new Error(r.error);
  return r.data;
}

export const ipc = {
  getCurrentUsage: () => commands.getCurrentUsage().then(unwrap),
  getPricing: () => commands.getPricing().then(unwrap),
  getSessionHistory: (days: number) => commands.getSessionHistory(days).then(unwrap),
  getCompactions: (days: number) => commands.getCompactions(days).then(unwrap),
  getDailyTrends: (days: number) => commands.getDailyTrends(days).then(unwrap),
  getModelBreakdown: (days: number) => commands.getModelBreakdown(days).then(unwrap),
  getDailyModelBreakdown: (days: number) => commands.getDailyModelBreakdown(days).then(unwrap),
  getProjectBreakdown: (days: number) => commands.getProjectBreakdown(days).then(unwrap),
  getCacheStats: (days: number) => commands.getCacheStats(days).then(unwrap),

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

  resizeWindow: (mode: 'compact' | 'compact-minimal' | 'expanded') => commands.resizeWindow(mode).then(unwrap),
  forceRefresh: (scope: 'active' | 'all') => commands.forceRefresh(scope).then(unwrap),

  // Warmup pillar
  getWarmupState: (accountId: string) => commands.getWarmupState(accountId).then(unwrap),
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
  listAvailableTerminals: () => commands.listAvailableTerminals().then(unwrap),
  launchProviderSession: (
    providerId: string,
    cwd: string,
    terminal: import('./generated/bindings').Terminal,
    resumeSessionId: string | null = null,
  ) => commands.launchProviderSession(providerId, cwd, terminal, resumeSessionId).then(unwrap),
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
};
