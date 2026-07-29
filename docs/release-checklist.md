# Release Checklist

Before tagging a release, complete every item on both macOS and Windows.

## macOS (14+)
- [ ] Fresh install (download `.dmg`, drag to Applications, remove quarantine)
- [ ] OAuth paste-back: click "Sign in with Claude", complete in browser, paste `code#state`, verify usage loads
- [ ] Use Claude Code credentials shortcut: sign out, click "Use Claude Code credentials", verify usage loads
- [ ] `debug_force_threshold(five_hour, 75)` fires a notification once
- [ ] Re-run `debug_force_threshold(five_hour, 75)` before reset -> no notification
- [ ] Open expanded report; all 6 tabs render
- [ ] Disconnect network -> stale indicator appears within 15m; notifications do not fire
- [ ] System clock moved backward 2h -> `CachedUsage` marks stale; countdown does not go negative

## Windows (11)
- [ ] Fresh install (`.msi`), SmartScreen "Run anyway"
- [ ] Repeat every macOS step that uses auth + tabs + debug threshold
- [ ] Verify DACL on `credentials.json` fallback (icacls shows user-only access)

## Windows (10)
- [ ] WebView2 auto-bootstrap succeeds
- [ ] Popover renders with translucent-solid fallback (no Mica)

## Relay-model re-ingest (added 2026-07-27)

Migration 0006 wipes `session_events` and `jsonl_cursors` on every account and forces a full re-ingest from the JSONL source of truth, so the walker can re-dedupe relay-model turns on `message.id` instead of a per-line key.

- [ ] Before upgrading, note the current 30-day totals (tokens + cost) for an account with real usage
- [ ] Install the upgrade; app doesn't crash or hang while the re-ingest runs (larger histories take longer than a normal poll)
- [ ] Totals reappear after re-ingest completes — not left at zero
- [ ] If the account has any third-party relay usage (GLM, k3, MiniMax, kimi), its cost/token totals are now lower than before the upgrade, not inflated
- [ ] Native Claude Code totals (non-relay) are unchanged from the pre-upgrade note — this migration should only correct relay-model numbers

## Multi-account swap (added 2026-05-05)

- [ ] Fresh install → upstream `/login` as account A → tray app launches → A appears as active in Accounts list
- [ ] Add B via "Use upstream's current login" path (after upstream `/login` as B)
- [ ] Add C via paste-back OAuth (without changing upstream's login)
- [ ] All three show usage in the Accounts sub-screen with correct numbers
- [ ] Click row B → swap → verify CC primary store + `~/.claude.json` reflect B
- [ ] **Keychain blob is valid JSON, not a sentinel.** After the swap above, run `security find-generic-password -s "Claude Code-credentials" -w | python3 -c 'import json,sys; print(json.loads(sys.stdin.read())["claudeAiOauth"]["accessToken"][:12])'` — must print the first 12 chars of B's access token. If it errors with a parse failure or prints `-`, the keychain write is silently storing garbage (regression of the `-w "-"` bug — `security` has no stdin-mode for `-w`)
- [ ] Hot reload — leave a `claude` CLI session running as A in another terminal; swap to B in tray; within ~30s, send a CC turn and verify it succeeds against B (check `~/.claude/logs` or run `/account` in CC)
- [ ] Hot reload under in-flight refresh — force the running CC to refresh (e.g., wait until access expiry near or use `--debug` log to confirm refresh-in-flight) and trigger swap mid-refresh; verify final keychain state is B (`security find-generic-password -s "Claude Code-credentials" -w | jq -r .claudeAiOauth.refreshToken | head -c 12`); guardian re-applies within 60s
- [ ] Repeat with VS Code extension running — toast shows running-process hint, restart extension and confirm B
- [ ] Run `cswap --switch-to A` externally → tray app's active dot moves to A within one poll interval; no false `unmanaged_active_account` banner
- [ ] Upstream `/login` as new D externally → `unmanaged_active_account` banner appears; click "Add to accounts" → D appears, banner clears
- [ ] Remove C → upstream's active login (A or B) untouched
- [ ] Single-account upgrade: install previous version with one OAuth account → upgrade to multi-account → existing account appears as Slot 1, no manual action
- [ ] Org-shared accounts: add two in same org → bars show identical numbers, "shares quota with…" hint appears

## Custom model providers (added 2026-07-29)

Run on **both macOS and Windows**.

- [ ] Providers tab lists `Anthropic (official)` first; it has no delete control
- [ ] Add a provider from the GLM preset — base URL, model and `CLAUDE_CODE_MAX_CONTEXT_TOKENS` prefill
- [ ] Launch it against a project folder; the terminal opens in that folder and `/status` reports the custom endpoint
- [ ] Launch a second provider while the first is still running; both sessions work independently
- [ ] Launch `Anthropic (official)`; the session uses the currently active managed account
- [ ] `ps aux | grep -i ghostty` (macOS) shows **no** API key in the command line
- [ ] Generated scripts under the app data dir are mode `0700` and are swept on next app start
- [ ] Enable "Set default" on a provider; the banner appears and names it
- [ ] With the default on, run `claude` by hand — it uses the default provider
- [ ] `~/.claude/settings.json` still contains your hooks, `enabledPlugins`, `statusLine` and `model`
- [ ] A `settings.json.switchboard-<ts>` backup exists; no more than 5 accumulate
- [ ] Turn the default off; `settings.json` returns to its previous content and a pre-existing hook still fires
- [ ] Set a default while `settings.json` already has a hand-written `ANTHROPIC_BASE_URL` — the confirmation prompt lists it
- [ ] **Switch default A → B → off** (spec §4.1). Hand-set `ANTHROPIC_MODEL` first; after turning off, that value must be back and neither provider's keys may remain
- [ ] **Decline the confirmation on a switch** — the previous default must still be fully in effect afterwards
- [ ] **Hand-edit a managed key while a default is active**, then turn the default off — your edit survives and the UI names it
- [ ] **Concurrency**: with the app open, run `/config` in Claude Code and toggle something, then set a default — either it succeeds or it reports the file changed; the `/config` change is never lost
- [ ] **Permissions**: `stat -f '%Sp' ~/.claude/settings.json` reads `-rw-------` before and after enabling and clearing a default; backups are `-rw-------` too
- [ ] Delete a provider that is currently the default; `settings.json` is cleaned up first
- [ ] Tray tooltip names the default provider while one is set, and omits the line when none is
- [ ] Settings → Terminal lists only installed terminals; choosing one is used by the next launch
- [ ] **Windows**: launch works on a machine with neither `pwsh` nor `wt.exe` installed, and with ExecutionPolicy at its `Restricted` default

## Session browser (added 2026-07-29)

- [ ] Sessions tab lists real sessions grouped by project, newest project first
- [ ] **No subagent transcripts appear** — cross-check `ls ~/.claude/projects/*/*/subagents/*.jsonl | wc -l` against the row count; the browser must show none of them
- [ ] No headless (`-` project) sessions appear
- [ ] Expanding a row shows Recap / Asked / Left off / Touched and collapses any other open row
- [ ] **Recap matches the `※ recap:` line** shown at the bottom of that conversation in Claude Code, with no trailing `(disable recaps in /config)`
- [ ] A session with several recaps shows the most recent one
- [ ] **Left off is never tool output** — spot-check a session that ended on a tool result
- [ ] Search matches title, project, model, Asked, Left off, and a touched filename; results are flat, not grouped
- [ ] Empty-corpus and no-match states are distinct
- [ ] **A `glm-5.2` session resolves to the GLM provider** and resumes without a prompt (regression: `[1m]` normalization)
- [ ] A session with an unconfigured model opens the picker and warns about cross-model resume
- [ ] Resume opens a new terminal in the session's own folder; `/status` shows the expected endpoint
- [ ] **Resume a session that is still open in another terminal** — both windows keep working, the original transcript is unchanged, and the fork appears as a new row on rescan
- [ ] Cost tab is unchanged apart from its label
- [ ] Subagent backfill: `SELECT COUNT(DISTINCT source_file) FROM session_events WHERE source_file LIKE '%/subagents/agent-%'` returns the on-disk count, not 13
