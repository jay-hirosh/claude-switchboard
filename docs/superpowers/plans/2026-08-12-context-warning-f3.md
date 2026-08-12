# F3 — Context-Window Warning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Warn once when a live session's context crosses 80% of its window — a native notification plus a passive "how full is this session" chip on its "Now running" row.

**Architecture:** A Rust port of `contextWindow.ts`'s window-size lookup (`context_window_for`) computes each session's percentage inside `note_ingest` (F1) — context only changes when the transcript writes, so this is the natural evaluation point. A per-entry `armed` bool (hysteresis: fires once crossing ≥80%, re-arms only below 70%) lives on `LiveEntry` and survives Cooling round-trips the same way `first_seen` already does. `note_ingest`'s return type changes from `()` to `Option<ContextWarning>`; `lib.rs`'s watcher-consumer block (already firing F2's notifications from the same loop) fires this one too, gated on a new Settings toggle. The popover chip is a **pure frontend addition** — `LiveSessionInfo` already carries `context_tokens` and `model` from F1, and the frontend already has its own `windowFor()` in `contextWindow.ts`, so no new binding is needed for the chip itself.

**Tech Stack:** Rust (Tauri v2 backend, tauri-plugin-notification), React 19 + TypeScript, Vitest, cargo test.

**Spec:** `docs/superpowers/specs/2026-08-12-context-warning-f3-design.md`

## Global Constraints

- Thresholds: fires when `armed && pct >= 80`; re-arms when `pct < 70`. Fixed, not user-configurable. New `LiveEntry`s start `armed: true` — a resumed session already above 80% on its very first touch fires immediately, which is correct (better to warn late than never).
- **Correction to the spec's wording**: the spec's §3 claims the Rust port should "mirror the TS module's normalization (strip provider prefixes/[1m] suffix)". Reading the actual `windowFor()` in `src/sessions/contextWindow.ts`, there is **no provider-prefix stripping anywhere in it** — it only strips a trailing `[1m]` suffix (case-insensitively) before checking the model against `NATIVE_1M_MODELS`. Implement `context_window_for` to match what the TS function **actually does**, not the spec's inaccurate paraphrase: `[1m]`-suffix stripping only, nothing else.
- `context_window_for` returns a bare `u64` always (never an "unknown" case) — this is a deliberate, spec-mandated divergence from `windowFor()`, which legitimately returns `total: null` for unrecognized/third-party models. The backend's job is "warn early rather than never," so anything not in the explicit 1M list defaults to 200,000. The **frontend chip does NOT get this same default** — it keeps using `windowFor()`'s real null-safe behavior (hide the chip rather than show a fabricated percentage next to a third-party model name). This asymmetry is intentional; do not try to unify it.
- Context evaluation is scoped to the PARENT transcript's latest event only (`latest_event_for_file`, already parent-only per F1) — no subagent context handling.
- No DB schema change. No new IPC command. `Settings.notify_context_warning: bool`, default `true`, via container-level `#[serde(default)]` + custom `Default` impl only — same pattern F2's `notify_session_finished` already established (empirically verified there; no field-level default fn needed).
- Package manager pnpm. Rust tests: `cd src-tauri && cargo test` (or `cargo test --manifest-path src-tauri/Cargo.toml` from repo root). TS: `pnpm lint`, `pnpm test`.
- Known pre-existing baseline failure (NOT yours): `src/lib/__tests__/theme.test.ts` fails at collection (localStorage error). Bar = no new failures.

---

### Task 1: `context_window_for` + hysteresis in `note_ingest` (pure logic)

**Files:**
- Modify: `src-tauri/src/live_sessions.rs` (constants ~line 8-9, `LiveEntry` struct ~line 18-29, `note_ingest` ~line 127-186, test module)

**Interfaces:**
- Consumes: nothing new — same `latest_event_for_file`/`live_session_totals` F1 already calls.
- Produces: `fn context_window_for(model: &str) -> u64` (crate-visible, not `pub` — only `live_sessions.rs` needs it); `pub struct ContextWarning { pub project: String, pub pct: u8 }`; `LiveSessionRegistry::note_ingest(...) -> Option<ContextWarning>` (signature changes from `()` to this — Task 2 updates the one caller). `LiveEntry` gains a private `context_warning_armed: bool` field (internal state, not exposed on `LiveSessionInfo`).

- [ ] **Step 1: Write the failing tests**

Add to `src-tauri/src/live_sessions.rs`'s existing `#[cfg(test)] mod tests` (reuse `fresh()`/`root_and_file()`; add one new seeding helper since the existing `seed()` hardcodes `source_line: 0` for every call, which makes `latest_event_for_file`'s `ORDER BY source_line DESC` ambiguous across multiple seeds on the same file — these tests need a real, increasing sequence):

```rust
    /// Like `seed()`, but with an explicit `source_line` so a test can seed
    /// a SEQUENCE of events on the same file and rely on
    /// `latest_event_for_file` picking the last one deterministically —
    /// `seed()` hardcodes `source_line: 0` for every call, which is fine
    /// for single-seed tests but ambiguous across multiple.
    fn seed_at_line(db: &Db, source_file: &str, tokens: u64, model: &str, line: i64) {
        let ev = StoredSessionEvent {
            ts: Utc::now(),
            project: "proj".into(),
            model: model.into(),
            input_tokens: tokens,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_creation_5m_tokens: 0,
            cache_creation_1h_tokens: 0,
            cost_usd: 0.01,
            source_file: source_file.into(),
            source_line: line,
            event_id: format!("{source_file}:seed:{line}"),
        };
        db.ingest_atomic(source_file, &[ev], &[], 1, 100).unwrap();
    }

    #[test]
    fn context_window_for_native_1m_models() {
        for m in [
            "claude-sonnet-5",
            "claude-opus-4-7",
            "claude-opus-4-8",
            "claude-opus-5",
            "claude-fable-5",
            "claude-mythos-5",
        ] {
            assert_eq!(context_window_for(m), 1_000_000, "{m} should resolve to 1M");
        }
    }

    #[test]
    fn context_window_for_unknown_or_third_party_defaults_to_200k() {
        assert_eq!(context_window_for("claude-sonnet-4-6"), 200_000, "known-but-not-1M Anthropic model");
        assert_eq!(context_window_for("glm-4.6"), 200_000, "third-party model");
        assert_eq!(context_window_for("some-future-model-id"), 200_000, "unrecognized model");
    }

    #[test]
    fn context_window_for_strips_1m_suffix_case_insensitively() {
        assert_eq!(context_window_for("claude-sonnet-4-6[1m]"), 1_000_000);
        assert_eq!(context_window_for("claude-sonnet-4-6[1M]"), 1_000_000);
    }

    #[test]
    fn a_resumed_session_already_above_80_fires_on_its_first_touch() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        let reg = LiveSessionRegistry::default();
        seed_at_line(&db, key, 900_000, "claude-opus-5", 0); // 90% of 1M
        let w = reg.note_ingest(&db, &file, &root, Utc::now());
        let w = w.expect("a fresh entry defaults armed=true, so an already-high session fires immediately");
        assert_eq!(w.pct, 90);
        assert_eq!(w.project, "proj");
    }

    #[test]
    fn context_warning_fires_once_crossing_80_then_does_not_refire_while_still_high() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();

        seed_at_line(&db, key, 790_000, "claude-opus-5", 0); // 79%
        assert!(reg.note_ingest(&db, &file, &root, t0).is_none(), "79% must not fire");

        seed_at_line(&db, key, 810_000, "claude-opus-5", 1); // 81%
        let w = reg.note_ingest(&db, &file, &root, t0 + Duration::seconds(10));
        let w = w.expect("crossing 80% must fire");
        assert_eq!(w.pct, 81);

        seed_at_line(&db, key, 850_000, "claude-opus-5", 2); // 85%, still high
        assert!(
            reg.note_ingest(&db, &file, &root, t0 + Duration::seconds(20)).is_none(),
            "staying above 80% after already firing must not refire"
        );
    }

    #[test]
    fn context_warning_rearms_below_70_and_refires_on_the_next_climb() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();

        seed_at_line(&db, key, 850_000, "claude-opus-5", 0); // 85% — fires
        assert!(reg.note_ingest(&db, &file, &root, t0).is_some());

        seed_at_line(&db, key, 650_000, "claude-opus-5", 1); // 65% — compaction happened
        assert!(
            reg.note_ingest(&db, &file, &root, t0 + Duration::seconds(10)).is_none(),
            "dropping below 70% must not itself fire — it only re-arms"
        );

        seed_at_line(&db, key, 820_000, "claude-opus-5", 2); // climbs past 80% again
        let w = reg.note_ingest(&db, &file, &root, t0 + Duration::seconds(20));
        assert!(w.is_some(), "a re-armed session must fire again on a fresh climb past 80%");
    }

    #[test]
    fn context_warning_armed_state_survives_a_cooling_round_trip() {
        // Same "write during Cooling preserves first_seen" property F1 already
        // relies on — armed state must be preserved the same way, not reset.
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();

        seed_at_line(&db, key, 850_000, "claude-opus-5", 0); // fires, now disarmed
        assert!(reg.note_ingest(&db, &file, &root, t0).is_some());
        reg.prune(t0 + Duration::seconds(121)); // -> Cooling, still disarmed
        seed_at_line(&db, key, 860_000, "claude-opus-5", 1); // write during Cooling, still >80%
        assert!(
            reg.note_ingest(&db, &file, &root, t0 + Duration::seconds(150)).is_none(),
            "a write during Cooling while still disarmed and still >80% must not refire"
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test live_sessions::`
Expected: compile errors — `context_window_for`/`ContextWarning` don't exist; `note_ingest`'s return type doesn't match the new tests' `.is_none()`/`.expect(...)` usage.

- [ ] **Step 3: Implement**

3a. Add constants near the existing ones (after `MIN_NOTIFY_SPAN_SECS`):

```rust
const CONTEXT_WARN_PCT: u32 = 80;
const CONTEXT_REARM_PCT: u32 = 70;

/// Models whose registry entry sets `context.native_1m: true` — MUST stay
/// in sync with `NATIVE_1M_MODELS` in `src/sessions/contextWindow.ts` (that
/// file's comment points back here). Explicit list, not a family prefix:
/// the split runs within families (Sonnet 5 is 1M, Sonnet 4.6 is 200K).
const NATIVE_1M_MODELS: &[&str] = &[
    "claude-sonnet-5",
    "claude-opus-4-7",
    "claude-opus-4-8",
    "claude-opus-5",
    "claude-fable-5",
    "claude-mythos-5",
];

/// Resolves a model id to its context window in tokens. Mirrors
/// `contextWindow.ts::windowFor`'s ACTUAL behavior (only a trailing `[1m]`
/// suffix is stripped — that TS function does no provider-prefix
/// stripping, despite what an earlier draft of this feature's spec
/// claimed). Deliberately always returns a concrete size, never "unknown":
/// unlike the frontend's `windowFor` (which returns `null` for
/// unrecognized/third-party models so the UI never shows a fabricated
/// percentage), a notification's job is to warn early rather than never,
/// so anything outside `NATIVE_1M_MODELS` defaults to 200K.
fn context_window_for(model: &str) -> u64 {
    let lower = model.to_ascii_lowercase();
    let bare = lower.strip_suffix("[1m]").unwrap_or(&lower);
    if NATIVE_1M_MODELS.contains(&bare) {
        1_000_000
    } else {
        200_000
    }
}

/// A live session's context crossed `CONTEXT_WARN_PCT` of its window —
/// worth a "approaching compaction" notification. See
/// `LiveSessionRegistry::note_ingest`.
#[derive(Debug, Clone)]
pub struct ContextWarning {
    pub project: String,
    pub pct: u8,
}
```

3b. Add `context_warning_armed: bool` to `LiveEntry`, after `state`:

```rust
    /// Hysteresis for the context-window warning: true = eligible to fire
    /// the next time `CONTEXT_WARN_PCT` is crossed; false = already fired
    /// for this climb, won't refire until pct drops below
    /// `CONTEXT_REARM_PCT`. New entries start `true` — a resumed session
    /// already above the threshold fires on its first touch.
    context_warning_armed: bool,
```

3c. Rewrite `note_ingest`'s signature and body. Change the signature line to:

```rust
    pub fn note_ingest(
        &self,
        db: &Db,
        touched_file: &Path,
        projects_root: &Path,
        now: DateTime<Utc>,
    ) -> Option<ContextWarning> {
```

Change every early `return;` inside it (the two `let ... else { return; };` guards) to `return None;`. After the existing `let latest = db.latest_event_for_file(&parent_key).ok().flatten();` line, extract `project`/`model`/`context_tokens` as local bindings BEFORE the `sessions.write()` block (they're currently built inline inside the `LiveEntry` literal — pull them out so both the literal and the new pct/hysteresis logic can use them):

```rust
        let project = latest.as_ref().map(|l| l.project.clone()).unwrap_or_default();
        let model = latest.as_ref().map(|l| l.model.clone()).unwrap_or_default();
        let context_tokens = latest.map(|l| l.context_tokens).unwrap_or(0);
```

Inside the `sessions.write()` block, after the existing `is_new_live_period`/`first_seen` lookups, add a third lookup and the hysteresis decision (before the `if is_new_live_period { ... }` / `sessions.insert(...)` that already exist):

```rust
        let was_armed = sessions
            .get(&parent_key)
            .map(|e| e.context_warning_armed)
            .unwrap_or(true);
        let pct = if context_tokens > 0 {
            ((context_tokens as f64 / context_window_for(&model) as f64) * 100.0) as u32
        } else {
            0
        };
        let (warn, now_armed) = if was_armed && pct >= CONTEXT_WARN_PCT {
            (true, false)
        } else if pct < CONTEXT_REARM_PCT {
            (false, true)
        } else {
            (false, was_armed)
        };
```

Update the `LiveEntry` literal inside `sessions.insert(...)` to use the extracted `project`/`model`/`context_tokens` bindings (instead of the inline `latest.as_ref().map(...)` expressions, which are now redundant) and add the new field:

```rust
        sessions.insert(
            parent_key.clone(),
            LiveEntry {
                session_id,
                source_file: parent_key,
                project: project.clone(),
                model,
                total_tokens,
                total_cost_usd,
                context_tokens,
                first_seen,
                last_activity: now,
                state: SessionState::Live,
                context_warning_armed: now_armed,
            },
        );
```

Finally, change the function's tail (it currently ends implicitly after `sessions.insert(...)`, returning `()`) to return the warning:

```rust
        if warn {
            Some(ContextWarning { project, pct: pct.min(100) as u8 })
        } else {
            None
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test live_sessions::`
Expected: compile error — this task's signature change breaks `note_ingest`'s ONE other caller (`lib.rs`'s watcher-consumer block, which currently discards a `()` return and will now discard an `Option<ContextWarning>` — that actually still compiles fine as a discarded expression statement, so unlike F2's `prune()` change, THIS signature change is source-compatible with an unchanged caller). Confirm: run `cargo build` — it should succeed even before Task 2, because Rust allows silently discarding a non-`()` return value from a bare statement. If `cargo build` unexpectedly fails, that means something else broke; investigate rather than assuming it's "the same expected gap" F2's Task 1 had (this task's gap, if any, is different in kind). Then run the full suite: `cargo test` — expect all pre-existing tests plus this task's 7 new ones to pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/live_sessions.rs
git commit -m "feat(live-sessions): detect and report context-window warnings in note_ingest"
```

---

### Task 2: Fire the notification (backend wiring + Settings field)

**Files:**
- Modify: `src-tauri/src/app_state.rs` (`Settings` struct, `Default` impl — same location pattern as `notify_session_finished`)
- Modify: `src-tauri/src/lib.rs` (watcher-consumer block ~line 720-739)

**Interfaces:**
- Consumes: `LiveSessionRegistry::note_ingest` returning `Option<ContextWarning>` from Task 1.
- Produces: `Settings.notify_context_warning: bool` (default `true`). No further interface — Task 3 only needs the field name.

- [ ] **Step 1: Write the failing test**

In `src-tauri/src/app_state.rs`'s `#[cfg(test)] mod settings_tests`, mirror the existing `notify_session_finished` test exactly (check its current exact JSON fixture first — it must now ALSO be missing `notify_context_warning`, proving THAT field defaults correctly too):

```rust
    #[test]
    fn settings_without_notify_context_warning_field_defaults_to_true() {
        // Same legacy-shaped fixture as the sibling notify_session_finished
        // test — missing both notification-toggle fields entirely.
        let json = r#"{
            "polling_interval_secs": 300,
            "stagger_gap_secs": 30,
            "thresholds": [75, 90],
            "payg_threshold": 85,
            "theme": "system",
            "launch_at_login": false,
            "crash_reports": false,
            "preferred_auth_source": null
        }"#;
        let s: Settings = serde_json::from_str(json).expect("legacy settings must still parse");
        assert!(s.notify_context_warning);
    }
```

(Grep the file first to copy the CURRENT exact fixture shape — fields may have shifted since this plan was written; the point is a JSON blob missing both new-in-2026-08-12 toggle fields.)

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test settings_without_notify_context_warning`
Expected: compile error — no such field.

- [ ] **Step 3: Implement**

3a. `src-tauri/src/app_state.rs` — add to `Settings`, after `notify_session_finished`:

```rust
    /// Notify when a session's context crosses 80% of its window. No
    /// field-level `#[serde(default = "...")]` needed — same mechanism as
    /// `notify_session_finished` (container-level `#[serde(default)]` +
    /// this being `true` in the custom `Default` impl below).
    pub notify_context_warning: bool,
```

Add `notify_context_warning: true,` to the custom `Default for Settings` impl, after `notify_session_finished: true,`.

3b. `src-tauri/src/lib.rs` — update the watcher-consumer block. It currently (post-F2) looks like:

```rust
while let Some((path, n)) = rx.recv().await {
    let _ = handle_for_events.emit("session_ingested", n);
    state_for_ingest.live_sessions.note_ingest(
        &state_for_ingest.db,
        &path,
        &root_for_ingest,
        chrono::Utc::now(),
    );
    let _ = handle_for_events.emit(
        "live_sessions_changed",
        state_for_ingest.live_sessions.live_snapshot(),
    );
}
```

Change to capture the return value and fire the notification:

```rust
while let Some((path, n)) = rx.recv().await {
    let _ = handle_for_events.emit("session_ingested", n);
    let context_warning = state_for_ingest.live_sessions.note_ingest(
        &state_for_ingest.db,
        &path,
        &root_for_ingest,
        chrono::Utc::now(),
    );
    let _ = handle_for_events.emit(
        "live_sessions_changed",
        state_for_ingest.live_sessions.live_snapshot(),
    );
    if let Some(w) = context_warning {
        if state_for_ingest.settings.read().notify_context_warning {
            use tauri_plugin_notification::NotificationExt;
            let _ = handle_for_events
                .notification()
                .builder()
                .title(format!("Session context at {}%", w.pct))
                .body(format!("{} — approaching compaction", w.project))
                .show();
        }
    }
}
```

(Grep the current exact block before editing to confirm variable names haven't drifted — `handle_for_events`, `state_for_ingest`, `root_for_ingest` should all still match F1's original naming since F2 didn't touch this specific block.)

- [ ] **Step 4: Run the backend suite**

Run: `cd src-tauri && cargo test`
Expected: all pass. Run `cargo build` and `cargo clippy --all-targets -- -D warnings` to confirm clean compilation — check for any OTHER `Settings { ... }` exhaustive literal the compiler flags (the PAYG and F2 features both hit one in `store/queries.rs`'s round-trip test; fix any flagged site the same way — add the new field with a test-distinguishing value plus a round-trip assertion, not just a default).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/app_state.rs src-tauri/src/lib.rs
# also add src-tauri/src/store/queries.rs if the compiler flagged an exhaustive Settings literal there
git commit -m "feat(live-sessions): fire a native notification when session context crosses 80%"
```

---

### Task 3: Popover chip + Settings toggle (frontend)

**Files:**
- Modify: `src/popover/NowRunningSection.tsx` (`Row` component)
- Modify: `src/popover/__tests__/NowRunningSection.test.tsx` (new cases)
- Modify: `src/sessions/contextWindow.ts` (cross-reference comment only)
- Modify: `src/lib/generated/bindings.ts` (`Settings` type — add `notify_context_warning: boolean; ` after `notify_session_finished`)
- Modify: `src/settings/SettingsPanel.tsx` (Notifications card)
- Modify: `src/settings/__tests__/SettingsPanel.test.tsx` (new case)

**Interfaces:**
- Consumes: `LiveSessionInfo.context_tokens`/`.model` (already exist, from F1); `windowFor` from `src/sessions/contextWindow.ts` (already exists); `Settings.notify_context_warning` from Task 2 (via the bindings edit here).
- Produces: nothing further downstream — final task of the final feature.

- [ ] **Step 1: Write the failing tests**

1a. `src/sessions/contextWindow.ts` — add a one-line cross-reference comment right above `NATIVE_1M_MODELS`'s existing doc comment (no test needed, this is documentation only):

```ts
// Mirrored in Rust at src-tauri/src/live_sessions.rs::NATIVE_1M_MODELS for
// the context-window-warning notification — keep both lists in sync.
```

1b. Append to `src/popover/__tests__/NowRunningSection.test.tsx` (reuse the file's existing `session()` fixture factory):

```tsx
describe('NowRunningSection context chip', () => {
  it('hides the chip below 60%', () => {
    render(
      <NowRunningSection
        sessions={[session({ model: 'claude-opus-5', context_tokens: 500_000 })]} // 50% of 1M
      />,
    );
    expect(screen.queryByText('50%')).toBeNull();
  });

  it('shows a muted chip between 60% and 80%', () => {
    render(
      <NowRunningSection
        sessions={[session({ model: 'claude-opus-5', context_tokens: 650_000 })]} // 65%
      />,
    );
    expect(screen.getByText('65%')).toBeTruthy();
  });

  it('shows a warn-colored chip at 80% and above', () => {
    render(
      <NowRunningSection
        sessions={[session({ model: 'claude-opus-5', context_tokens: 850_000 })]} // 85%
      />,
    );
    const chip = screen.getByText('85%');
    expect(chip.className).toContain('color-warn');
  });

  it('hides the chip for a model with no known window', () => {
    render(
      <NowRunningSection
        sessions={[session({ model: 'glm-4.6', context_tokens: 900_000 })]} // huge, but unknown window
      />,
    );
    expect(screen.queryByText(/\d+%/)).toBeNull();
  });
});
```

(If the existing `session()` fixture factory in this test file doesn't accept `model`/`context_tokens` overrides, check its exact current signature and adjust — it's a `Partial<LiveSessionInfo>` overrides object per F1's own test file, so this should already work as written.)

1c. Append to `src/settings/__tests__/SettingsPanel.test.tsx` (mirror the `notify_session_finished` toggle test exactly, and add `notify_context_warning: true` to the file's `baseSettings` fixture — check its current exact field list first, it must include every `Settings` field or the component may hit an untyped/wrong render path):

```tsx
  it('renders and toggles the context-warning notification setting', () => {
    render(<SettingsPanel />);
    const toggle = screen.getByLabelText(/context warnings/i) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
    fireEvent.click(toggle);
    expect(toggle.checked).toBe(false);
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `pnpm exec vitest run src/popover/__tests__/NowRunningSection.test.tsx src/settings/__tests__/SettingsPanel.test.tsx`
Expected: FAIL — no chip rendered at all yet; no toggle labeled "context warnings" yet; `baseSettings` missing the new field (TypeScript error surfaced as a test failure or via `pnpm lint`).

- [ ] **Step 3: Implement**

3a. `src/popover/NowRunningSection.tsx` — add the import and a small pct helper, then render the chip in `Row`:

```tsx
import { windowFor } from '../sessions/contextWindow';
```

```tsx
function contextPct(session: LiveSessionInfo): number | null {
  if (session.context_tokens === 0) return null;
  const { total } = windowFor(session.model);
  if (total === null) return null;
  return Math.min(100, Math.round((session.context_tokens / total) * 100));
}
```

In `Row`, after the elapsed `<span>`:

```tsx
      {(() => {
        const pct = contextPct(session);
        if (pct === null || pct < 60) return null;
        return (
          <span
            className={`mono shrink-0 text-[length:var(--text-micro)] tabular-nums ${
              pct >= 80 ? 'text-[color:var(--color-warn)]' : 'text-[color:var(--color-text-muted)]'
            }`}
          >
            {pct}%
          </span>
        );
      })()}
```

(An inline IIFE keeps `pct` computed once per row without adding a second component; if this file's existing conventions elsewhere prefer extracting such blocks into a named sub-component instead, e.g. `<ContextChip session={session} />`, match whatever convention the file already leans toward — check `Row`'s existing structure before choosing.)

3b. `src/lib/generated/bindings.ts` — add `notify_context_warning: boolean; ` to the `Settings` type, immediately after `notify_session_finished: boolean; ` (matching Rust struct field order, per this file's established within-type convention).

3c. `src/settings/SettingsPanel.tsx` — add a second `Toggle` in the Notifications card, immediately after the "Session finished" toggle (check its exact current location first — it was the last thing F2 added, right before the "Notifications fire once per bucket reset cycle" footer):

```tsx
          <Toggle
            label="Context warnings"
            description="Notify when a session's context passes 80% of its window."
            checked={local.notify_context_warning}
            onChange={(e) => update('notify_context_warning', e.target.checked)}
          />
```

- [ ] **Step 4: Run the full frontend gate**

Run: `pnpm exec vitest run src/popover/__tests__/NowRunningSection.test.tsx src/settings/__tests__/SettingsPanel.test.tsx` → PASS. Then `pnpm lint` → clean. Then `pnpm test` → no new failures beyond the known pre-existing `theme.test.ts` collection error.

- [ ] **Step 5: Commit**

```bash
git add src/popover/NowRunningSection.tsx src/popover/__tests__/NowRunningSection.test.tsx src/sessions/contextWindow.ts src/lib/generated/bindings.ts src/settings/SettingsPanel.tsx src/settings/__tests__/SettingsPanel.test.tsx
git commit -m "feat(popover): add the context-window warning chip and settings toggle"
```

---

## Verification checklist (after all tasks)

- `cargo test` (from `src-tauri/`) fully green, `cargo clippy --all-targets -- -D warnings` clean; `pnpm lint` clean; `pnpm test` no new failures.
- Manual: a live session whose context climbs past 80% of its resolved window shows an amber `NN%` chip on its "Now running" row and fires one native notification titled "Session context at NN%"; a session that compacts (context drops) and climbs past 80% again fires a second time; a third-party/relay model's row never shows a chip (window unknown) even though its backend notification path still treats it as 200K.
- No DB schema change; no new IPC command.

This is the last of the four features from the 2026-08-12 roadmap (PAYG budget alerts, F1, F2, F3) — after this merges, the "Live awareness" phase of `docs/superpowers/roadmap-2026-08-12.md` is complete.
