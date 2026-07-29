# Custom Model Providers — Design Specification

**Date:** 2026-07-29
**Status:** Design pending user review
**Scope:** Spec A of two. Spec B (session browser + one-click resume) is deferred and depends on the provider model defined here.
**Source-validated against:** Claude Code v2.1.220 (`/opt/homebrew/lib/node_modules/@anthropic-ai/claude-code/bin/claude.exe`, Bun-compiled bundle), cc-switch `origin/main` @ `12b972a6`.

---

## 1. Overview

Let users run Claude Code against third-party Anthropic-compatible endpoints (GLM/z.ai, Kimi, DeepSeek, MiniMax, OpenRouter, or any custom URL) from Switchboard.

Switchboard becomes a **launcher** for provider-scoped sessions, plus an opt-in global default. It does not proxy traffic, and by default it does not write `~/.claude/settings.json`.

### 1.1 Why this is not account swapping

Switchboard's existing account swap is hot-reloadable. Provider switching is not, and the difference is structural rather than a matter of effort.

| | Account swap (existing) | Provider switch (this spec) |
|---|---|---|
| What changes | OAuth token in keychain / `.credentials.json` | `ANTHROPIC_BASE_URL` + auth + model env |
| When Claude Code reads it | Every ~30s (cache TTL) + on file mtime change | **Once, at process startup** |
| Running sessions | Adopt within ~30s | Never notice |
| Conversation impact | None — same endpoint, same model | Model changes underneath the transcript |

Source evidence, from the v2.1.220 bundle:

1. **Settings `env` is applied to `process.env` exactly once during startup**, and it *overwrites* the inherited shell environment:
   ```js
   Object.assign(process.env, elr(Rt().env, "globalConfig"));
   for (let r of ph_) { …; Object.assign(process.env, elr(Hr(r)?.env, r)); }   // ph_ = user, flag, policy
   Object.assign(process.env, elr(Hr("policySettings")?.env, "policySettings"));
   ```
   `Object.assign` — not a defer-to-existing merge. **Settings `env` beats exported shell variables.**

   `ph_` is `["userSettings", "flagSettings", "policySettings"]`. `~/.claude/settings.json` is `userSettings`, so it lands in this unconditional-overwrite path — which is precisely the scope this spec writes to. Project- and local-scope settings (`.claude/settings.json`, `.claude/settings.local.json`) take a *different*, filtered path (`for (…) if (rYt(r, n)) process.env[r] = n`) where `rYt` gates each key against an allowlist. That filtering is a deliberate trust boundary — a checked-in repo file should not be able to redirect your endpoint — and it reinforces §4's decision to write user scope only. The allowlist's exact membership was not extracted during design and is not relied on by anything here; confirm it during implementation only if project-scope writing is ever contemplated (it is not, in this spec).

2. **The base URL is read from `process.env` on every use**, both as a constructor default (`constructor({ baseURL: e = vA("ANTHROPIC_BASE_URL"), … })`) and at request-build time (`EQg()` falls through to `process.env.ANTHROPIC_BASE_URL`). This point does *not* establish that the endpoint is frozen — it is re-read constantly. It is listed to forestall the natural objection that re-reading might permit a hot swap: re-reading `process.env` cannot help when nothing ever re-populates `process.env` from settings. Evidence #3 is what closes the argument.

3. **No settings-file watcher exists on that path.** `Dut()` is invoked only from the startup sequence, and the sole `subscribe` callback on the settings store clears caches without re-applying `env`. A running session therefore cannot observe a provider change. **This is the load-bearing claim.**

4. **Anthropic's own host-integration API refuses endpoint swaps mid-session.** The `CLAUDE_CODE_HOST_CREDS_FILE` / `CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST` refresh path contains the guard `host-creds refresh rejected: endpoint changed`. Credentials are designed to hot-rotate; endpoints are designed to be session-scoped.

cc-switch demonstrates the consequence of ignoring this. Its switch handler (`src/hooks/useProviderActions.ts:306`) emits restart warnings for `codex`, `grokbuild`, and `claude-desktop`, but Claude Code falls through to a bare `"Switch successful!"` — hence [issue #3057](https://github.com/farion1231/cc-switch/issues/3057), "Switching back to Claude Official still uses previous provider in active sessions."

### 1.2 Why launcher rather than config mutation

Because finding (1) above means writing `env` to `~/.claude/settings.json` **silently overrides per-process env**, including the user's existing `~/Developer/scripts/start-claude-{glm,kimi,minimax}.sh`. Those scripts would appear to work — the terminal opens, the `export` lines run — while every session routed to whatever Switchboard last wrote.

Per-process launching avoids this entirely and is strictly more capable:

| | settings.json | launcher |
|---|---|---|
| Concurrent providers | one, machine-wide | one per terminal |
| Restart boundary | the central problem | does not exist |
| Stale running sessions | must detect and warn | does not exist |
| Risk to hooks / plugins / statusline | edits a shared file | never touches it |
| Manually started `claude` | hijacked | unaffected |

The restart boundary that dominates cc-switch's UX is an artifact of the mechanism, not an inherent property of provider switching.

### 1.3 Goals

| Goal | Decision |
|---|---|
| Launch a Claude Code session against a chosen provider | Yes — per-process env, macOS + Windows |
| Never break existing user launch scripts | Yes — default path never writes `settings.json` |
| Support concurrent sessions on different providers | Yes — falls out of per-process env |
| Optional global default for bare `claude` | Yes, opt-in, with explicit warning |
| Curated presets for known providers | Yes — 5 entries + Custom |
| Usage / cost tracking for third-party providers | **No** — see §8 |
| Warm-up scheduling for third-party providers | **No** — meaningless for pay-per-token |
| Local proxy / failover | **No** — see §9 |

---

## 2. Data model

New migration `src-tauri/src/store/migrations/0007_providers.sql`:

```sql
CREATE TABLE IF NOT EXISTS providers (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    kind         TEXT NOT NULL DEFAULT 'third_party',  -- 'official' | 'third_party'
    base_url     TEXT,
    auth_token   TEXT,
    env_json     TEXT NOT NULL DEFAULT '{}',
    extra_args   TEXT NOT NULL DEFAULT '[]',
    preset_id    TEXT,
    sort_index   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS provider_default (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    provider_id   TEXT REFERENCES providers(id) ON DELETE SET NULL,
    managed_env   TEXT NOT NULL DEFAULT '{}',
    applied_at    INTEGER
);
```

The `REFERENCES` clause is enforced, not decorative: `store/schema.sql:2` sets `PRAGMA foreign_keys = ON`, and `schema.sql` is `execute_batch`'d on *every* connection open — both the fresh-create path and the healthy-existing-DB path in `open_or_recover` (`store/mod.rs:78`) — so the pragma applies per-connection as SQLite requires. An existing `FOREIGN KEY` on `api_snapshots` already relies on this.

Note that `ON DELETE SET NULL` is a backstop, not the mechanism: deleting a provider that is currently the default must still clear the default first (§7), because a nulled `provider_id` beside a populated `managed_env` would be an orphaned manifest with no UI path to undo the `settings.json` mutation it describes.

`managed_env` is a JSON object mapping each key Switchboard wrote to **the value that key held beforehand** — `null` when the key was absent. It is therefore both the manifest of what we own and the undo record needed to restore the file:

```json
{ "ANTHROPIC_BASE_URL": null, "ANTHROPIC_MODEL": "claude-opus-5" }
```

Clearing the default removes keys whose recorded prior value is `null` and restores the rest.

```rust
pub struct Provider {
    pub id: String,                          // uuid v4
    pub name: String,                        // "GLM"
    pub kind: ProviderKind,                  // Official | ThirdParty
    pub base_url: Option<String>,            // None for Official
    pub auth_token: Option<String>,          // plaintext — see below
    pub env: BTreeMap<String, String>,       // ANTHROPIC_MODEL, context knobs, …
    pub extra_args: Vec<String>,             // appended to the claude invocation
    pub preset_id: Option<String>,
    pub sort_index: i64,
}
```

**`extra_args` exists because the generated script bypasses the user's shell.** Invoking the binary directly is deliberate — it is what makes the launch reproducible — but it also skips shell functions and aliases. The project owner's shell defines `claude()` as a wrapper injecting `--dangerously-skip-permissions`, and their launch scripts pass it explicitly:

```bash
exec /opt/homebrew/bin/claude --dangerously-skip-permissions
```

A flag-free generated script would therefore start prompting for permissions on every launch — a visible regression against the scripts this feature replaces. `extra_args` is per-provider rather than global so that a provider can be launched differently when wanted; it defaults to empty, and each element is quoted by the same function that quotes env values (§3.1), never concatenated as a single string.

**`Anthropic (official)`** is a seeded row with `kind = Official` and an empty `env`. Launching it applies **no overrides**, so the session inherits whichever account the existing accounts feature has active. The provider row is a launch target only; identity ownership stays with the accounts subsystem. It sorts first and cannot be deleted.

**Credential storage is plaintext**, in Switchboard's SQLite DB and — when the optional default is enabled — in `~/.claude/settings.json`. This is a deliberate decision by the project owner: a real user has 5–10 providers, these are revocable service keys rather than identity credentials, and the local-disk exposure is judged acceptable against the complexity of keychain indirection. Rejected alternative: `apiKeyHelper` + OS keychain (Claude Code supports `apiKeyHelper` in the settings schema, re-invoked on `CLAUDE_CODE_API_KEY_HELPER_TTL_MS`, which would make third-party credentials hot-rotatable). Recorded here so the trade-off is visible if the decision is revisited.

---

## 3. Launcher

New module, mirroring the existing `os_scheduler` / `claude_code_creds` platform split:

```
src-tauri/src/providers/
├── mod.rs
├── model.rs          Provider, ProviderKind, env resolution
├── store.rs          SQLite CRUD + seeding
├── presets.rs        curated catalog
├── default_env.rs    the optional settings.json writer (§4)
└── launcher/
    ├── mod.rs        LaunchSpec, Terminal enum, dispatch
    ├── script.rs     script generation (shared shape, per-OS body)
    ├── macos.rs      Ghostty / Terminal.app / iTerm2 / kitty / WezTerm
    └── windows.rs    Windows Terminal / PowerShell / cmd
```

### 3.1 Secrets are never passed on a command line

Process command lines are world-readable (`ps` on macOS, Task Manager / WMI on Windows). Each launch instead writes a mode-**`0700`** script into Switchboard's app-data dir and passes the terminal only the path:

```sh
#!/bin/sh
# Generated by Claude Switchboard. Safe to delete.
cd '/Users/feixu/Developer/claude-usage-gh/claude-switchboard' || exit 1
export ANTHROPIC_BASE_URL='https://api.z.ai/api/anthropic'
export ANTHROPIC_AUTH_TOKEN='…'
export ANTHROPIC_MODEL='glm-5.2[1m]'
export CLAUDE_CODE_AUTO_COMPACT_WINDOW='1000000'
exec '/opt/homebrew/bin/claude' '--dangerously-skip-permissions'
```

The trailing arguments come from the provider's `extra_args` (§2), each quoted individually. Spec B's resume appends `--resume <id> --fork-session` after them.

**Mode `0700`, not `0600`.** The terminal executes the script directly (`ghostty -e <script>`), which requires the owner execute bit; a `0600` script fails with exit 126 and an empty terminal window. `0700` is equally private — the secret protection comes from the group and other bits being clear, which both modes satisfy. The alternative (keep `0600` and invoke `/bin/sh <script>`) works too but adds an interpreter argument to every terminal's command shape for no gain.

**No user-controlled text is interpolated into the script.** The header comment is a fixed string: a provider named `GLM⏎rm -rf ~/Documents⏎#` would otherwise inject a live command line. `name` additionally rejects control characters at the model boundary, and *every* interpolation — `cd`, `export`, and the PowerShell `Set-Location` / `$env:` equivalents — goes through the same quoting function. Nothing is concatenated raw.

The Windows equivalent is a `.ps1` using `Set-Location` and `$env:NAME = '…'`. It inherits the user-scoped ACL of `%LOCALAPPDATA%` rather than setting one explicitly.

Benefits beyond secret hygiene: the script is inspectable when a provider misbehaves, the two platforms differ only in the generated body, and Spec B's resume is a one-token change (`exec claude --resume <sessionId>`).

**Lifecycle.** Scripts are written to `<app-data>/launch/<uuid>.{sh,ps1}`. They are **never deleted immediately after spawning** — the terminal reads the file asynchronously, so eager deletion races the launch. Instead they are swept when older than one hour, both at app start **and on a recurring timer**. A startup-only sweep is insufficient for a menu-bar app that stays resident for weeks: token-bearing files would accumulate for the entire uptime.

### 3.2 Terminal dispatch

| OS | Default | Also supported | Command shape |
|---|---|---|---|
| macOS | Ghostty | Terminal.app, iTerm2, kitty, WezTerm | `open -na Ghostty.app --args -e <script>` |
| Windows | **PowerShell** | Windows Terminal | `powershell.exe -NoExit -ExecutionPolicy Bypass -File <script>` |

**Windows specifics**, each of which independently breaks a naïve command shape:

- **`powershell.exe`, never `pwsh`.** PowerShell 7 (`pwsh`) is a separate install that most machines do not have; Windows ships `powershell.exe` (5.1). Probing for `wt.exe` does not imply `pwsh` exists.
- **`-ExecutionPolicy Bypass` is mandatory.** The default policy on Windows client SKUs is `Restricted`, which blocks *all* scripts including locally authored ones. Without the flag every launch fails.
- **`wt.exe` is not the default choice.** Windows Terminal ships with Windows 11 but is a Store install on Windows 10, so defaulting to it would drop a large fraction of Win10 users into the fallback on first use. Plain `powershell.exe` is universally present; `wt.exe` is offered when detected.
- **`wt.exe` treats `;` as a command delimiter**, so a working directory containing a semicolon must be escaped for that terminal specifically.

Terminal choice is a Switchboard setting, defaulting to Ghostty on macOS (matching the project owner's existing scripts) and `powershell.exe` on Windows. Every provider row also offers **Copy launch command**, so an unsupported or missing terminal is never a dead end. Terminal availability is probed at save time and surfaced in settings, not discovered at launch.

**Working directory** is chosen per launch via the native folder picker (`tauri-plugin-dialog`, already a dependency), with the last-used directory remembered per provider.

> Note on the existing scripts: `start-claude-glm.sh` exports `CLAUDE_LAUNCH_DIR` across `open`, but macOS LaunchServices does not propagate environment to a newly launched application. The relaunch guard still works because `TERM_PROGRAM=ghostty` is true on the second pass, but `TARGET_DIR` most likely falls back to `$PWD` inside Ghostty rather than the intended folder. Baking `cd` into the generated script avoids the class of bug entirely.

---

## 4. Optional global default

An opt-in toggle that additionally writes the provider's `env` into `~/.claude/settings.json`, so bare `claude` invocations use it.

Switchboard has never written this file — it currently touches only the platform credential store and the `oauthAccount` slice of `~/.claude.json`. `settings.json` holds the user's hooks, `enabledPlugins`, `statusLine`, and permissions, so every write is defensive:

1. **Surgical merge.** Parse, mutate only keys inside `env`, re-serialize preserving everything else.
2. **Managed-env manifest.** `provider_default.managed_env` records every key written together with its prior value (§2). Clearing the default touches precisely those keys — never a wholesale `env` deletion. This is the failure cc-switch surfaces as *"switch succeeded, but backfilling the old provider config failed; your manual changes may not have been saved."*
3. **Backup before every write** to `~/.claude/settings.json.switchboard-<unix_nanos>`, retaining the 5 most recent. Nanosecond precision because two writes in the same second would otherwise overwrite each other's backup.
4. **Refuse on malformed JSON.** Never overwrite a file that failed to parse; surface the parse error instead.
5. **Confirm on foreign provider keys.** If `env` already contains keys absent from the manifest that this feature would own, require explicit confirmation before overwriting. The check covers **every key any preset can write** — `ANTHROPIC_*`, `CLAUDE_CODE_*`, **and `API_TIMEOUT_MS`** — not just the `ANTHROPIC_` prefix. §5's presets set `CLAUDE_CODE_MAX_CONTEXT_TOKENS`, `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, `CLAUDE_CODE_SUBAGENT_MODEL` and `API_TIMEOUT_MS`; a prefix-only filter would silently overwrite all four. (The project owner's own `env` block currently holds `CLAUDE_CODE_RETRY_WATCHDOG` — proof the block is not empty in practice.)
6. **Atomic write** — temp file in the same directory, then rename.
7. **Concurrency guard.** Claude Code writes `settings.json` itself — `/config` changes, plugin enable/disable, `statusLine` setup, theme changes. Atomicity prevents a *torn* file; it does nothing about a **lost update**: Switchboard reads, the user toggles a plugin in a running session, Switchboard's rename lands, the plugin change is gone. Capture `(mtime_ns, len)` at read, re-stat immediately before the rename, and abort with a retry prompt if either moved. This is the highest-probability data-loss path in this section — and it is not hypothetical: this file was observed changing under an unrelated writer during the design review of this very spec.
8. **Preserve file mode.** `~/.claude/settings.json` is `0600` on the project owner's machine. A naïve `fs::write` + rename **downgrades it to 0644** under a typical umask, because the replacement inherits the temp file's mode — a permissions regression on every write, in a file that will contain a plaintext API key once a default is set. The temp file must be created `0600` before any content is written, and the backup copy must be `0600` too (`fs::copy` preserves mode on Unix; assert it rather than assume). Windows uses the user-scoped ACL inherited from the profile directory.

### 4.1 Switching the default from A to B

`managed_env` is **write-once until cleared**. Switching directly from provider A to provider B must be `clear(A)` followed by `apply(B)` as a single logical transaction — never a recomputed manifest against the already-mutated file.

Without this, the manifest records the *previous provider's* values as the "prior" state and the user's real configuration is destroyed:

> User has `ANTHROPIC_MODEL: "claude-opus-5"` hand-set. Enable GLM → manifest `{BASE_URL: null, MODEL: "claude-opus-5"}`. Switch to Kimi *without clearing* → the manifest is recomputed against the current file and becomes `{BASE_URL: <glm url>, MODEL: <glm model>}`. Clear the default → `settings.json` is "restored" to GLM's values. `claude-opus-5` is gone permanently, and `ANTHROPIC_BASE_URL` pointing at GLM persists while the UI shows no default set.

Two further variants of the same defect, both closed by the same rule:

- **Orphaned keys.** If A writes six keys and B writes four, the two keys unique to A remain in `settings.json` forever — outside the manifest, invisible to the UI, still overriding the user's shell.
- **Editing the active provider.** Changing a provider's `env` while it is the default is a switch from itself to itself, and must go through the same clear-then-apply path.

**Ordering matters.** The foreign-key confirmation (defense #5) must be evaluated **before** `clear(A)`, not after. Clearing first restores the user's original values into the file, which the check then reports as foreign — so the user is prompted on every ordinary switch, and declining leaves the previous default already undone.

### 4.2 Drift detection on clear

Defense #5 detects keys *absent* from the manifest. It does not detect a key the user edited by hand **while the default was active** — the manifest claims Switchboard owns it, so `clear` silently reverts the user's edit.

`clear` therefore compares each key's current value against the value Switchboard wrote. Where they differ, the user changed it: leave it alone and report what was skipped, rather than reverting.

The warning copy names the actual consequence rather than gesturing at risk:

> Sets this provider for **every** Claude Code session, including ones you start yourself. This overrides `ANTHROPIC_*` variables exported by your shell — scripts like `start-claude-glm.sh` will stop taking effect. Sessions already running keep their current provider until restarted.

Turning the default **off** replays `managed_env` in reverse: keys recorded as `null` are removed, keys with a recorded prior value are restored to it.

---

## 5. Presets

`presets.rs` ships 5 verified entries: **GLM (z.ai)**, **Kimi**, **DeepSeek**, **MiniMax**, **OpenRouter**. Each prefills base URL, model ids, and — critically — the context and timeout knobs:

```
ANTHROPIC_MODEL, ANTHROPIC_SMALL_FAST_MODEL,
ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU,FABLE}_MODEL,
CLAUDE_CODE_SUBAGENT_MODEL,
CLAUDE_CODE_MAX_CONTEXT_TOKENS, CLAUDE_CODE_AUTO_COMPACT_WINDOW,
API_TIMEOUT_MS
```

These are the values users get wrong. Claude Code assigns any unrecognized model id a 200K context window (`_er = 200000`), so a 1M-token endpoint is silently under-used and a smaller one overflows mid-conversation. The preset carries the numbers; the user supplies only a key.

**`CLAUDE_CODE_MAX_CONTEXT_TOKENS` is gated and cannot be relied on alone.** The override applies only when the model id does *not* begin with `claude-`:

```js
let n = Z.CLAUDE_CODE_MAX_CONTEXT_TOKENS;
if (n !== void 0 && n > 0 && !lo(Ei(e)).startsWith("claude-")) return n;
return _er;   // 200000
```

A relay that echoes an Anthropic-style id — some present `claude-sonnet-4-5` verbatim for client compatibility — therefore ignores the knob entirely and gets 200K. The `[1m]` model-id suffix is the more reliable lever, because it is unconditional: `Wb(e) = /\[1m\]/i.test(e)` selects the 1M window, and `Qs(e) = e.replace(/\[1m\]$/i, "")` strips it before the request goes out (which is why the project owner's transcripts record `glm-5.2` while the launch script sets `glm-5.2[1m]`).

Presets whose model ids are non-`claude-` prefixed use `CLAUDE_CODE_MAX_CONTEXT_TOKENS`. Any preset targeting a relay that echoes `claude-*` ids must carry the `[1m]` suffix instead. Both mechanisms are recorded per preset so this is a data decision, not a code path.

**All `env` values must be JSON strings.** Claude Code validates the block as a string→string record, so the advanced env editor must emit `"1000000"`, never `1000000`. The Rust model is `BTreeMap<String, String>`, which enforces this on the backend; the constraint applies to the form.

A preset **seeds** a provider row and does not own it — user edits persist and are never overwritten by a preset update. `preset_id` is retained for display and future migration only. A **Custom** form (name, base URL, token, model, optional advanced env) covers everything else.

---

## 6. UI

New `src/providers/`:

| Component | Responsibility |
|---|---|
| `ProvidersPanel.tsx` | Section container, list, empty state |
| `ProviderRow.tsx` | One provider: name, endpoint, model, Launch, overflow menu |
| `ProviderForm.tsx` | Add/edit, preset picker, advanced env editor |
| `DefaultProviderBanner.tsx` | Shown only while a global default is active |
| `LaunchDialog.tsx` | Folder picker + terminal choice + Copy command |

Mounted as a section in `ExpandedReport`. **The compact popover is untouched** — providers are configured occasionally, not glanced at, and the popover's one-second contract is reserved for usage.

### 6.1 Tray

The tray marker appears **only while a global default is set.** That is the single state in which the existing bars could mislead — the user may have forgotten that all sessions are routed elsewhere, and the bars would answer a question they are no longer asking.

Sessions launched from the Providers panel deliberately get **no** tray signal: they are explicit acts in their own visible windows, several may run at once on different providers, and there is no single truthful answer to encode.

The bars themselves never change meaning. They report Anthropic subscription state, which stays accurate whether or not work is currently being routed there. The marker adds the missing qualifier, not a correction.

---

## 7. Errors

| Condition | Behavior |
|---|---|
| Terminal not installed | Detected at settings time; Launch falls back to Copy command with an inline explanation |
| `claude` binary not found | Validated when a provider is saved, not at launch |
| Script write fails | Error toast; nothing spawned |
| `settings.json` unparseable | Refuse to write; surface parse error; default toggle stays off |
| Unmanaged `ANTHROPIC_*` keys present | Require confirmation before overwrite |
| Provider deleted while it is the default | Clear the default first (manifest replayed in reverse), then delete |
| `settings.json` changed under us between read and write | Abort the write, keep the backup, prompt to retry (§4 defense #7) |
| User hand-edited a managed key while the default was active | Leave it, report it as skipped, do not revert (§4.2) |
| Terminal spawn returns non-zero | Report exit status and the generated script path for inspection |

**Launch failure is not reliably detectable on macOS.** `open -na …` returns 0 as soon as LaunchServices accepts the request — well before the app tries, and possibly fails, to exec the script. A non-zero exit therefore catches a missing application but not a failed launch: the user would see a successful toast and an empty terminal window.

Two consequences. First, this is why §3.1's mode must be right — `0700` failures would have been invisible. Second, if launch confirmation is wanted later, it needs a liveness signal from the script itself (touch a sentinel file that Switchboard polls briefly), not an exit code. That is deliberately **not** in this spec: the failure modes it would catch are all eliminated at save time (§7 row 2) or by construction, and polling for a sentinel adds a timing-dependent code path for a case that should not occur.

---

## 8. Known consequence: third-party pricing is partial, and wrong in both directions

`jsonl_parser` already ingests third-party sessions — the project owner's machine holds 710 `glm-5.2`, 937 `k3`, and 489 `kimi-for-coding-highspeed` events.

`src-tauri/pricing.json` already carries deliberate third-party entries (documented in its `_comment_third_party` block, asserted by `third_party_relay_models_are_priced_at_vendor_rates`), so coverage is **partial, not absent**:

| Model on this machine | Matched prefix | Rate |
|---|---|---|
| `glm-5.2` | `glm` | $1.40 / $4.40 |
| `k3` | `k3` | $3.00 / $15.00 |
| `kimi-for-coding-highspeed` | *(none)* | $0 |

Two distinct inaccuracies follow, and neither is safely one-directional:

1. **Unmatched models cost $0.** Any relay model without a prefix entry — `kimi-for-coding-highspeed` today, and every provider a user adds via the Custom form — contributes tokens but no cost. Under-reporting.
2. **Matched models can be mispriced.** `PricingTable::lookup` is `needle.starts_with(prefix)` against bare prefixes such as `glm` and `k3`, so *every* future variant is absorbed at the incumbent rate regardless of its real price. `glm-5.2-air` (a cheaper tier) bills at $1.40/$4.40 and `k3-turbo` at $3.00/$15.00. Over-reporting.

There is additionally no notion of a **relay markup**: the bundled rates are the vendors' own published API prices, and a relay reselling access may charge differently. `pricing.json` already says so.

This is **documented, not fixed** here — it is pre-existing behaviour that this feature makes more visible by making third-party providers easy to add, not behaviour this feature introduces. Per-provider pricing (a rate attached to the provider row rather than guessed from the model id) is the natural fix and belongs in its own spec.

**What this spec must not do** is claim third-party usage is free. Any UI copy asserting "$0" or "not billed" for third-party providers would be false for `glm` and `k3` today.

---

## 9. Rejected alternatives

**Local proxy (cc-switch's approach).** Pin `ANTHROPIC_BASE_URL` to `127.0.0.1:<port>` and re-route per request from a routing table read on every call. This is the only design that hot-swaps a *running* session, and it enables auto-failover and circuit breaking. Rejected because it is 68 of cc-switch's 221 backend files (31%), requires Switchboard to run continuously or every session breaks, and inserts a local intermediary into all API traffic. The launcher delivers the practical benefit — a session on the provider you chose — for a fraction of the surface.

Note that a proxy would solve only the transport problem. Switching provider mid-conversation still breaks thinking-block signatures (Claude Code recovers via `invalid_thinking_signature` → `tengu_thinking_signature_strip_retry`, discarding the reasoning trail), changes the effective context window, and cold-starts the prompt cache. *Hot-swappable* and *safe mid-conversation* are separate properties; only the first is purchasable with engineering.

**`apiKeyHelper` + OS keychain.** See §2.

**Full provider catalog.** Rejected as ongoing maintenance and the UI density the project owner explicitly wants to avoid.

---

## 10. Files touched

**New (Rust):** `providers/{mod,model,store,presets,default_env}.rs`, `providers/launcher/{mod,script,macos,windows}.rs`, `store/migrations/0007_providers.sql`

**New (TS):** `src/providers/{ProvidersPanel,ProviderRow,ProviderForm,DefaultProviderBanner,LaunchDialog}.tsx` + `__tests__/`

**Modified:** `src-tauri/src/lib.rs` (module + command registration), `src-tauri/src/commands.rs` (provider commands), `src-tauri/src/app_state.rs` (provider store handle), `src-tauri/src/store/mod.rs` (migration registration), `src-tauri/src/tray_icon/` (default-active marker), `src/report/ExpandedReport.tsx` (section mount), `src/lib/ipc.ts` + `src/lib/types.ts` (bindings), `src/settings/SettingsPanel.tsx` (terminal preference), `docs/release-checklist.md`

---

## 11. Testing

**Rust unit tests**
- `default_env`: add keys / clear keys / preserve unrelated `env` entries / preserve `hooks`, `enabledPlugins`, `statusLine` / restore a pre-existing key to its prior value on clear / remove a key that did not exist before / round-trip to byte-identical state / refuse malformed JSON / backup rotation caps at 5
- `default_env` foreign-key detection covers `ANTHROPIC_*`, `CLAUDE_CODE_*` **and `API_TIMEOUT_MS`** (§4 defense #5) — a test asserting each of the four preset-written `CLAUDE_CODE_*` / `API_TIMEOUT_MS` keys is detected, since a prefix-only filter passes the first and fails the rest
- `default_env` **A → B switch** (§4.1): enable A, switch to B without clearing, then clear — the file must return to its pre-A state, with no keys unique to A orphaned. This is the C2 regression test and must exist before the switch path ships
- `default_env` **ordering**: the foreign-key check runs before `clear(A)` — declining the confirmation on a switch leaves A's default fully intact
- `default_env` **drift** (§4.2): a managed key edited by hand while active is left alone on clear and reported as skipped
- `default_env` **concurrency** (§4 defense #7): mutating the file between read and rename aborts the write
- `default_env` **modes** (§4 defense #8): a `0600` settings.json is still `0600` after apply and after clear; the backup is `0600`; no `0644` temp file is observable
- `store`: CRUD, official row seeded and undeletable, cascade behavior when the default provider is deleted
- `presets`: expansion produces the documented key set; every preset declares either `CLAUDE_CODE_MAX_CONTEXT_TOKENS` or a `[1m]`-suffixed model id (§5); all values are strings
- `launcher::script`: correct shell and PowerShell escaping for values containing quotes, spaces, `$`, backticks, **newlines and other control characters**; a provider name containing a newline cannot introduce a script line; generated file mode is **`0700`**
- `launcher`: command construction per terminal, without spawning; no terminal's argv contains a secret; the Windows shape uses `powershell.exe` with `-ExecutionPolicy Bypass`

**Frontend tests** follow the existing `src/**/__tests__` pattern: row rendering, form validation, default-toggle confirmation flow, banner visibility.

**Manual smoke** (added to `docs/release-checklist.md`, run on macOS and Windows): launch each preset and confirm `/status` reports the expected endpoint; launch two providers concurrently and confirm independence; enable the default, start `claude` by hand, confirm it uses the default; disable it and confirm `settings.json` returns to its prior content; confirm a pre-existing hook still fires after a default toggle cycle.

---

## 12. Follow-on: Spec B (session browser)

Deferred, dependent on the provider model above. Claude Code writes what is required into `~/.claude/projects/*.jsonl`. Field coverage measured across 160 local transcripts (100.7 MB):

| Field | Coverage | Purpose |
|---|---|---|
| `sessionId` (filename) | 100% | `claude --resume <id>` |
| `cwd`, `gitBranch` | 100% | grouping and display |
| first user message | 98% | **primary recap** |
| `lastPrompt` | 97% | secondary recap |
| `message.model` | 91% | which provider served the session |
| `aiTitle` | **33%** | recap enhancement when present |

`aiTitle` is a recent Claude Code addition — 49% coverage in the last 7 days against 19% at 7–30 days — so it will improve over time but cannot be the primary recap today. The recap is therefore a three-tier fallback: `aiTitle` → first user message → `lastPrompt`, which covers ~99% of sessions.

The 9% missing `model` are sessions with no assistant turn (aborted before a first response); they have no provider to infer and must render as unknown rather than being hidden.

No summarization or inference is required — it is a read-and-render job over files `jsonl_parser` already walks. A full read of all 160 transcripts takes 0.07 s warm, so a direct scan needs no caching layer.

Resume launches a **new** terminal with the matching provider's env and `--resume <sessionId>`; it never mutates the current global state. That is the same code path as §3, so Spec B is largely UI. The one open design question is what to do when a session's model maps to no configured provider.
