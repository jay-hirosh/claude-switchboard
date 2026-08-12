# F2 — Session-Finished Notifications Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fire one native notification when a session that ran ≥10 minutes goes quiet for good — F1's Cooling→Departed transition, extended with a span floor and a one-shot dedup set.

**Architecture:** `LiveSessionRegistry::prune` (F1) currently uses `HashMap::retain` to silently drop Departed entries. This plan changes its signature to also collect and return the entries being dropped, filtered to ones that both cleared the 10-minute span floor and haven't already fired — a `Vec<FinishedSession>`. `lib.rs`'s prune tick (already reading settings for other purposes) checks `Settings.notify_session_finished` and fires one native notification per returned session, reusing the notifier's existing `humanize_duration` helper (promoted to `pub(crate)`).

**Tech Stack:** Rust (Tauri v2 backend, tokio, tauri-plugin-notification), React 19 + TypeScript, Vitest, cargo test.

**Spec:** `docs/superpowers/specs/2026-08-12-session-finished-f2-design.md`

## Global Constraints

- Span floor: `(last_activity − first_seen) ≥ 600` seconds, measured on registry residency this app run (matches F1's `first_seen` semantics exactly — not the transcript's real age).
- Dedup: in-memory only, one `HashSet<String>` of session_ids inside `LiveSessionRegistry` — no DB table, no migration. A session that departs, then a NEW registry entry for the same transcript starts later (fresh `first_seen`) may notify again — that's a new run of work, not a re-notify of the same departure.
- The dedup insert happens in `prune()` itself, unconditionally on span-eligible departures — independent of whether `notify_session_finished` is on at that moment. The setting only gates whether `lib.rs` actually calls the OS notification API; flipping it back on later must NOT retroactively notify for a session that already departed while it was off.
- Notification title: exactly `Claude Code session finished`. Body: `{project} — {cost}, {duration}` when `total_cost_usd > 0.0`, else `{project} — {duration}` (cost segment omitted entirely, not shown as `$0.00`). Duration uses `humanize_duration`'s existing `"Xh Ym"` / `"Ym"` format (space-separated, unpadded) — reuse it, do not reinvent a duration formatter.
- `Settings.notify_session_finished: bool`, default `true`, set in the custom `Default` impl (a bare `#[serde(default)]` on a `bool` field defaults to `false`, which is wrong here — see the existing `payg_threshold` field's doc comment for why the struct-level default alone isn't enough for non-`false` defaults).
- Not gated on active account slot — sessions aren't account-scoped, unlike every existing notifier bucket.
- No DB schema change. No new IPC command.
- Package manager pnpm. Rust tests: `cd src-tauri && cargo test` (or `cargo test --manifest-path src-tauri/Cargo.toml` from repo root). TS: `pnpm lint`, `pnpm test`.
- Known pre-existing baseline failure (NOT yours): `src/lib/__tests__/theme.test.ts` fails at collection (localStorage error). Bar = no new failures.

---

### Task 1: `prune()` returns finished sessions (pure logic)

**Files:**
- Modify: `src-tauri/src/live_sessions.rs` (`LiveSessionRegistry` struct ~line 56-59, `prune` ~line 152-182, test module)

**Interfaces:**
- Consumes: nothing new — same `SessionState`/`LiveEntry` internals F1 already built.
- Produces: `pub struct FinishedSession { pub project: String, pub total_cost_usd: f64, pub live_span_secs: i64 }`; `LiveSessionRegistry::prune(&self, now: DateTime<Utc>) -> (bool, Vec<FinishedSession>)` — the `bool` keeps its current F1 meaning (Live set changed, for the `live_sessions_changed` emit); the `Vec` is newly-departed sessions eligible for a one-shot notification (span ≥ 600s, not previously notified). Task 2 consumes both.

- [ ] **Step 1: Write the failing tests**

Add to `src-tauri/src/live_sessions.rs`'s existing `#[cfg(test)] mod tests` (reuse `fresh()`/`seed()`/`root_and_file()` helpers already there):

```rust
    #[test]
    fn ten_minute_session_departs_with_a_finished_entry() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &file, &root, t0);
        // Still active 9 min in — no departure yet.
        reg.note_ingest(&db, &file, &root, t0 + Duration::seconds(540));
        let (_, finished) = reg.prune(t0 + Duration::seconds(540) + Duration::seconds(301));
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].live_span_secs, 540);
    }

    #[test]
    fn eight_minute_session_departs_without_a_finished_entry() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &file, &root, t0);
        reg.note_ingest(&db, &file, &root, t0 + Duration::seconds(480)); // 8 min
        let (_, finished) = reg.prune(t0 + Duration::seconds(480) + Duration::seconds(301));
        assert!(finished.is_empty(), "under the 10-minute floor must not notify");
    }

    #[test]
    fn a_finished_session_is_reported_exactly_once() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &file, &root, t0);
        reg.note_ingest(&db, &file, &root, t0 + Duration::seconds(600));
        let departed_at = t0 + Duration::seconds(600) + Duration::seconds(301);
        let (_, first) = reg.prune(departed_at);
        assert_eq!(first.len(), 1);
        // A second prune pass over the now-empty registry must not re-report it.
        let (_, second) = reg.prune(departed_at + Duration::seconds(30));
        assert!(second.is_empty());
    }

    #[test]
    fn write_during_cooling_delays_departure_and_still_counts_full_span() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &file, &root, t0);
        reg.note_ingest(&db, &file, &root, t0 + Duration::seconds(600)); // 10 min mark
        // Goes quiet, transitions to Cooling...
        let (_, mid) = reg.prune(t0 + Duration::seconds(600) + Duration::seconds(121));
        assert!(mid.is_empty(), "Cooling is not departure");
        // ...but resumes activity before fully departing.
        let t_resume = t0 + Duration::seconds(600) + Duration::seconds(200);
        reg.note_ingest(&db, &file, &root, t_resume);
        // Now quiet again long enough to actually depart — span must be
        // measured from the ORIGINAL first_seen, not reset by the Cooling dip.
        let (_, finished) = reg.prune(t_resume + Duration::seconds(301));
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].live_span_secs, (t_resume - t0).num_seconds());
    }

    #[test]
    fn finished_session_carries_project_and_cost() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5"); // seed()'s mk_event sets cost_usd: 0.01
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &file, &root, t0);
        reg.note_ingest(&db, &file, &root, t0 + Duration::seconds(600));
        let (_, finished) = reg.prune(t0 + Duration::seconds(600) + Duration::seconds(301));
        assert_eq!(finished[0].project, "proj");
        assert!(finished[0].total_cost_usd > 0.0);
    }
```

(If `seed()`'s helper doesn't set a distinguishable `project`/`cost_usd`, check its current definition in the test module first and adjust the assertions to match its actual fixture values rather than guessing — the point of this test is only "the fields are threaded through," not a specific number.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test live_sessions::`
Expected: compile errors — `prune` still returns a bare `bool`, not a tuple; `FinishedSession` doesn't exist.

- [ ] **Step 3: Implement**

Add near `LiveEntry` (before `LiveSessionRegistry`):

```rust
/// A session that just departed (Cooling -> removed) with enough live span
/// to be worth a "session finished" notification, and not already
/// reported. See `LiveSessionRegistry::prune`.
#[derive(Debug, Clone)]
pub struct FinishedSession {
    pub project: String,
    pub total_cost_usd: f64,
    /// Wall-clock span this app run watched the session (last_activity -
    /// first_seen), in seconds — NOT the transcript's real age.
    pub live_span_secs: i64,
}

/// Sessions live at least this long (in the registry, this app run) before
/// their departure is worth a notification — short-lived sessions (a couple
/// of quick questions) would otherwise fire noise on every departure.
const MIN_NOTIFY_SPAN_SECS: i64 = 600;
```

Change `LiveSessionRegistry`'s struct to add the dedup set:

```rust
#[derive(Default)]
pub struct LiveSessionRegistry {
    sessions: RwLock<HashMap<String, LiveEntry>>,
    /// session_ids already reported as finished this app run — see `prune`.
    notified: RwLock<std::collections::HashSet<String>>,
}
```

Rewrite `prune`:

```rust
    /// Walks every entry: Live -> Cooling at `LIVE_QUIET_SECS` quiet,
    /// Cooling -> removed at `COOLING_QUIET_SECS` quiet. Call on a timer
    /// (the 30s tick in lib.rs); pure state transition, no I/O.
    ///
    /// Returns `(changed, finished)`: `changed` is true if the Live set
    /// changed (any entry left Live via Cooling or removal) — lets the
    /// caller decide whether to emit `live_sessions_changed` without a
    /// separate before/after snapshot comparison that could race a
    /// concurrent `note_ingest`. `finished` is newly-departed sessions
    /// whose live span cleared `MIN_NOTIFY_SPAN_SECS` and haven't already
    /// been reported — the caller (lib.rs) decides whether to actually
    /// fire a notification for them, gated on `Settings.notify_session_finished`.
    pub fn prune(&self, now: DateTime<Utc>) -> (bool, Vec<FinishedSession>) {
        let mut sessions = self.sessions.write();
        let before_live = sessions
            .values()
            .filter(|e| e.state == SessionState::Live)
            .count();

        let mut departing = Vec::new();
        sessions.retain(|_, e| {
            let quiet = (now - e.last_activity).num_seconds();
            if quiet >= COOLING_QUIET_SECS {
                departing.push((
                    e.session_id.clone(),
                    FinishedSession {
                        project: e.project.clone(),
                        total_cost_usd: e.total_cost_usd,
                        live_span_secs: (e.last_activity - e.first_seen).num_seconds(),
                    },
                ));
                false
            } else {
                if quiet >= LIVE_QUIET_SECS {
                    e.state = SessionState::Cooling;
                }
                true
            }
        });

        let after_live = sessions
            .values()
            .filter(|e| e.state == SessionState::Live)
            .count();
        drop(sessions);

        let mut notified = self.notified.write();
        let finished: Vec<FinishedSession> = departing
            .into_iter()
            .filter(|(id, f)| f.live_span_secs >= MIN_NOTIFY_SPAN_SECS && notified.insert(id.clone()))
            .map(|(_, f)| f)
            .collect();

        (before_live != after_live, finished)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test live_sessions::`
Expected: PASS (5 new tests + all existing F1 tests still passing — the signature change means the compiler will flag every OTHER call site of `prune`, including `lib.rs`'s prune-tick spawn; that call site is fixed in Task 2, so a full-workspace `cargo build` will still fail after this task alone — that's expected and fine, this task's own test target compiles and passes in isolation via `cargo test live_sessions::` which only needs the crate's lib target to typecheck as a whole. If `cargo test live_sessions::` itself fails to compile because `lib.rs` doesn't build, that confirms the expected coupling — proceed to Task 2 immediately, don't try to make Task 1 "green" in total isolation).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/live_sessions.rs
git commit -m "feat(live-sessions): prune() returns newly-finished, notification-eligible sessions"
```

---

### Task 2: Fire the notification (backend wiring + Settings field)

**Files:**
- Modify: `src-tauri/src/app_state.rs` (`Settings` struct ~line 28-52, `Default` impl ~line 55-70)
- Modify: `src-tauri/src/notifier/rules.rs` (`humanize_duration` ~line 72-81 — visibility only)
- Modify: `src-tauri/src/lib.rs` (prune-tick spawn — the block Minor-C's fix in F1 left as `let changed = ...prune(...); if changed { emit(...) }`)

**Interfaces:**
- Consumes: `LiveSessionRegistry::prune` returning `(bool, Vec<FinishedSession>)` from Task 1; `crate::notifier::rules::humanize_duration(d: chrono::Duration) -> String` (promoted to `pub(crate)`).
- Produces: `Settings.notify_session_finished: bool` (default `true`). No new public interface for later tasks — Task 3 only needs the Settings field name, already defined here.

- [ ] **Step 1: Write the failing test**

In `src-tauri/src/app_state.rs`'s `#[cfg(test)] mod settings_tests`, add (mirroring the existing `settings_without_payg_threshold_field_defaults_to_85`-style test — check its exact current name/shape first and match the pattern):

```rust
    #[test]
    fn settings_without_notify_session_finished_field_defaults_to_true() {
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
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!(s.notify_session_finished);
    }
```

(Match this fixture's JSON keys exactly against the struct's CURRENT field list — grep the file first, since fields may have shifted since this plan was written; the point is a JSON blob missing `notify_session_finished` entirely.)

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test settings_without_notify_session_finished`
Expected: compile error — no such field.

- [ ] **Step 3: Implement**

3a. `src-tauri/src/app_state.rs` — the `Settings` struct already carries a container-level `#[serde(default)]` (confirmed: this is what already makes `payg_threshold`'s `u8` default to `85` — not `0` — for a JSON blob missing that key, with NO field-level attribute at all). Serde's container-level `#[serde(default)]` fills any missing field from `Settings::default()`, per field, using the type's real `Default` impl — not a bare `bool`'s own zero value. So `notify_session_finished` needs no field-level `#[serde(default = "...")]` fn; it only needs to exist in the struct and be set correctly in the custom `Default` impl:

```rust
    /// Notify when a session that ran >= 10 minutes goes quiet for good.
    /// No field-level `#[serde(default = "...")]` needed — the struct's
    /// container-level `#[serde(default)]` (above) already fills any
    /// missing field from this type's own `Default` impl below, same
    /// mechanism that already makes `payg_threshold` default to 85 (not 0)
    /// with no field-level attribute of its own.
    pub notify_session_finished: bool,
```

And in the custom `Default for Settings` impl, add `notify_session_finished: true,` (order matching the struct field order). This is the ONLY change needed to get the default right — do not add a field-level default function; Task 1's test in Step 1 exists specifically to prove this empirically rather than trust the doc comment's claim.

3b. `src-tauri/src/notifier/rules.rs` — change `fn humanize_duration` to `pub(crate) fn humanize_duration`.

3c. `src-tauri/src/lib.rs` — update the prune-tick spawn. It currently looks like (from F1's fix wave):

```rust
let changed = prune_state.live_sessions.prune(chrono::Utc::now());
if changed {
    use tauri::Emitter;
    let _ = prune_handle.emit("live_sessions_changed", prune_state.live_sessions.live_snapshot());
}
```

Change to:

```rust
let (changed, finished) = prune_state.live_sessions.prune(chrono::Utc::now());
if changed {
    use tauri::Emitter;
    let _ = prune_handle.emit("live_sessions_changed", prune_state.live_sessions.live_snapshot());
}
if !finished.is_empty() && prune_state.settings.read().notify_session_finished {
    use tauri_plugin_notification::NotificationExt;
    for f in finished {
        let duration = crate::notifier::rules::humanize_duration(chrono::Duration::seconds(f.live_span_secs));
        let body = if f.total_cost_usd > 0.0 {
            format!("{} — ${:.2}, {}", f.project, f.total_cost_usd, duration)
        } else {
            format!("{} — {}", f.project, duration)
        };
        let _ = prune_handle
            .notification()
            .builder()
            .title("Claude Code session finished")
            .body(body)
            .show();
    }
}
```

(Grep the current exact block before editing — F1's fix wave may have named the captured variables slightly differently than this sketch; the STRUCTURE is what matters, not the exact variable names.)

- [ ] **Step 4: Run the backend suite**

Run: `cd src-tauri && cargo test`
Expected: all pass (Task 1's 5 tests, Task 2's 1 new test, every pre-existing test). Run `cargo build` and `cargo clippy --all-targets -- -D warnings` to confirm the whole binary compiles cleanly — this task is what makes Task 1's signature change fully build.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/app_state.rs src-tauri/src/notifier/rules.rs src-tauri/src/lib.rs
git commit -m "feat(live-sessions): fire a native notification when a long session finishes"
```

---

### Task 3: Settings toggle (frontend)

**Files:**
- Modify: `src/lib/generated/bindings.ts` (`Settings` type — add `notify_session_finished: boolean; ` after `terminal`)
- Modify: `src/settings/SettingsPanel.tsx` (Notifications card — after the pay-as-you-go threshold slider and its caption, ~line 244-254)
- Modify: `src/settings/__tests__/SettingsPanel.test.tsx` (the file Task 4 of the PAYG feature created — append a case, following its established mock pattern)

**Interfaces:**
- Consumes: `Settings.notify_session_finished` from Task 2 (via the bindings edit here).
- Produces: user-editable toggle persisted through the existing `update('notify_session_finished', v)` → `save()` flow. Nothing later depends on this.

- [ ] **Step 1: Write the failing test**

In `src/settings/__tests__/SettingsPanel.test.tsx`, add (reuse the file's existing mock `settings` fixture — it must include `notify_session_finished: true` or the component may hit a different render branch; check the fixture's exact current shape first, per the note the PAYG feature's Task 4 review left about every `Settings` field needing to be present):

```tsx
  it('renders and toggles the session-finished notification setting', () => {
    render(<SettingsPanel />);
    const toggle = screen.getByLabelText(/session finished/i) as HTMLInputElement;
    expect(toggle.checked).toBe(true);
    fireEvent.click(toggle);
    expect(toggle.checked).toBe(false);
  });
```

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm exec vitest run src/settings/__tests__/SettingsPanel.test.tsx`
Expected: FAIL — no element with accessible label matching `/session finished/i`.

- [ ] **Step 3: Implement**

3a. Bindings: add `notify_session_finished: boolean; ` to the `Settings` type, positioned after `terminal` to match the Rust struct's field order (this file is otherwise alphabetically sorted BY TYPE NAME, but fields WITHIN one type follow the Rust struct's declared order, per the PAYG feature's established convention).

3b. `SettingsPanel.tsx`, inside the Notifications `<Card>`, after the pay-as-you-go threshold slider's caption paragraph and before the existing "Notifications fire once per bucket reset cycle" footer line:

```tsx
          <Toggle
            label="Session finished"
            description="Notify when a session that ran 10+ minutes goes quiet."
            checked={local.notify_session_finished}
            onChange={(e) => update('notify_session_finished', e.target.checked)}
          />
```

(`Toggle` is already imported in this file for the "Launch at login" toggle in the General section — no new import needed.)

- [ ] **Step 4: Run the full frontend gate**

Run: `pnpm exec vitest run src/settings/__tests__/SettingsPanel.test.tsx` → PASS. Then `pnpm lint` → clean. Then `pnpm test` → no new failures beyond the known pre-existing `theme.test.ts` collection error.

- [ ] **Step 5: Commit**

```bash
git add src/lib/generated/bindings.ts src/settings/SettingsPanel.tsx src/settings/__tests__/SettingsPanel.test.tsx
git commit -m "feat(settings): add the session-finished notification toggle"
```

---

## Verification checklist (after all tasks)

- `cargo test` (from `src-tauri/`) fully green, `cargo clippy --all-targets -- -D warnings` clean; `pnpm lint` clean; `pnpm test` no new failures.
- Manual: with a real Claude Code session running ≥10 minutes then left idle ≥5 more minutes (or by temporarily lowering the constants for a local smoke test), a native notification titled "Claude Code session finished" appears with project/cost/duration; flipping the new Settings toggle off suppresses it for the NEXT departure (not retroactively); a session under 10 minutes never notifies.
- No DB schema change; no new IPC command.
