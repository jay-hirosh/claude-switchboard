# Session Browser & One-Click Resume — Design Specification

**Date:** 2026-07-29
**Status:** Design pending user review
**Scope:** Spec B of two. Depends on the provider model and launcher from `docs/superpowers/specs/2026-07-29-custom-model-providers-design.md`.
**Measured against:** the 160 top-level transcripts in `~/.claude/projects/` (100.7 MB), excluding 138 subagent sidechains at `<sessionId>/subagents/` (§3). Corpus total is 298 files / 131.7 MB. Claude Code v2.1.220.

---

## 1. Overview

Browse past Claude Code sessions, understand what each one was actually about, and resume any of them in a new terminal running the provider it originally used.

### 1.1 The problem this solves

Claude Code's built-in `/resume` lists sessions by title. A title is not enough to identify a session you started weeks ago, and once you run several providers the problem compounds: the local machine holds sessions on `claude-opus-5`, `glm-5.2`, `k3` and `claude-sonnet-5` that are visually indistinguishable, in the same repo, on the same branch.

Resuming the wrong one is not merely confusing — resuming a `glm-5.2` conversation while Anthropic is the active provider silently continues it on a different model.

### 1.2 What the transcripts actually contain

Coverage measured across the 70 **substantive** sessions (see §3 for the filter):

| Signal | Coverage | Use |
|---|---|---|
| `sessionId` (filename) | 100% | `claude --resume <id>` |
| `cwd`, `gitBranch` | 100% | grouping, launch directory |
| First user message | **100%** | "Asked" — the intent |
| Last user message | **100%** | "Left off" — where it stopped |
| Turn count, time span | 100% | weight and duration |
| `away_summary` (recap) | 50% | **Recap** — goal, state and next action |
| `aiTitle` | 71% | title when present |
| Tool usage histogram | 71% | (not surfaced in v1) |
| Touched file paths | 61% | "Touched" file chips |
| `message.model` | **84%** | provider resolution |

The 16% without a usable model (11 of 70) are sessions that ended before any assistant turn, or whose only recorded model is the `<synthetic>` placeholder Claude Code writes for locally-generated messages. They must render as *unknown provider* and route through the picker (§7) — not be hidden, and never silently resumed.

> **Denominator note — reconciling with Spec A §12.** That table measured all 160 transcripts; this one measures the 70 substantive sessions (§3). Where they differ, this table governs, because the browser only ever lists substantive sessions.
>
> | Field | All 160 (Spec A) | Substantive 70 (here) |
> |---|---|---|
> | `aiTitle` | 33% | **71%** |
> | `message.model` | 91% | **84%** |
>
> `aiTitle` rises because headless transcripts have no title — they have no conversation. `message.model` falls because the all-files figure counted the `<synthetic>` placeholder as a model; here it does not.

Full parse of all 160 transcripts takes **0.35 s** warm, so the browser reads from disk directly rather than persisting a table. It does keep an **in-memory memo in `AppState`, invalidated on `max(mtime)` change** — roughly fifteen lines, no migration, no staleness class. Without it every tab switch re-reads and re-parses 100.7 MB, because `ExpandedReport` remounts tab components across its slide transition; on a menu-bar app open all day that is real disk and battery cost. The first scan after boot is cold and slower than 0.35 s.

### 1.3 Goals

| Goal | Decision |
|---|---|
| Identify a session without opening it | Yes — expandable recap card |
| Resume in the provider the session originally used | Yes — via the Spec A launcher |
| Never silently resume on the wrong model | Yes — unresolved models prompt |
| Never disturb the current session or global config | Yes — launches a new terminal only |
| Find a session among many | Yes — search + grouping |
| Delete transcripts | **No** — destructive, out of scope |
| Token/cost per session | **No** — that is the Cost tab's job |
| Full transcript viewer | **No** — see §9 |

---

## 2. Tabs

The existing `SessionsTab` is an accounting view over the SQLite `session_events` table: tokens, cost, model badges, subagent rollup. It is renamed to **Cost**, with no behavioural change.

A new **Sessions** tab becomes the browse-and-resume view described here.

The rename is deliberate. Both views legitimately list sessions, but they answer different questions — *"where did my money go"* versus *"what was I doing"*. Naming the accounting view **Cost** makes that split legible instead of shipping two tabs that both claim the word "Sessions".

**The rename carries no migration cost.** `activeTab` is component-local `useState` in `ExpandedReport.tsx`, not persisted in the Zustand store, so no user has a stale `'sessions'` tab id saved anywhere. The identifier appears in exactly three places, all in that one file (`TAB_CONFIG`, the `useState` initialiser, and `prevTabRef`). The file `SessionsTab.tsx` is **not** renamed — only its tab id and label — so the diff stays reviewable and its exported `modelLabel` / `isHeadlessProject` helpers keep working for both tabs.

---

## 3. What counts as a session

54% of local transcripts (87 of 160) are **headless**: project directory `-`, no `cwd`, generated by scheduled warm-up invocations. They have no conversation and cannot meaningfully be resumed. Listing them would make the browser majority noise.

A transcript is included when **all** of:

1. `cwd` is present and non-empty
2. the project directory is not `-`
3. at least one **real user message** exists — a `type: "user"` record carrying a non-empty `text` content block that is not a known injection marker (`<system-reminder>`, `<command-name>`, `<local-command-stdout>`, `<command-message>`). Matching known markers costs the same as a bare `<` prefix guess and will not misfire on pasted markup or generics such as `Vec<T>`. Records whose content is entirely `tool_result` blocks are not user messages at all and never qualify — see §4.3

This yields 70 of 160 transcripts today.

**Bounding.** The cap is **200 sessions after filtering**, not 200 files scanned. Applied before the filter it would be actively harmful: with subagent transcripts on disk the corpus is already 298 files, so a pre-filter cap would let one session's 102 subagent files consume half the budget and evict real sessions. Scanning is cheap (§1.2); it is the result list that needs bounding.

**Subagent transcripts must be excluded explicitly.** Subagent (Task/Agent tool) turns are recorded **exclusively as separate files** at `<project>/<sessionId>/subagents/agent-*.jsonl`, and not at all inline. The corpus holds 138 of them across 7 sessions — one session alone contributes 102.

**All 138 satisfy the three inclusion conditions above**: they carry a real `cwd`, a project directory that is not `-`, and real user turns (the subagent's prompt). Nothing in the filter distinguishes them, so `scan.rs` must skip any path containing a `subagents/` segment as a stated rule.

This is not a hypothetical. `scan.rs` is described below as a directory walk; the idiomatic Rust choice (`walkdir`) recurses, which would return **208 rows instead of 70**, dominated by the 102 from a single session. Each would render with a plausible title and *Asked* text, and each would offer a Resume button running `claude --resume <agentId>` against an id that is not a session.

> An earlier revision of this spec claimed the opposite — "recorded as inline records … measured: 0 sidechain-only files". That measurement used a flat `~/.claude/projects/*/*.jsonl` glob, which structurally cannot reach a third directory level. The corpus is 298 `.jsonl` files, not 160: 160 top-level plus 138 subagent transcripts. Every coverage figure in §1.2 remains correct for the 160 top-level files, which are the only ones this browser lists.

---

## 4. The recap

### 4.1 Title

`aiTitle` when present (71%), otherwise the first user message truncated to a single line. Never empty.

### 4.2 Collapsed row

```
▸ Plan custom model swapping feature          opus-5 · 2h ago
  claude-switchboard · main · 12 turns
```

Title, model badge, relative time, project basename, branch, turn count.

Model badges reuse the existing `modelLabel()` from `src/report/SessionsTab.tsx`, which already collapses Anthropic families to their tier name and renders third-party ids cleanly (`glm-5.2`, `k3`) — so the two tabs stay visually consistent.

### 4.3 Expanded card

```
▾ Plan custom model swapping feature          opus-5 · 2h ago
  claude-switchboard · main · 12 turns over 2h07m

  Recap    Goal: add third-party model provider support to
           Switchboard. The spec is written and committed.
           Next: review the spec, or save the hand-off prompt.
  Asked    "we need a to plan for another major feature which
            allows users to swap in there own model…"
  Left off "what about spec B? previous session list and one
            click resume feature?"
  Touched  custom-model-providers-design.md · +3 more

                                            [ Resume → ]
```

**Recap** leads the card when present. Claude Code writes its own end-of-session summary — the `※ recap:` line shown at the bottom of a conversation — as a `type: "system"`, `subtype: "away_summary"` record with the text in `content`. It is the single most identifying signal in the transcript because it states goal, state **and next action**:

> Goal: add third-party model provider support to Switchboard. The design spec is written and committed (`60913d2`), and I've given you a hand-off prompt for a fresh agent to review it. Next: review the spec yourself, or tell me to save that prompt to `docs/`.

Three properties govern its handling:

- **50% coverage** (35 of 70). Unlike `aiTitle` this is *not* recency-correlated — 47% in the last 7 days against 57% at 7–30 days — so it is a steady half, not a feature ramping up. It layers on top of the 100% floor below; it does not replace it.
- **Rewritten as the session moves.** 20 sessions carry more than one record, one carries 16. Take the **last**, which is the current one.
- **Strip the trailing `(disable recaps in /config)`.** Present on all 35; it is interface chrome, not content.

It is not used for the title — it is a paragraph, the wrong shape — so the title chain in §4.1 is unchanged.

**Both *Asked* and *Left off* draw from the filtered real-user-message sequence — the same predicate §3 uses for inclusion, reapplied during extraction.** The 100% figures are a property of that filter, not of the raw data.

This is not a refinement. `type: "user"` records also carry **tool results**, which have no text block at all. On the 70 sessions this browser lists, **41 (58%) end on a `type: "user"` record that is not a real user message**. Taking "the last `type: user` record" would render *Left off* as `"This command requires approval"` or a raw directory listing — or blank — on the majority of rows.

- **Recap** — last `away_summary`, chrome suffix stripped, clamped to 3 lines. Omitted entirely when absent (50%).
- **Asked** — first real user message, 100% under the filter, clamped to 2 lines.
- **Left off** — last real user message, 100% under the filter, clamped to 2 lines. Omitted when identical to *Asked* (single-turn sessions; median turn count is 3).
- **Touched** — up to 4 files from `tool_use` blocks with a `file_path` input, most-frequent first, plus an overflow count. Basenames, except where two entries would collide — this repo yields `mod.rs · +3 more` often enough to matter — in which case the parent segment is included (`store/mod.rs`). Omitted entirely when absent (39%) rather than rendering an empty label.
- **Stats** — turn count and wall-clock span from first to last `timestamp`.

Only one row is expanded at a time; expanding another collapses the previous.

---

## 5. Grouping and search

**Grouping.** Sessions group under their project (`cwd`), projects ordered by their most recent session, sessions ordered by recency within. Today that is 70 sessions across 6 projects, so the whole list is navigable without scrolling far.

**Search.** A single filter box matching case-insensitively against title, project path, branch, model, *Recap*, *Asked*, *Left off*, and touched-file names. *Left off* is frequently the memorable thing about an abandoned session, and a filename is often how you remember what you were editing. While a query is active the grouping flattens to a single recency-ordered result list, because grouping fights matching — a two-result query should not be split across two headers.

Empty states are distinct: "no sessions yet" (nothing on disk) versus "no sessions match" (filter too narrow, with a clear-filter action).

---

## 6. Model → provider resolution

```
norm(session.model)  ─── compared against ───  norm(provider.<each model key>)
   │
   ├─ match on ANTHROPIC_MODEL, ANTHROPIC_SMALL_FAST_MODEL, or
   │  ANTHROPIC_DEFAULT_{OPUS,SONNET,HAIKU,FABLE}_MODEL
   │        → that provider (lowest sort_index wins on tie)
   ├─ session.model starts with "claude-"   → Anthropic (official)
   └─ otherwise                             → unresolved

norm(x) = x.lowercase().strip_suffix("[1m]")     // applied to BOTH sides
```

**`norm` must be applied to both operands.** Claude Code strips the `[1m]` context modifier before writing the transcript, so the *session* side arrives already normalized — `glm-5.2`, `k3`. The value that still carries the suffix is the **provider's**, straight from the launch config:

```
provider config              transcript records
  ANTHROPIC_MODEL=glm-5.2[1m]  →  glm-5.2   (710 events)
  ANTHROPIC_MODEL=k3[1m]       →  k3        (1289 events)
```

Normalizing only the session side is a no-op, and `"glm-5.2" == "glm-5.2[1m]"` is false — so GLM and k3 would fall through to the §7 picker on every single resume. These are the two heaviest third-party providers in the corpus and precisely the ones this section exists to handle.

Lowercasing is equally load-bearing: the MiniMax launch script sets `MiniMax-M2.7-highspeed`, recorded verbatim.

All six model env keys are checked, not just `ANTHROPIC_MODEL`. 519 recorded events are `kimi-for-coding-highspeed`, which the Kimi config sets as `ANTHROPIC_SMALL_FAST_MODEL` — a single-key match would miss every one.

**Ties resolve by `sort_index`, then `id`.** Two providers can declare the same model id (duplicating a row does it immediately), and "any provider's" is not a rule. Deterministic ordering means a given session always resolves to the same provider.

**Deleting a provider must not silently reroute its sessions.** The `claude-*` → official fallback sits after exact match, which is correct — but it means deleting a provider that used Anthropic-style model ids reclassifies all its past sessions as official, and §7 would then resume them on Anthropic with no prompt. That is the exact failure §1.3 promises to prevent. Sessions whose model matched a now-deleted provider are therefore treated as **unresolved**, not official: resolution consults the set of model ids ever configured, so a disappeared provider produces a prompt rather than a silent substitution.

---

## 7. Resume

Clicking **Resume**:

1. Resolve the provider from the session's model (§6).
2. **Resolved** → launch immediately.
3. **Unresolved, or no usable model recorded (16%, §1.2)** → open a picker listing configured providers, defaulting to the best guess, with an *Add a provider* action that opens the Providers tab form. When the chosen provider's model differs from the one recorded in the transcript, the picker states the consequence in one line: thinking-block signatures will not validate and are stripped on retry (Spec A §9), the prompt cache cold-starts, and the effective context window changes. This path is not rare — it covers the 16% with no usable model — so it must not present a cross-model resume as an unremarkable default.
4. Launch via the Spec A launcher: `launcher::launch(LaunchSpec { provider, cwd: session.cwd, terminal, resume_session_id: Some(id) })`.

This opens a **new terminal** in the session's original working directory. Global configuration is never touched and the current session keeps its provider — resuming a `glm-5.2` session while working on Anthropic changes nothing about the session you are already in.

### 7.1 Resume always forks

The generated command is `claude --resume <id> --fork-session`, never a bare `--resume`.

Without it, resuming a session that is **still open in another terminal** puts two Claude Code processes on the same `<sessionId>.jsonl`, both appending. Nothing in the transcript format arbitrates that, and the damage is to conversation history the user cannot reconstruct.

Detecting the conflict instead was rejected: `process_detection.rs` counts CLI processes but cannot map one to a session id, so the check would be unreliable in exactly the case that matters. `--fork-session` removes the failure by construction rather than detecting it.

The trade-off is accepted deliberately: continued work lands in a **new** transcript rather than appending to the original, so a resumed session appears as a sibling entry in the list on next scan. That is the correct semantics for a browse-and-reopen action — the thing you clicked stays exactly as you left it.

`LaunchSpec.resume_session_id` and the `--resume` rendering in `launcher::script` are **already implemented and tested in Spec A**, so no new launch machinery is required.

**Failure modes.** A session whose `cwd` no longer exists reports "folder no longer exists" and offers to pick a different one rather than launching into a missing directory. Terminal-missing and launch-failure paths are inherited from Spec A.

---

## 8. Architecture

```
src-tauri/src/sessions/
├── mod.rs        SessionSummary type, re-exports
├── scan.rs       directory walk + inclusion filter (§3)
└── recap.rs      per-transcript extraction (§4)
```

One Tauri command: `list_resumable_sessions() -> Vec<SessionSummary>`. Provider resolution (§6) lives in TypeScript beside `modelLabel`, not behind IPC — it is a pure function over data the frontend already holds, so a per-row IPC round-trip would buy nothing.

```rust
pub struct SessionSummary {
    pub session_id: String,
    pub cwd: String,
    pub project_name: String,       // basename of cwd
    pub git_branch: Option<String>,
    pub title: String,              // aiTitle, else truncated first message
    pub recap: Option<String>,      // last away_summary, chrome stripped
    pub asked: String,              // first user message
    pub left_off: Option<String>,   // last user message; None when == asked
    pub touched_files: Vec<String>, // basenames, most-frequent first, max 4
    pub touched_overflow: usize,
    pub model: Option<String>,
    pub turns: u32,
    pub started_at: String,         // RFC3339
    pub ended_at: String,
}
```

Parsing is line-by-line and tolerant: a malformed line is skipped, never fatal. A transcript that fails entirely is omitted from the list with a warning logged, so one bad file cannot blank the tab.

**Frontend:** `src/sessions/` — `SessionsBrowserTab.tsx`, `SessionRow.tsx`, `SessionRecapCard.tsx`, `ResumeProviderPicker.tsx`, `useResumableSessions.ts`.

---

## 8.1 Prerequisite: a shipped ingestion bug this spec makes visible

Not a Spec B defect, but §2 renames a tab whose rollup logic depends on it, and the Cost/Sessions split invites direct comparison between the two.

Two discovery paths disagree about subagent transcripts:

- `watcher.rs:31` watches with `RecursiveMode::Recursive`, so subagent files written **while Switchboard is running** are ingested.
- `walker.rs:16` `discover_jsonl_files` — the startup and backfill scan — is a two-level `read_dir` that skips the `<sessionId>/` directory outright (`if !fmeta.is_file() { continue; }`, line 39). Anything written while the app was closed is **never backfilled**.

Measured on the live database:

```
subagent transcripts on disk:   138
  present in session_events:     13
  never ingested:               125

Tokens in the never-ingested files:
  input:       12,261,121
  output:        1,094,629
  cache-read: 104,520,949
```

`SessionsTab.tsx` already carries `SUBAGENT_SEGMENT = '/subagents/agent-'` and rollup logic keyed on it, commented "subagents' API calls are real" — the frontend is built to display data the backfill path cannot supply. The Cost tab is therefore under-reporting today.

The fix is one level of traversal in `discover_jsonl_files` plus a re-ingest migration. It is sequenced **before** the browser work in the implementation plan, because shipping a second session view while the first under-counts invites a bug report about the discrepancy.

Note the relationship to §3: ingestion **wants** subagent transcripts (their API calls cost money); the browser **must not list** them (they are not resumable sessions). The two paths deliberately diverge, which is why §3 states the exclusion as a rule rather than leaving it implicit.

---

## 9. Rejected alternatives

**Full transcript viewer.** A scrollable rendered conversation with tool-call formatting. Rejected for v1: it needs message rendering, tool-result formatting and virtualized scrolling for the 21.5 MB outlier transcript, and the recap card already answers "what was this about". Revisit if the recap proves insufficient in use.

**Persisting session metadata to SQLite.** A `sessions` table populated during JSONL ingestion. Rejected — but not on latency grounds, which was the original and wrong argument. The cost that matters is *repetition*: without memoization every tab switch re-parses 100.7 MB, since `ExpandedReport` remounts tab components across its slide transition. §1.2 therefore adopts the middle option this section originally skipped — an in-memory memo keyed on `max(mtime)`, which removes the repetition without a migration, a table, or a staleness class.

**Extending the existing Sessions tab in place.** Rejected in favour of the Cost/Sessions split (§2): the accounting view is scoped to a day window and rolls subagents into parents, neither of which suits browsing, and bolting resume onto it would compromise both.

**Deleting transcripts.** Permanently destroys conversation history. Out of scope.

---

## 10. Files touched

**Modified first (prerequisite, §8.1):** `src-tauri/src/jsonl_parser/walker.rs`, plus a re-ingest migration under `src-tauri/src/store/migrations/`

**New (Rust):** `sessions/{mod,scan,recap}.rs`
**New (TS):** `src/sessions/{SessionsBrowserTab,SessionRow,SessionRecapCard,ResumeProviderPicker}.tsx`, `src/sessions/useResumableSessions.ts`, `src/sessions/__tests__/`
**Modified:** `src-tauri/src/lib.rs` (module + both `collect_commands!` lists), `src-tauri/src/commands.rs`, `src/report/ExpandedReport.tsx` (rename `sessions` → `cost`, add the new tab), `src/lib/ipc.ts`, `docs/release-checklist.md`

The existing `src/report/SessionsTab.tsx` is **not** renamed as a file — only its tab label and id change — to keep the diff reviewable. Its exported helpers `modelLabel` and `isHeadlessProject` are reused by the new tab.

---

## 11. Testing

**Rust unit tests** run against fixture JSONL written to a `tempdir`:
- inclusion filter: headless (`project == "-"`), missing `cwd`, and zero-user-turn transcripts are excluded; a normal transcript is included
- title fallback chain: `aiTitle` present → used; absent → first user message truncated
- recap: the **last** of several `away_summary` records wins; the `(disable recaps in /config)` suffix is stripped; absent yields `None` rather than an empty string
- `left_off` is `None` when it equals `asked`
- touched-file extraction: ranked by frequency, capped at 4, overflow counted, absent when no `file_path` inputs exist
- synthetic user turns (content starting with `<`) do not count toward `turns` and never become `asked`
- malformed lines are skipped; a wholly malformed file is omitted rather than panicking
- **subagent exclusion**: a fixture at `<project>/<sessionId>/subagents/agent-x.jsonl` that satisfies every inclusion condition is still not listed (C1 regression)
- **`tool_result` exclusion in both fields**: a session whose final `type: "user"` record is a tool result yields the last *real* message as `left_off`, never the tool output; the same for `asked` when the first `type: "user"` record is a tool result
- **end-to-end provider resolution**, not normalization in isolation: a provider configured `ANTHROPIC_MODEL = "glm-5.2[1m]"` resolves a session recorded as `glm-5.2`. A unit test that only asserts `norm("glm-5.2[1m]") == "glm-5.2"` passes while the feature is broken, because the defect is which operand `norm` is applied to (C2 regression)
- case-insensitive match: provider `MiniMax-M2.7-highspeed` resolves a session recorded in any casing
- resolution via a non-`ANTHROPIC_MODEL` key: `kimi-for-coding-highspeed` configured as `ANTHROPIC_SMALL_FAST_MODEL` resolves
- tie-break: two providers declaring the same model id resolve to the lower `sort_index`, deterministically
- a session whose model matched a since-deleted provider resolves to **unresolved**, not official, even when the id looks Anthropic-style
- `claude-*` → official; unknown → `None`
- the 200-session cap is applied **after** filtering and keeps the most recent by mtime

**Frontend tests** follow the existing `__tests__` pattern: collapsed/expanded rendering, one-row-at-a-time expansion, search flattening the grouping, the two distinct empty states, resume calling the launcher with the resolved provider, and the picker appearing for an unresolved model.

Additionally: the generated resume command always carries `--fork-session` (§7.1), asserted at the launcher boundary so a future refactor cannot drop it.

**Manual smoke** (added to `docs/release-checklist.md`, both platforms): the tab lists real sessions grouped by project; a `glm-5.2` session resolves to the GLM provider; resuming opens a new terminal in the right folder and `/status` shows the expected endpoint; an unconfigured model prompts rather than resuming; a session whose folder was deleted reports it. Critically — **resume a session that is still open in another terminal**, confirm both windows keep working, and confirm the original transcript is unchanged while the forked one appears as a new entry on rescan.
