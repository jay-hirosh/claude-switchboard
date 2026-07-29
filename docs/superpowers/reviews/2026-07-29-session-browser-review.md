# Adversarial Review — Session Browser & One-Click Resume (2026-07-29)

**Reviews:** `docs/superpowers/specs/2026-07-29-session-browser-design.md`
**Reviewer stance:** Adversarial — find problems before implementation burns time.
**Verdict ordering:** Critical issues block implementation. High issues must be fixed before the section they affect is built. Medium issues are material but non-blocking. Low issues are optional.

**Verification basis:** the live `~/.claude/projects/` corpus (298 files / 131.7 MB), `src-tauri/src/jsonl_parser/{walker,watcher,record}.rs`, `src/report/{SessionsTab,ExpandedReport}.tsx`, the Switchboard SQLite database, and the Spec A implementation plan at `939e81f`. All checks read-only; nothing was modified.

---

## 1. Summary verdict

**Ship with changes, and sequence three of them first.** The empirical work in this spec is markedly better than Spec A's — every coverage percentage, the parse timing, and the median turn count reproduce exactly against the live corpus, and §6's less-obvious justifications are correct. The architecture is proportionate and the Spec A dependency is real and already discharged.

The failures are concentrated in three places where a measurement was taken through too narrow an aperture:

- **C1** — a flat glob made 138 files invisible, producing a sidechain claim that is precisely backwards.
- **C2** — normalization is applied to the operand that was already normalized, so the two heaviest third-party providers never resolve.
- **H1** — a "100% available" figure that holds only under a filter §4 never restates and §11 never tests.

C1 additionally exposed **H2**, a live under-counting bug in shipped ingestion: 125 subagent transcripts holding 12.3M input and 104.5M cache-read tokens that the backfill path has never read, against a Cost tab already coded to display them.

Revise §3, §4.3, §6 and §11 before writing the implementation plan.

---

## 2. What holds up

Re-derived independently against the live corpus. These need no further defence:

| Claim | Spec | Measured |
|---|---|---|
| Substantive sessions | 70 | **70** |
| `aiTitle` coverage | 71% | **71%** |
| `message.model` coverage | 84% | **84%** |
| First/last user message | 100% | **100%** |
| Touched files | 61% | **61%** |
| Median turn count | 3 | **3** |
| Full parse | 0.35 s | **0.37 s** (Python; Rust will be faster) |

Also verified sound:

- **§6 lowercasing is genuinely required.** The MiniMax launch script sets `MiniMax-M2.7-highspeed` — mixed case, recorded as-is.
- **§6 checking all six model env keys is required.** 519 recorded events are `kimi-for-coding-highspeed`, which the Kimi script sets as `ANTHROPIC_SMALL_FAST_MODEL`, not `ANTHROPIC_MODEL`. A single-key match would miss them.
- **§7's Spec A dependency is real.** `resume_session_id` and `--resume` rendering are implemented with tests in the Spec A plan (lines 884, 903, 949, 1095, 1176).
- **§10's "both `collect_commands!` lists"** is accurate — `lib.rs:160` and `lib.rs:197`.
- **§2's tab rename is low-risk.** Tab state is `useState`, never persisted to `localStorage`; `TAB_COMPONENTS[activeTab] ?? SessionsTab` already falls back safely. `prevTabRef` drives only slide direction.
- **§3's `<`-prefix heuristic.** Across every user message in the corpus, zero real user messages begin with `<` that are not known injection markers.
- **`<system-reminder>` contamination of *Asked* is negligible** — 1 affected text block corpus-wide, 3 multi-block user messages.

---

## 3. Critical issues

### C1. §3's sidechain claim is inverted, and the 138 files it hides all pass the inclusion filter

**Where:** §3 ("Sidechains"), §1.2 header ("160 local transcripts … 100.7 MB"), §8 (`scan.rs` — "directory walk").

**Problem:** The spec states:

> **Sidechains.** Subagent turns are recorded as inline records within the parent transcript, not as separate files (measured: 0 sidechain-only files). No special handling is required.

Both halves are false. Subagent turns are recorded **exclusively** as separate files at `<project>/<sessionId>/subagents/agent-*.jsonl`, and **not at all** inline:

```
sessions with a subagents/ dir: 7
  4a164a16  agents=12   parent_lines=1043   inline_sidechain=0
  52ecf62a  agents=6    parent_lines=256    inline_sidechain=0
  8723ad3b  agents=5    parent_lines=48     inline_sidechain=0
  b9ee113b  agents=8    parent_lines=1227   inline_sidechain=0
  2d4bb438  agents=4    parent_lines=2499   inline_sidechain=0
  80e4f14d  agents=1    parent_lines=57     inline_sidechain=0
  5351b367  agents=102  parent_lines=1636   inline_sidechain=0
```

There are **138** such files. The "0 sidechain-only files" measurement is an artifact of scanning `~/.claude/projects/*/*.jsonl` — a flat glob that structurally cannot reach a third level. Depth distribution of the real corpus:

```
160 files at depth 2   ← what the spec measured
138 files at depth 4   ← subagents/, invisible to the flat glob
```

This matters because **all 138 pass §3's inclusion filter**. Running the three stated conditions against them:

```
subagent files: 138
  with cwd: 138
  PASSING Spec B section-3 inclusion filter: 138

  agent-aaef14b63cacc7e2c.jsonl
    cwd=/Users/feixu/Developer/claude-usage-gh/claude-switchboard
    asked="I'm reviewing a design spec at docs/superpowers/specs/2026-07-26-trend"
```

**Failure scenario:** §8 describes `scan.rs` as a "directory walk". An implementer reaching for `walkdir` — the idiomatic Rust choice and the plain reading of that phrase — gets **208 rows instead of 70**. The single Stoner session contributes 102 phantom entries that dominate the list. Each renders with a plausible title and *Asked* text, and each gets a Resume button that runs `claude --resume <agentId>` against an ID that is not a session.

The correct behaviour happens only if the implementer copies `walker.rs`'s two-level `read_dir`, and §8 gives no reason to do so — the spec explicitly says none is needed.

**Fix:** Exclude `subagents/` explicitly in `scan.rs`, state it in §3 as a rule rather than an absence, and add a §11 test asserting a subagent transcript is not listed. Correct the §1.2 header to describe the corpus as top-level transcripts excluding sidechains.

---

### C2. §6 normalizes the wrong side; GLM and k3 resolve to *unresolved*

**Where:** §6 (resolution diagram and the paragraph immediately following).

**Problem:** The diagram applies `normalize()` to the session's model, then exact-matches against the provider's raw configured value:

```
normalize(model)   lowercase, strip a trailing [..] context modifier
   ├─ exact match against any provider's ANTHROPIC_MODEL, …
```

But Claude Code already strips `[1m]` before writing the transcript — §6 says so itself in the next paragraph. The session side therefore arrives pre-normalized, and stripping it again is a no-op. The value that still carries the suffix is the **provider's**:

```
launch script config:              transcript records:
  ANTHROPIC_MODEL="glm-5.2[1m]"  →   glm-5.2   (710 events)
  ANTHROPIC_MODEL="k3[1m]"       →   k3        (1289 events)
```

`normalize("glm-5.2")` = `"glm-5.2"`, compared against `"glm-5.2[1m]"` → no match → unresolved.

**Failure scenario:** every GLM and k3 session falls through to the §7 picker on every resume — the two heaviest third-party providers in the corpus, and precisely the ones §6's own prose names. The section diagnoses the hazard correctly ("A naive exact match would fail on precisely the two providers used most") and then implements the fix on the wrong operand.

Note that §11's test as written — `model normalization: glm-5.2[1m] → glm-5.2` — would **pass** while the feature stays broken, because it exercises the normalizer in isolation rather than the match.

**Fix:** Normalize both sides before comparison. Change the §11 test to assert end-to-end resolution: a provider configured with `glm-5.2[1m]` must resolve a session recorded as `glm-5.2`.

---

## 4. High issues

### H1. §4.3's "Left off — 100% available" holds only under an unstated filter, and §11 does not test it

**Where:** §4.3 ("Left off"), §11 (Rust unit tests).

**Problem:** "Last user message" is not the last `type: "user"` record. Tool results are also `type: "user"`, carrying no text block at all. On the 70 sessions the browser will actually list:

```
substantive sessions: 70
  where the LAST type:user record is not a real user message: 41 (59%)
```

**Failure scenario:** a naive implementation renders `Left off "This command requires approval"` or a raw directory listing — or an empty field — on the **majority** of listed rows. Two real examples from the corpus:

```
[{"type":"tool_result","content":"This command requires approval","is_error":true,…}]
[{"tool_use_id":"toolu_01PReq…","type":"tool_result","content":"Applications\nLibrary\nS…"}]
```

The 100% figure is achievable — it reproduces — but only by reapplying §3's real-user-message filter during *extraction*, not merely for *inclusion*. §4.3 presents 100% as a property of the data rather than of the filter. §11 covers "synthetic user turns … never become `asked`" and says nothing about `left_off` and nothing about `tool_result` in either field.

**Fix:** State in §4.3 that both *Asked* and *Left off* draw from the filtered real-user-message sequence, and add §11 tests for `tool_result` exclusion in both fields.

---

### H2. Shipped ingestion under-counts subagent usage (pre-existing; surfaced by C1)

**Where:** not a Spec B defect — `src-tauri/src/jsonl_parser/walker.rs:16`. Recorded here because §2 renames a tab that claims subagent rollup.

**Problem:** Two discovery paths disagree. `watcher.rs:31` watches with `RecursiveMode::Recursive`, so subagent transcripts written **while Switchboard is running** are ingested. `walker.rs:16` `discover_jsonl_files` — the startup/backfill scan — is a two-level `read_dir` that skips the `<sessionId>/` directory outright (`!fmeta.is_file()`, line 39). Anything written while the app was closed is never backfilled.

The database confirms the asymmetry:

```
subagent transcripts on disk:   138
  present in DB:                 13
  NEVER ingested:               125

Tokens in the never-ingested files:
  input:       12,261,121
  output:       1,094,629
  cache-read: 104,520,949
```

`SessionsTab.tsx` already carries `SUBAGENT_SEGMENT = '/subagents/agent-'` and rollup logic keyed on it (lines 117–128, 259–284), commented "subagents' API calls are real" — so the frontend is built to display data the backfill path cannot supply.

**Fix:** one line of traversal in `discover_jsonl_files`, plus a re-ingest. Worth doing before Spec B ships, since a Sessions/Cost split invites direct comparison between the two tabs.

---

## 5. Medium issues

### M1. The 200-file cap turns hostile once C1 is resolved by recursing

**Where:** §3 ("Bounding").

§3 caps the scan at "the 200 most recently modified transcripts" and frames it as precautionary. With recursion the corpus is 298 files, so the cap is **already binding** — and because it applies to files *scanned*, before the inclusion filter, the 102 Stoner subagent transcripts would consume half the budget and evict real sessions. Whichever way C1 lands, state whether the cap applies before or after filtering; post-filter is the only version that does what the prose promises.

### M2. §9 rejects caching on the wrong axis

**Where:** §9 ("Persisting session metadata to SQLite"), §1.2 ("No new table, no cache, no invalidation logic").

The argument is latency: "a full scan is 0.35 s — a cache would add a migration, an invalidation path and a staleness class of bug to save a third of a second." The latency is not the problem; the repetition is. §8's `list_resumable_sessions()` re-reads and re-parses 100.7 MB per call, and `ExpandedReport` remounts tab components across its slide transition, so tab-switching re-reads the corpus. On a menu-bar app running all day that is real disk and battery cost.

The middle option §9 skips is an in-memory memo in `AppState` keyed on `max(mtime)` — no migration, no table, no staleness class, roughly fifteen lines. Also note the 0.35 s was measured warm; the first scan after boot will be slower.

### M3. §6 has no tie-break, and provider deletion silently reroutes

**Where:** §6.

"Exact match against **any** provider's" model keys is ambiguous when two providers declare the same id — easy to hit, since the MiniMax script sets `MiniMax-M2.7-highspeed` across five keys and duplicating a provider row creates the collision immediately. Specify deterministic ordering (`sort_index`).

Separately: the `claude-*` → official rule sits *after* exact match, which is correct — but it means deleting a provider that used Anthropic-style model ids silently reclassifies all its past sessions as official, and §7 then resumes them on Anthropic **without prompting**. That is exactly the case §1.3 promises to prevent ("Never silently resume on the wrong model").

### M4. §7's picker crosses a boundary Spec A flagged

**Where:** §7 step 3.

The picker lets the user resume an unresolved session on any configured provider, defaulting to a guess. Spec A §9 established that switching provider mid-conversation breaks thinking-block signatures (`invalid_thinking_signature` → `tengu_thinking_signature_strip_retry`, discarding the reasoning trail) and cold-starts the prompt cache. The picker makes that a one-click default action with no warning.

Given §1.2 says 16% of sessions have no usable model — and C2 currently routes GLM and k3 there as well — this path will see heavy traffic. A one-line caution when the chosen provider differs from the recorded model would cover it.

### M5. The corpus description is wrong independently of C1

**Where:** §1.2 header and denominator note.

"160 local transcripts in `~/.claude/projects/` (100.7 MB)" describes the flat glob, not the directory (298 files / 131.7 MB). Every percentage is correct *for that set*, but the denominator note goes to some trouble reconciling with Spec A while both specs share the same undercount. Restate as "160 top-level transcripts (excluding 138 subagent sidechains)" so the number stops implying completeness.

---

## 6. Low issues

- **Basename collisions in *Touched*** (§4.3). Up to 4 basenames; in this repo that often yields `mod.rs · +3 more`. Consider `parent/name` for ambiguous basenames.
- **Search scope** (§5). Matches title, project, branch, model, and *Asked* — not *Left off* or touched files. "Left off" is frequently the memorable thing about an abandoned session.
- **`<` heuristic** (§3). Measured 0 false positives, so it is fine to ship — but it is a prefix guess standing in for a structural property. Matching known markers (`<system-reminder>`, `<command-name>`, `<local-command-stdout>`) is the same amount of code and will not misfire on pasted markup.
- **`resolve_provider_for_model(model, providers)`** (§8) is a pure function whose provider list the frontend already holds; exposing it over IPC per row buys nothing over implementing it in TS beside `modelLabel`.

---

## 7. Scope

One plan is right. Spec B genuinely is mostly UI, the §8 module split is proportionate, and the Spec A dependency is real and already discharged in the plan.

But **sequence C1, C2 and H1 first, as backend work rather than polish.** C1 determines what `scan.rs` returns at all; C2 determines whether the resume path works for the two heaviest providers; H1 is a third fix in the same file (`recap.rs`). All three are cheap now and expensive to discover after the UI is built on top of them.

---

## Appendix — reproduction

Corpus shape and the depth split behind C1:

```bash
find ~/.claude/projects -name '*.jsonl' | wc -l                       # 298
find ~/.claude/projects -name '*.jsonl' | awk -F/ '{print NF-1}' \
  | sort | uniq -c                                                    # 160 @2, 138 @4
ls ~/.claude/projects/*/*/subagents/*.jsonl | wc -l                   # 138
```

Inline-sidechain check behind C1 (expects `isSidechain` records in parents; finds none):

```bash
python3 -c "
import json,glob,os
for sd in sorted(glob.glob(os.path.expanduser('~/.claude/projects/*/*/subagents'))):
    sess=os.path.basename(os.path.dirname(sd))
    parent=os.path.join(os.path.dirname(os.path.dirname(sd)), sess+'.jsonl')
    n=sum(1 for l in open(parent,errors='replace') if l.strip() and json.loads(l).get('isSidechain'))
    print(sess[:8], 'agents=%d'%len(glob.glob(sd+'/*.jsonl')), 'inline_sidechain=%d'%n)"
```

Model-id mismatch behind C2:

```bash
grep -h 'ANTHROPIC_MODEL=' ~/Developer/scripts/start-claude-*.sh   # glm-5.2[1m], k3[1m]
# vs message.model in transcripts:                                 # glm-5.2, k3
```

Never-ingested subagent tokens behind H2:

```bash
sqlite3 "$HOME/Library/Application Support/com.claude-switchboard.ClaudeSwitchboard/data.db" \
  "SELECT COUNT(DISTINCT source_file) FROM session_events
   WHERE source_file LIKE '%/subagents/agent-%';"                    # 13 of 138
```
