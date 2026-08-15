// Re-export all generated types. This file is the public surface for the rest
// of the frontend; the authoritative definitions live in generated/bindings.ts.
export type {
  AccountCacheStats,
  AccountStats,
  AuthSource,
  BurnRateProjection,
  CacheStats,
  CachedUsage,
  DailyAccountBucket,
  DailyBucket,
  DailyModelBucket,
  ExtraBurnRate,
  ExtraUsage,
  LiveSessionInfo,
  ModelAccountShare,
  ModelStats,
  PricingEntry,
  PricingTier,
  ProjectStats,
  RepoProjectStats,
  RepoStats,
  Settings,
  UsageSnapshot,
  Utilization,
} from './generated/bindings';

// StoredSessionEvent is the Rust name. Re-export as SessionEvent for
// backwards-compat with existing imports throughout the frontend.
export type { StoredSessionEvent as SessionEvent } from './generated/bindings';
export type { StoredCompaction as Compaction } from './generated/bindings';

// Frontend-only types — no Rust equivalent, not in generated bindings.
export interface HeatmapCell {
  date: string;
  value: number;
  level: 0 | 1 | 2 | 3 | 4;
}
