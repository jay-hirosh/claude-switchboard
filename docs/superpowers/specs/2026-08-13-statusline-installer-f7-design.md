# F7 — Statusline installer

**Status:** Design approved (user selected: shared-snapshot + CLI reader architecture; honest "not running" placeholder when the GUI is closed)
**Date:** 2026-08-13
**Depends on:** none directly, but reuses infrastructure from the custom-model-providers feature (`docs/superpowers/specs/2026-07-29-custom-model-providers-design.md`) and the shared-snapshot mechanism built for a different purpose (rate-limit-budget sharing with an external statusline daemon).

## 1. Problem

The roadmap (F7) asks for one-click install of a Switchboard-provided statusline command for Claude Code, showing 5H% in the terminal prompt — the inverse of what the app does today (reading Claude Code's state), and per the roadmap's own framing, "most external-surface risk" of the nine features: it writes into `~/.claude/settings.json`, a live file Claude Code itself reads on every session start and writes to concurrently (`/config`, plugin toggles, theme, and — not coincidentally — the very `statusLine` key this feature adds).

**Correction to an assumption in the roadmap's infrastructure table:** the table lists "Statusline daemon snapshot *reading*" (`poll_loop.rs::read_shared_snapshot`) as existing infrastructure "used by F7." That function reads `~/.claude/statusline-usage.json`, and it does contain the 5H% data this feature needs — but it was built purely as a *consumer* of a third-party, pre-existing statusline daemon's output, to avoid Switchboard competing for the same per-account `/usage` rate-limit budget. **No writer for that file exists anywhere in this codebase.** This feature has to add one.

**Correction to the roadmap's premise about reusable machinery:** the roadmap says this "must reuse the Providers feature's guarded-write machinery." That machinery (`src-tauri/src/providers/default_env.rs`) exists, but its low-level primitives (`read_settings`, `backup`, `stamp`, `write_atomic`) are module-private, not currently reusable from outside `default_env.rs`. This feature needs a small, additive visibility change (`fn` → `pub(crate) fn` on four functions), not a rewrite — but it isn't "already shared" as the roadmap implies.

## 2. Goals / non-goals

**Goals**
- One-click install/uninstall of a Switchboard-provided `statusLine` command in `~/.claude/settings.json`, showing 5-hour utilization % in the terminal prompt.
- Guarded write: backup before every mutation, an undo record that survives app restarts, and confirmation before overwriting a `statusLine` the user (or another tool) already configured — matching the exact posture the default-provider-env feature already established for the same file.
- Works via the existing shared-snapshot format (`~/.claude/statusline-usage.json`), which Switchboard's poll loop now also *writes*, not just reads.

**Non-goals**
- No support for the statusline working while the Switchboard GUI app is not running. The installed command reads the shared-snapshot file; if it's missing or stale (older than one polling interval), the command prints an honest "not running" placeholder rather than a number. A future, larger feature (a fully standalone CLI with its own DB read and independent active-account detection) could remove this limitation — explicitly out of scope here.
- No parsing or use of the session-context JSON Claude Code pipes to the statusline command via stdin. The command drains stdin (to avoid ever blocking Claude Code's write side) but ignores its contents — V1 shows only 5H%, not per-session context.
- No support for 7-day % or any other bucket in the statusline text itself (matches the roadmap's "showing 5H%").
- No change to `read_shared_snapshot`'s existing behavior or its rate-limit-avoidance purpose — this feature adds a writer, it doesn't touch the reader used by `poll_loop.rs`'s own adoption logic.

## 3. Behavior

### 3a. Snapshot writer (poll_loop.rs)

In `apply_fetch_outcome`'s `FetchOutcome::Ok` branch, gated to `Some(slot) == active_slot` (unlike F5's per-account write — a statusline can only ever describe *the* account Claude Code sessions are currently authenticated as, matching `read_shared_snapshot`'s own single-account, no-identity-field format): serialize `snapshot` to the exact JSON shape `read_shared_snapshot` already parses (`{five_hour, seven_day, seven_day_sonnet, seven_day_opus, extra_usage, fetched_at}` — i.e. `UsageSnapshot` as-is, already `Serialize`) and write it to `shared_usage_file_path()`. Best-effort: log-and-continue on failure, same posture as `insert_snapshot` and F5's `record_window_peak`.

`shared_usage_file_path()` (currently private in `poll_loop.rs`) widens to `pub(crate)` so the CLI subcommand (§3b) can resolve the identical path without duplicating the `claude_config_home()` + test-env-var-override logic.

**Self-consistency note:** Switchboard's own poll loop is now both a potential writer and (via the pre-existing rate-limit-avoidance path) a potential reader of the same file. This is harmless, not circular: if Switchboard just wrote a fresh snapshot, its own next-tick `read_shared_snapshot` check sees fresh data and skips an otherwise-redundant poll — a no-op optimization, not a new failure mode.

### 3b. `switchboard statusline` CLI subcommand

`cli.rs` currently has no subcommand parsing — `CliMode` recognizes only `--tick` and `--migrate` flags, falling through to GUI mode for anything else. Add a third mode:

```rust
pub enum CliMode { Tick, Migrate, Statusline, Gui }
```

Recognized as the first positional argument `statusline` (not a flag, to leave room for future subcommands without a flag-vs-subcommand collision). Dispatch in `main.rs` alongside the existing `Tick`/`Migrate` arms: `cli::run_statusline()`, headless, no Tauri runtime, no DB connection — mirrors `run_tick`'s headless posture but is simpler (no DB needed at all).

`run_statusline()`:
1. Drain and discard stdin (read to EOF, ignore contents) — Claude Code pipes session JSON in; not reading it risks the write side blocking on a full pipe buffer on some platforms for larger payloads.
2. Call `poll_loop::read_shared_snapshot(path, max_age, None)` — `active_since` is `None` here (this is a stateless one-shot invocation with no in-memory "when did the active account last change" to compare against; the max-age check alone is sufficient staleness protection for this use).
3. If `Some(snapshot)` with `five_hour.is_some()`: print `"5H {}%"` (rounded) to stdout, exit 0.
4. Otherwise (missing file, stale, or no `five_hour` data): print `"Switchboard: not running"` to stdout, exit 0. Never a non-zero exit or stderr output for this case — Claude Code renders whatever the command prints; erroring would show nothing useful.

`max_age` uses the same default the poll loop's own adoption check uses (`polling_interval_secs`, default 300s) — but the CLI subcommand has no `AppState`/`Settings` to read a user-configured interval from. V1 uses a fixed, generous constant (e.g. 10 minutes) rather than plumbing settings into a headless one-shot command — simpler, and a statusline showing "not running" a few minutes later than strictly necessary after the user quits Switchboard is a reasonable, honest default.

### 3c. Guarded settings.json write (`src-tauri/src/statusline_installer.rs`, new)

Built on `default_env.rs`'s primitives, widened from private to `pub(crate)`: `read_settings`, `backup`, `stamp`, `write_atomic`.

```rust
pub fn apply(path: &Path, command: &str) -> Result<Option<Value>> // returns prior statusLine value, or None if absent
pub fn clear(path: &Path, prior: &Option<Value>, written: &Value) -> Result<bool> // returns false if skipped (drift detected)
```

Single-key analogue of `default_env::apply`/`clear`: `apply` stamps, reads, backs up, sets `settings["statusLine"] = {"type": "command", "command": command}`, writes atomically, returns the prior value (or `None`) as the undo record. `clear` checks drift (does the current `statusLine` value still equal what we wrote? if not, someone changed it since — skip, don't silently revert their edit, matching `default_env::clear`'s exact reasoning) then restores the prior value or removes the key entirely if it was absent before.

`command` is resolved once, at install time, from `std::env::current_exe()`, formatted as `"\"{exe_path}\" statusline"` (quoted for the space in `Claude Switchboard.app`'s path on macOS; double-quotes are accepted by both POSIX shells and `cmd.exe`).

### 3d. Undo state + DB

New migration `0011_statusline_install.sql`, mirroring `provider_default`'s singleton-row shape:

```sql
CREATE TABLE IF NOT EXISTS statusline_install (
    id                 INTEGER PRIMARY KEY CHECK (id = 1),
    prior_value        TEXT,              -- JSON of the pre-install statusLine value, NULL if absent
    installed_command  TEXT NOT NULL,     -- exact command string written, for drift detection
    installed_at       INTEGER NOT NULL
);
```

### 3e. Tauri commands

```rust
#[derive(Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum InstallStatuslineOutcome {
    Applied,
    NeedsConfirmation { foreign_value: Value }, // the existing statusLine settings.json already has
}

get_statusline_install_state() -> Option<StatuslineInstallState>   // installed_command + installed_at, for the settings UI
install_statusline(force: bool) -> InstallStatuslineOutcome
uninstall_statusline() -> bool   // false if skipped due to drift (surfaced as a notice, same as clear_default_provider)
```

`install_statusline`'s foreign-value check (analogous to `foreign_provider_keys`, but for a single object-valued key rather than a set of env-var keys): read current `settings.json`, and if `statusLine` is present AND doesn't match `statusline_install`'s `installed_command` (i.e., it's either never been touched by Switchboard, or drifted since), return `NeedsConfirmation` unless `force`. Same ordering requirement as `set_default_provider`: this check must run before any prior Switchboard-owned value is cleared.

### 3f. Frontend

New Settings section in `SettingsPanel.tsx`, matching the existing `h2` + `Card` pattern used by every other section: "Terminal statusline" with a one-line description and an Install/Uninstall button reflecting `get_statusline_install_state()`. On `NeedsConfirmation`, a plain `window.confirm()` (matching `ProvidersTab.tsx`'s existing pattern exactly, not a new custom modal component) showing the foreign value before re-invoking with `force=true`.

## 4. Edge cases

- **`settings.json` doesn't exist yet** (fresh Claude Code install): `read_settings` already handles this (empty object), reused as-is.
- **Concurrent write by Claude Code itself**: `write_atomic`'s existing `FileStamp` check (mtime+len captured before mutation, re-verified immediately before the atomic rename) already guards this — reused as-is, no new logic needed.
- **User uninstalls Switchboard entirely** (app deleted, `current_exe()` path no longer resolves): the installed `statusLine` command becomes a dangling path. Claude Code's own handling of a failing statusline command is out of this app's control — no different in kind from any other case where a user removes a tool that wired itself into their shell config. Not handled specially.
- **User moves the Switchboard app after installing** (e.g. drags `.app` to a different folder on macOS): same class of limitation — the installed command string is a resolved absolute path, not re-resolved dynamically. Acceptable for V1; documented as a known limitation rather than solved (solving it would mean either a stable well-known install location assumption or a wrapper shim, both out of scope).
- **Uninstall after the user hand-edited `statusLine` while Switchboard's was active**: drift check in `clear()` catches this — returns `false` (skipped), frontend shows a notice, same UX as `clear_default_provider`'s existing skipped-keys notice.
- **Statusline subcommand invoked with no shared-snapshot file at all** (never installed, or installed on a machine that's never run the GUI): same as "stale" — prints "Switchboard: not running."

## 5. Testing

Backend:
- `default_env.rs` primitive widening: existing tests continue to pass unchanged (pure visibility change, no behavior change) — run as a regression check, not new test-writing.
- `statusline_installer.rs`: `apply` backs up before writing, sets the key correctly, returns the correct prior value (`None` when absent, `Some` when present); `clear` restores prior value / removes the key when absent-before; `clear` detects drift and skips without reverting; concurrent-write guard behaves the same as `default_env.rs`'s existing coverage (can largely reuse those test patterns directly).
- `cli::run_statusline` (or an inner function separated from process-exit/stdio concerns for testability): fresh snapshot → correct "5H N%" formatting; missing file → "not running"; stale (older than max_age) → "not running"; malformed/foreign JSON in the shared-snapshot file → "not running", not a crash.
- Poll-loop writer: a successful poll of the active slot writes the shared-snapshot file in the format `read_shared_snapshot` can parse back (round-trip test, mirroring F5's upsert tests' style).

Frontend:
- Settings section: renders "Install" when uninstalled, "Uninstalled" state correctly reflects `get_statusline_install_state()`; Install → Applied updates the button state; Install → NeedsConfirmation triggers the confirm dialog and force-reinvokes on accept, does nothing on cancel.

## 6. File-level checklist

New: `src-tauri/src/store/migrations/0011_statusline_install.sql` (next after `0010_window_peaks.sql`), `src-tauri/src/statusline_installer.rs` (+ tests).
Modified: `src-tauri/src/providers/default_env.rs` (widen 4 primitives to `pub(crate)`, no behavior change), `src-tauri/src/store/schema.sql` (table def for fresh installs), `src-tauri/src/store/queries.rs` (get/set/clear for the new singleton row, mirroring `provider_default`'s existing query functions), `src-tauri/src/poll_loop.rs` (writer in `apply_fetch_outcome`, widen `shared_usage_file_path` to `pub(crate)`), `src-tauri/src/cli.rs` (new `Statusline` mode + `run_statusline`), `src-tauri/src/main.rs` (dispatch the new mode), `src-tauri/src/commands.rs` (3 new commands + `InstallStatuslineOutcome`), `src-tauri/src/lib.rs` (register the 3 commands in both `collect_commands!` blocks), `src/lib/generated/bindings.ts` (regenerated), `src/lib/ipc.ts` (3 wrappers), `src/settings/SettingsPanel.tsx` (+ test, new section).
