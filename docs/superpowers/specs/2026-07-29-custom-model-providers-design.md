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
   for (let r of ph_) { …; Object.assign(process.env, elr(Hr(r)?.env, r)); }   // user/project/local
   Object.assign(process.env, elr(Hr("policySettings")?.env, "policySettings"));
   ```
   `Object.assign` — not a defer-to-existing merge. **Settings `env` beats exported shell variables.**

2. **The API client binds the base URL at construction time**: `constructor({ baseURL: vA("ANTHROPIC_BASE_URL"), … })`.

3. **No settings-file watcher exists on that path.** A running session cannot observe a provider change.

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
    pub preset_id: Option<String>,
    pub sort_index: i64,
}
```

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

Process command lines are world-readable (`ps` on macOS, Task Manager / WMI on Windows). Each launch instead writes a mode-`0600` script into Switchboard's app-data dir and passes the terminal only the path:

```sh
#!/bin/sh
# claude-switchboard launch script — provider: GLM (generated 2026-07-29T11:22:03Z)
cd '/Users/feixu/Developer/claude-usage-gh/claude-switchboard' || exit 1
export ANTHROPIC_BASE_URL='https://api.z.ai/api/anthropic'
export ANTHROPIC_AUTH_TOKEN='…'
export ANTHROPIC_MODEL='glm-5.2[1m]'
export CLAUDE_CODE_AUTO_COMPACT_WINDOW='1000000'
exec '/opt/homebrew/bin/claude'
```

The Windows equivalent is a `.ps1` with `$env:NAME = '…'` and a trailing `& claude`, written with an ACL restricting access to the current user.

Benefits beyond secret hygiene: the script is inspectable when a provider misbehaves, the two platforms differ only in the generated body, and Spec B's resume is a one-token change (`exec claude --resume <sessionId>`).

**Lifecycle.** Scripts are written to `<app-data>/launch/<uuid>.{sh,ps1}` and swept on next app start, not immediately after spawning — the terminal reads the file asynchronously, so eager deletion races the launch.

### 3.2 Terminal dispatch

| OS | Default | Also supported | Command shape |
|---|---|---|---|
| macOS | Ghostty | Terminal.app, iTerm2, kitty, WezTerm | `open -na Ghostty.app --args -e <script>` |
| Windows | Windows Terminal | PowerShell, cmd | `wt.exe -d <cwd> pwsh -NoExit -File <script>` |

Terminal choice is a Switchboard setting, defaulting to Ghostty on macOS (matching the project owner's existing scripts) and Windows Terminal on Windows. Every provider row also offers **Copy launch command**, so an unsupported or missing terminal is never a dead end. Terminal availability is probed at save time and surfaced in settings, not discovered at launch.

**Working directory** is chosen per launch via the native folder picker (`tauri-plugin-dialog`, already a dependency), with the last-used directory remembered per provider.

> Note on the existing scripts: `start-claude-glm.sh` exports `CLAUDE_LAUNCH_DIR` across `open`, but macOS LaunchServices does not propagate environment to a newly launched application. The relaunch guard still works because `TERM_PROGRAM=ghostty` is true on the second pass, but `TARGET_DIR` most likely falls back to `$PWD` inside Ghostty rather than the intended folder. Baking `cd` into the generated script avoids the class of bug entirely.

---

## 4. Optional global default

An opt-in toggle that additionally writes the provider's `env` into `~/.claude/settings.json`, so bare `claude` invocations use it.

Switchboard has never written this file — it currently touches only the platform credential store and the `oauthAccount` slice of `~/.claude.json`. `settings.json` holds the user's hooks, `enabledPlugins`, `statusLine`, and permissions, so every write is defensive:

1. **Surgical merge.** Parse, mutate only keys inside `env`, re-serialize preserving everything else.
2. **Managed-env manifest.** `provider_default.managed_env` records every key written together with its prior value (§2). Clearing the default touches precisely those keys — never a wholesale `env` deletion. This is the failure cc-switch surfaces as *"switch succeeded, but backfilling the old provider config failed; your manual changes may not have been saved."*
3. **Backup before every write** to `~/.claude/settings.json.switchboard-<unix_ts>`, retaining the 5 most recent.
4. **Refuse on malformed JSON.** Never overwrite a file that failed to parse; surface the parse error instead.
5. **Confirm on unmanaged keys.** If `env` already contains `ANTHROPIC_*` keys absent from the manifest, require explicit confirmation before overwriting — this is the case where a user configured Claude Code by hand or via cc-switch.
6. **Atomic write** — temp file in the same directory, then rename.

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

These are the values users get wrong. Claude Code assigns any unrecognized model id a 200K context window, so a 1M-token endpoint is silently under-used and a smaller one overflows mid-conversation. The preset carries the numbers; the user supplies only a key.

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
| Terminal spawn returns non-zero | Report exit status and the generated script path for inspection |

---

## 8. Known consequence: third-party usage reports as $0

`jsonl_parser` already ingests third-party sessions — the project owner's machine currently holds 710 `glm-5.2`, 937 `k3`, and 489 `kimi-for-coding-highspeed` events. `PricingTable::cost_for` returns zero for unrecognized model ids (asserted by `unknown_model_is_zero_cost_not_panic`), so tokens are counted while cost is not.

This is the safe failure direction — under-reporting rather than inflating with Anthropic prices — and it is consistent with the config-only scope. It is **documented, not fixed** here. Per-provider pricing is a candidate for a later spec.

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
- `default_env`: add keys / clear keys / preserve unrelated `env` entries / preserve `hooks`, `enabledPlugins`, `statusLine` / restore a pre-existing key to its prior value on clear / remove a key that did not exist before / round-trip to byte-identical state / refuse malformed JSON / detect unmanaged keys / backup rotation caps at 5
- `store`: CRUD, official row seeded and undeletable, cascade behavior when the default provider is deleted
- `presets`: expansion produces the documented key set
- `launcher::script`: correct shell and PowerShell escaping for values containing quotes, spaces, `$`, and backticks; generated file mode is `0600`
- `launcher`: command construction per terminal, without spawning

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
