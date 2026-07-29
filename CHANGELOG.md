# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Custom model providers.** Run Claude Code against third-party Anthropic-compatible endpoints (GLM, Kimi, DeepSeek, MiniMax, OpenRouter, or a custom URL) by launching provider-scoped terminal sessions from the new Providers tab. Sessions use per-process environment variables, so several providers can run at once and your own launch scripts keep working.
- **Optional global default.** A provider can additionally be set as the default for bare `claude` invocations. This writes `~/.claude/settings.json` and is guarded by a backup, an undo manifest and a confirmation prompt — it overrides `ANTHROPIC_*` variables exported by your shell.
- **Session browser.** A new Sessions tab lists past Claude Code sessions grouped by project, each expanding to show Claude Code's own end-of-session recap, what you asked, where you left off, and which files it touched. One click resumes any session in a new terminal running the provider it originally used — resuming always forks, so a session still open elsewhere is never disturbed. The previous Sessions tab, which reports tokens and cost, is now called **Cost**.

### Fixed

- **Two accounts could show identical usage.** The statusline daemon's shared snapshot (`~/.claude/statusline-usage.json`) carries no account identity, so after the live account changed — a swap, an external `cswap`, or a Claude Code / cowork instance started on another machine — the previous account's numbers were adopted for the new active account. Both rows then showed the same percentages and reset times for up to one polling interval. Snapshots written before the active account last changed are now ignored.
- **Subagent usage was under-counted.** The startup backfill skipped subagent transcripts that the live watcher already collected, so sessions run while the app was closed never had their subagent API calls counted. Existing data is re-ingested automatically on upgrade.

## v1.2.0 — 2026-07-27

### Added

- **Trends tab: click a day to see its per-model breakdown.** Clicking any bar reveals that day's token, cost, and cache breakdown per model, sorted by cost — the whole window's per-day/per-model data loads once alongside the chart, so opening the breakdown is instant with no extra fetch.

### Changed

- **Sessions tab groups subagent transcripts under their parent conversation.** Task/Agent-tool subagent runs used to show up as separate top-level rows; they now roll into the conversation that spawned them, as an indented row per subagent, with their usage counted toward the parent's totals. Model badges also read more clearly: Anthropic tiers collapse to a short name (opus/sonnet/haiku/fable), and third-party/relay models keep a cleaned version of their real name instead of a generic label. Headless or scheduled invocations (no working directory) are labeled "headless" and visually demoted instead of showing a bare, broken-looking "-".
- **Per-model 7-day usage only renders when the account's plan actually reports it.** Pro and Team-Pro plans share a single combined 7-day quota and return null for both the Opus and Sonnet sub-buckets — those rows no longer render empty on plans that don't provide the split.

### Fixed

- **Third-party relay models (GLM, k3, MiniMax, kimi) could show inflated cost and token totals — up to 2-8x.** These relays never write a `requestId`, and write each response's usage across multiple JSONL lines (one per content block); the dedup key fell back to a per-line identifier, so every duplicate line was counted separately. Session history now dedupes on the response's own `message.id` when `requestId` is absent, and existing history is automatically re-ingested once on first launch after this update so past totals correct themselves.
- **Several tray-repositioning call sites could still crash the app.** `move_window(Position::TrayCenter)` panics inside the positioner plugin when the window isn't on any monitor yet — e.g. shown before being placed, or mid display-reconfiguration — and under this app's `panic = "abort"` release profile that takes down the whole process, not just the one window. v1.1.3 closed this for the cold-launch popover-sizing path specifically; window resize, relaunching while already running, the tray menu's Show item, and clicking the tray icon could all still hit it. All four now go through one helper that checks for a monitor first and falls back to a safe screen position, same as the existing first-run fallback.

## v1.1.3 — 2026-07-24

### Fixed

- **App could crash immediately on launch.** On a cold start, the popover could call into the tray-positioning logic before the tray icon had ever reported its on-screen location. That path panicked, and since release builds run with `panic = "abort"`, the panic took down the entire app instead of just failing quietly. The window is now sized correctly immediately and re-anchors to the tray as soon as its position is known — normally on the very next launch or the first click.
- **Wrong bundle identifier on macOS.** The committed `Info.plist` still shipped `com.claude-limits.app` — a leftover from the pre-rename project name — while every other part of the app expected `com.claude-switchboard.app`.

### Changed

- **Expanded view drops a redundant usage summary.** The condensed 5h/7d + Opus/Sonnet + pay-as-you-go readout at the top of the expanded report duplicated the accounts sidebar and cost significant vertical space above the tabs.

## v1.1.2 — 2026-07-23 (not published — see v1.1.3)

### Fixed

- **Windows release build was still broken in v1.1.1.** CI's own artifact-upload workaround (added for an older, buggier `tauri-action`) looked for a `.nsis.zip` updater bundle that current Tauri versions no longer produce — Tauri now signs the NSIS `.exe` directly. The workaround failed on every Windows run looking for a file that would never exist, even though the underlying build was fine. Removed it; `tauri-action` now handles updater artifacts and the `latest.json` manifest natively on both platforms.

## v1.1.1 — 2026-07-23 (not published — see v1.1.2)

### Fixed

- **Auto-updater signing was broken in v1.1.0.** The release build failed on both macOS and Windows at the artifact-signing step because the CI signing key was corrupted; no v1.1.0 binaries were ever published. This release rotates to a fresh signing keypair and ships the working auto-updater, pricing updates, and fixes originally intended for v1.1.0 (see below).

## v1.1.0 — 2026-07-23 (not published — see v1.1.2)

### Added

- **Auto-updater is now fully wired.** The bundler produces signed updater artifacts (`.app.tar.gz` + `.sig` on macOS, `.nsis.zip` + `.sig` on Windows) and the release workflow uploads them explicitly, fixing a tauri-action quirk that dropped the `.sig` files.
- **Support for the latest Anthropic models:** Fable 5, Mythos 5, Sonnet 5, and Opus 4.8, with current per-token and cache pricing.
- **Third-party relay pricing:** MiniMax M2.7, GLM 5.1, and Kimi K3 are now costed in session history.
- **Historical cost re-computation.** On first launch after this update, Switchboard re-prices past session events using the new table, so reports reflect correct costs for previously unknown or corrected models.
- **Shared usage snapshot.** When the statusline daemon has already polled the active account’s `/usage` endpoint, Switchboard adopts that fresh snapshot instead of competing for the same rate-limit budget.
- **Startup hydration.** Last-known-good usage data is restored from the local database on launch, so the popover no longer flashes “usage unavailable” while the first poll is in flight.
- **MIT license.**

### Changed

- **Compact popover redesign.** The default glance view is now 208 px tall and shows only the 5 h and 7 d hero numbers plus a Details disclosure; expanding reveals the Opus/Sonnet split and pay-as-you-go row.
- **Reset countdowns now sit beside their bucket labels** instead of on a separate caption row, freeing vertical space.
- **Warm-up controls are collapsed by default** in each account row; the one-line summary shows Off / On · manual / Every 5 h / Custom (n).
- **Manual refresh is faster across all accounts:** the stagger gap drops from 30 s to 5 s for a user-initiated refresh.
- **README install section** now points to the releases page and explains first-launch quarantine / SmartScreen steps.
- The `Unreleased` comparison link now points to the `claude-switchboard` repository.

### Fixed

- **“Usage unavailable” on transient failures.** Cached usage numbers now stay visible during a 429 or network hiccup, with a stale hint like “rate-limited (429) — showing last good data from 3m ago.”
- **429 backoff escalation.** `Retry-After: 0` no longer triggers ever-increasing exponential backoff; Switchboard retries at the next scheduled poll instead of backing off for minutes.
- **Refresh spinner never stopping.** The spinner now stops on the first `usage_updated` event or a 10 s cap.
- **Popover detached from the menu bar** when expanding Details or re-showing the window. Compact modes now keep the top edge glued to the tray.
- **Cache-savings calculation** is now per-model instead of a Sonnet-only flat rate, so Opus and Fable sessions report accurate savings.

## v1.0.0 — 2026-05-07

**Renamed: Claude Limits is now Claude Switchboard.** First release on the
new repository at https://github.com/FeiXu-1131372/claude-switchboard.

### Migration
- v0.3.x users: install Switchboard from the new releases page. On first
  launch, your data (usage history, account credentials, settings) migrates
  automatically. The old install is preserved as a fallback.
- The old `claude-limits` repository ships one final v0.4.0 release with an
  in-app banner pointing here.

### Functional changes
- None. v1.0.0 is a pure rebrand; warm-up & scheduling features arrive in
  v1.1.0.

## [0.3.0] — 2026-05-07

The multi-account release. Manage every Claude account you sign in to from the same popover, see them all stacked, and switch which one Claude Code uses with a single click — running CC sessions adopt the new account within ~30 seconds, no restart.

### Added

- **Multi-account support.** First-class slots for as many Claude accounts as you want. Each slot has its own polled usage, its own backoff state, and its own row in the Accounts sub-screen. Backed by `accounts.json` (mode-0600, owner-ACL'd, file-locked, atomic-write) and a per-slot in-memory cache.
- **Accounts sub-screen.** Stacked rows for every managed account with 5h/7d bars and reset countdowns. The currently-active slot is rendered with an accent-tinted background and 3px left border. Inactive rows show a hover-revealed **Switch account** button. Plan badges (MAX / PRO / etc.) inline with the email.
- **One-click account swap.** `swap_to_account` writes the target's CC creds to the OS Keychain (macOS) or `~/.claude/.credentials.json` (Windows), splices the matching `oauthAccount` slice into `~/.claude.json`, and reconciles `state.active_slot` eagerly so the UI reflects the new active without waiting on a poll tick. Two-step transaction with rollback: if the global-config write fails, the credential write is reverted.
- **Hot-reload after swap.** Running `claude` CLI sessions and the VS Code extension adopt the new account within ~30 seconds (CC's own in-process keychain cache TTL on macOS, one API tick on Windows). A 60-second `KeychainGuardian` polls every 2s and re-applies the swap target if a stale in-flight OAuth refresh from the previous account writes back rotated tokens.
- **Add-account chooser.** Two paths: "Use upstream's current login" (imports the live `claude` creds in one click) and "Sign in with Claude" (in-app OAuth via local redirect server with PKCE). Idempotent on `accountUuid` — re-adding refreshes the stored blobs in place.
- **Throttled stagger polling.** Replaces the previous parallel fan-out. The poll loop maintains a per-slot `next_poll_at` schedule, picks at most one due slot per tick, and staggers slots by 30 seconds (compressed automatically when the configured interval can't fit `slots × 30s`). The active slot polls first; previously-active and other inactive slots trail at fixed offsets. Schedule is re-seeded on swap/add/remove and respects per-slot 429 backoff.
- **Refresh scope.** `forceRefresh` (and the underlying `force_refresh` command) takes a scope argument: `"active"` (popover home refresh icon, fast path, only re-fetches the current slot) or `"all"` (Accounts panel refresh button, kicks off a staggered round across every slot).
- **Process-detection hint on swap.** The swap-confirm card surfaces how many CLI processes / VS Code windows are currently running CC, with a "adopting in ~30s" hint so the user understands the hot-reload semantics.
- **Unmanaged-active banner.** When CC is logged into an account that hasn't been imported yet, the popover shows a one-tap banner offering to add it.
- **Inactive-slot token rotation.** `refresh_inactive` runs from the poll loop's `token_for_slot` path: refreshes any inactive slot whose token is within 2 minutes of expiry, persists the rotated refresh token back to `accounts.json` under the file lock. Active slot is never refreshed by us — CC owns that.
- **First-launch migration.** Existing single-account installs are automatically migrated into Slot 1 of the new multi-account store on first launch, with a `migrated_accounts` event so the UI can confirm.
- **`accounts_changed` reconciliation event.** The poll loop emits `accounts_changed` whenever `state.active_slot` transitions, so the UI's `is_active` flags stay correct even when the active account changes outside an in-app swap (e.g., `claude login` from the terminal).

### Changed

- **OAuth flow now uses a local redirect server** instead of paste-back. Callback lands on `http://127.0.0.1:<port>/callback`, the listener consumes the code, exchanges it for a token, and shuts down. Far less chance of paste corruption.
- **Tray badge / popover header now reflect the active slot's email.** Header shows the active email; clicking it opens the Accounts sub-screen.
- **Empty-state routing.** With zero managed accounts the app routes straight to AuthPanel — covers both first-run and the post-`claude-login`-but-not-yet-imported case.
- **`@tauri-apps/api`** bumped to `2.11.0`.

### Fixed

- **macOS Keychain write was storing the literal string `"-"`.** `security add-generic-password -w "-"` plus a piped stdin was a misread of the CLI: `security` has no stdin-mode for `-w`, so the dash was stored verbatim and the JSON payload silently discarded. Every swap appeared to succeed but every subsequent `claude` invocation read `-` from the Keychain and showed "logged out". Now passes the JSON to `-w <payload>` directly.
- **AccountsPanel lost the active-slot highlight whenever the active account changed outside an in-app swap.** The frontend's `is_active` flags only refreshed on `init()`, in-app swap, or `migrated_accounts` events; nothing emitted `accounts_changed` from the poll loop's reconciliation. Now emitted whenever `state.active_slot` transitions.
- **In-app swap raced the poll loop's reconciliation.** `swap_to_account` now eagerly sets `state.active_slot = Some(slot)` before returning, so `list_accounts` (called immediately by the UI) sees the post-swap state without waiting on a tick.
- **`schedule_by_slot` retained entries for removed accounts.** `remove_account` now drops the entry; the poll loop also reconciles on each tick.
- **Backoff state survived a swap.** Cleared after every swap — backoff was earned by a different token and shouldn't lock out the new bearer.
- **`AuthPanel` routing for fresh logins.** Routes to AuthPanel whenever `accounts.length === 0`, regardless of `requiresSetup`. Prevents the "live CC creds exist but unimported" case from bottoming out on `LoadingShell` forever.
- Nullable `resets_at` and `utilization` in usage payloads are guarded throughout the UI.
- Popover background made fully opaque on Windows; spurious DWM shadow removed.
- Settings panel cleanups; corner radius / gitignore / clippy tidying across the multi-account merge.

### Removed

- **Single-account `token_store` and the `Conflict` auth variant.** Multi-account makes "OAuth vs CC keychain conflict" structurally impossible — each is just a different slot. `preferred_auth_source` plumbing removed.
- The legacy paste-back OAuth UI (replaced by the local-redirect flow).

### Migration notes

- **Upgrading from v0.2.0 is automatic.** On first launch the existing single-account creds are migrated into Slot 1 of the new multi-account store. No manual action required.
- **The macOS Keychain entry created by ≤ 0.3.0-rc may hold the literal string `"-"`** if you swapped accounts on an affected pre-release build. After upgrading to 0.3.0, the next swap rewrites it correctly. If your terminal still shows "logged out" after one swap, run `claude login` once to re-seed the entry — subsequent swaps will keep it correct.
- Auto-update from v0.2.0 onward is silent; only the *first* install on a new machine still triggers Gatekeeper / SmartScreen.

## [0.2.0] — 2026-04-29

First public release. See the [GitHub release notes](https://github.com/FeiXu-1131372/claude-limits/releases/tag/v0.2.0) for the full changelog — highlights:

- Token persistence moved off the OS Keychain to a mode-0600 / owner-ACL'd file (no first-launch keychain prompt that looks like malware).
- Auto-updater wired up (ed25519-signed bundles).
- Refresh-token rotation working for both OAuth and Claude Code sources.
- Settings persistence via SQLite; CSP set; corrupted-DB recovery.
- Anthropic-warm token system with native vibrancy (macOS) / Mica (Windows 11) / translucent solid (Windows 10).

[Unreleased]: https://github.com/FeiXu-1131372/claude-switchboard/compare/v1.2.0...HEAD
[1.2.0]: https://github.com/FeiXu-1131372/claude-switchboard/compare/v1.1.3...v1.2.0
[1.1.0]: https://github.com/FeiXu-1131372/claude-switchboard/compare/v1.0.0...v1.1.0
[1.0.0]: https://github.com/FeiXu-1131372/claude-switchboard/releases/tag/v1.0.0
[0.3.0]: https://github.com/FeiXu-1131372/claude-limits/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/FeiXu-1131372/claude-limits/releases/tag/v0.2.0
