# Handoff — implement Custom Model Providers + Session Browser

Paste everything below the line into a fresh agent session started in
`/Users/feixu/Developer/claude-usage-gh/claude-switchboard`.

---

Implement two features in this repo, end to end, following the written plans.

## What to read first, in this order

1. `docs/superpowers/plans/2026-07-29-custom-model-providers.md` — Spec A plan, 12 tasks
2. `docs/superpowers/plans/2026-07-29-session-browser.md` — Spec B plan, 9 tasks

The plans contain the actual code and the actual commands. Read the corresponding
spec only when you need the *why* behind a decision:

- `docs/superpowers/specs/2026-07-29-custom-model-providers-design.md`
- `docs/superpowers/specs/2026-07-29-session-browser-design.md`

Both specs survived adversarial review; the plans already incorporate every
accepted finding. Baseline commit: `d5425f3` on `main`.

## What you are building

**Spec A — Custom Model Providers.** Run Claude Code against third-party
Anthropic-compatible endpoints (GLM, Kimi, DeepSeek, MiniMax, OpenRouter, custom)
by launching provider-scoped terminal sessions. Switchboard writes a short
`0700` script holding per-process `export`s and `exec claude`, then spawns the
user's terminal at it. Nothing global is mutated, so several providers run
concurrently. A separate opt-in path merges the same env into
`~/.claude/settings.json` for bare `claude` invocations.

**Spec B — Session Browser.** A new Sessions tab lists past Claude Code sessions
grouped by project, each expanding to a recap card (Claude Code's own
`away_summary`, what you asked, where you left off, files touched). One click
resumes a session in a new terminal on the provider it originally used. The
existing Sessions tab — an accounting view — is renamed **Cost**.

## Execution order

**Spec A tasks 1–12 first, complete, then Spec B tasks 1–9.**

Do not reorder. Two hard dependencies:

- Spec B Task 8 calls `launcher::launch` and reads the `providers` table, both
  created by Spec A.
- Migration numbering assumes this order. Spec A owns `0007` and bumps the
  schema to 7; Spec B owns `0008` and bumps to 8. Running them out of order
  requires renumbering both, and **both** the `create_fresh_db` stamp and the
  trailing stamp in `migrate()` must agree.

One exception worth knowing: **Spec B Task 1 is independently shippable** and
fixes a live data bug (subagent transcripts never backfilled — 125 files,
12.3M input tokens uncounted). It has no dependency on anything. If you want an
early win, it is safe to do first — but then renumber its migration to `0007`
and Spec A's to `0008`, and say so in the commit.

## How to execute

Use the `superpowers:subagent-driven-development` skill (preferred) or
`superpowers:executing-plans`. Work task by task. Each task ends with a
commit; do not batch commits across tasks.

Every task follows the same shape: write the failing test, run it and confirm it
fails, implement, run it and confirm it passes, commit. Do not skip the
confirm-it-fails step — it is what proves the test exercises the thing.

## Hard rules

**Commit to `main`. Do not create a branch or worktree.** The user's global
`CLAUDE.md` forbids it without explicit confirmation. If you believe isolation
is genuinely needed, stop and ask.

**Never weaken a test to make it pass.** Several tests encode defects that took
adversarial review to find. If one fails, the implementation is wrong, not the
test. These specifically must not be relaxed, deleted, or narrowed:

| Test | Guards |
|---|---|
| `switching_default_a_to_b_then_clearing_restores_the_users_own_value` | Permanent loss of the user's hand-set `ANTHROPIC_MODEL` |
| `apply_and_clear_preserve_a_0600_settings_file` | Silently downgrading a secret-bearing file to world-readable |
| `write_aborts_when_the_file_changed_under_us` | Destroying a concurrent Claude Code `/config` edit |
| `resolves a [1m]-suffixed provider config against a stripped session model` | GLM and k3 never resolving — the two heaviest providers |
| `subagent_transcripts_are_never_discovered` | Listing 208 rows instead of 70, with broken Resume buttons |
| `tool_results_never_become_asked_or_left_off` | Rendering tool output as the recap on 58% of rows |
| `no_command_carries_a_secret_in_its_arguments` | API keys visible in `ps` |

**Add every new Tauri command to BOTH `collect_commands!` lists** in
`src-tauri/src/lib.rs` — there is a `#[cfg(not(debug_assertions))]` list around
line 160 and a `#[cfg(debug_assertions)]` list around line 197. Adding to only
one compiles fine and breaks the other build profile.

**Do not touch `~/.claude/settings.json` outside `providers::default_env`,** and
do not run the app against the user's real settings file while iterating on that
module. Its tests use `tempdir`.

**Design tokens only.** Every colour, radius, spacing and duration comes from
`var(--…)`. The spacing scale bottoms out at `--space-2xs` — there is no
`--space-3xs`. Icons come from `src/lib/icons.ts` (Lucide). No emojis.

## Known traps, already accounted for in the plans

These are called out where they matter; listed here so nothing surprises you.

- **Two forward references.** Spec A Task 7 imports `ProviderForm` (Task 8) and
  Spec B Task 7 imports `useResume` (Task 8). Both plans create a stub in the
  earlier task and replace it in the later one. Keep the stubs; do not try to
  reorder around them.
- **`useResume` returns JSX**, so the file must be `.tsx`, not `.ts`.
- **`ModalShell` takes `id` and `onDismiss`**, not `onClose`, and `id` is
  required. **`IconButton` takes a required `label` prop** and sets `aria-label`
  itself — do not pass `aria-label` directly.
- **New dependencies:** `which` (runtime) and `filetime` (dev) for Spec A.
  `cargo add` them when the task says to, not upfront.
- Where a plan says to read an existing component and match its real prop names,
  do that — adapt the call site, never the shared component.

## Verification

Run after every task:

```bash
cd src-tauri && cargo test
cd .. && npm test && npm run lint
```

Run before declaring either feature done:

```bash
cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings
cd .. && npm test && npm run build
```

Do not report a task complete on a failing or skipped test. If something cannot
be verified — anything Windows-specific, or terminal spawning, which needs a GUI
— say so explicitly and mark it for the manual smoke pass rather than claiming
it works.

Both plans end with a manual smoke checklist appended to
`docs/release-checklist.md`. Two items in particular fail *silently* rather than
loudly, so they must actually be exercised rather than reasoned about:

1. A `glm-5.2` session resumes directly, without the provider picker.
2. The Sessions tab lists no subagent transcripts — cross-check the row count
   against `ls ~/.claude/projects/*/*/subagents/*.jsonl | wc -l`.

## Reporting

Report progress per task: what you implemented, the test command you ran, and
its result. At the end, summarise what is complete, what is verified by tests,
what still needs manual smoke on each OS, and anything you deviated from the
plan on and why.

If a plan step is wrong — the codebase has moved, an API differs, a test cannot
work as written — stop and say so with the evidence rather than improvising a
workaround. The plans are detailed enough that a mismatch usually means
something real changed.
