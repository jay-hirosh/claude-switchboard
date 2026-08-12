import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AccountListEntry,
  CachedUsage,
  LiveSessionInfo,
  SwapReport,
} from "./generated/bindings";

export type AppEvent =
  | { type: "usage_updated"; payload: { slot: number; cached: CachedUsage } }
  | { type: "session_ingested"; payload: number }
  | {
      type: "auth_required_for_slot";
      payload: { slot: number; email: string };
    }
  | {
      type: "unmanaged_active_account";
      payload: { email: string; account_uuid: string };
    }
  | { type: "requires_setup" }
  | { type: "migrated_accounts"; payload: number[] }
  | { type: "swap_completed"; payload: SwapReport }
  | { type: "accounts_changed"; payload: AccountListEntry[] }
  | { type: "stale_data" }
  | { type: "db_reset" }
  | { type: "watcher_error"; payload: string }
  | { type: "popover_hidden" }
  | { type: "popover_shown" }
  | { type: "live_sessions_changed"; payload: LiveSessionInfo[] };

export function subscribe(
  handler: (e: AppEvent) => void,
): Promise<UnlistenFn[]> {
  return Promise.all([
    listen<{ slot: number; cached: CachedUsage }>("usage_updated", (e) =>
      handler({ type: "usage_updated", payload: e.payload }),
    ),
    listen<number>("session_ingested", (e) =>
      handler({ type: "session_ingested", payload: e.payload }),
    ),
    listen<{ slot: number; email: string }>(
      "auth_required_for_slot",
      (e) => handler({ type: "auth_required_for_slot", payload: e.payload }),
    ),
    listen<{ email: string; account_uuid: string }>(
      "unmanaged_active_account",
      (e) =>
        handler({ type: "unmanaged_active_account", payload: e.payload }),
    ),
    listen("requires_setup", () => handler({ type: "requires_setup" })),
    listen<number[]>("migrated_accounts", (e) =>
      handler({ type: "migrated_accounts", payload: e.payload }),
    ),
    listen<SwapReport>("swap_completed", (e) =>
      handler({ type: "swap_completed", payload: e.payload }),
    ),
    listen<AccountListEntry[]>("accounts_changed", (e) =>
      handler({ type: "accounts_changed", payload: e.payload }),
    ),
    listen("stale_data", () => handler({ type: "stale_data" })),
    listen("db_reset", () => handler({ type: "db_reset" })),
    listen<string>("watcher_error", (e) =>
      handler({ type: "watcher_error", payload: e.payload }),
    ),
    listen("popover_hidden", () => handler({ type: "popover_hidden" })),
    listen("popover_shown", () => handler({ type: "popover_shown" })),
    listen<LiveSessionInfo[]>("live_sessions_changed", (e) =>
      handler({ type: "live_sessions_changed", payload: e.payload }),
    ),
  ]);
}
