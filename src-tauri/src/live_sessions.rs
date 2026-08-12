use crate::store::Db;
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const LIVE_QUIET_SECS: i64 = 120;
const COOLING_QUIET_SECS: i64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionState {
    Live,
    Cooling,
}

#[derive(Debug, Clone)]
struct LiveEntry {
    session_id: String,
    source_file: String,
    project: String,
    model: String,
    total_tokens: u64,
    total_cost_usd: f64,
    context_tokens: u64,
    first_seen: DateTime<Utc>,
    last_activity: DateTime<Utc>,
    state: SessionState,
    /// Hysteresis for the context-window warning: true = eligible to fire
    /// the next time `CONTEXT_WARN_PCT` is crossed; false = already fired
    /// for this climb, won't refire until pct drops below
    /// `CONTEXT_REARM_PCT`. New entries start `true` — a resumed session
    /// already above the threshold fires on its first touch.
    context_warning_armed: bool,
}

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

/// Anthropic models confirmed to be 200K — mirrors KNOWN_200K_PREFIXES in
/// src/sessions/contextWindow.ts. Keep both lists in sync.
const KNOWN_200K_PREFIXES: &[&str] = &[
    "claude-3-5-",
    "claude-3-7-",
    "claude-sonnet-4-0",
    "claude-sonnet-4-5",
    "claude-sonnet-4-6",
    "claude-opus-4-0",
    "claude-opus-4-1",
    "claude-opus-4-5",
    "claude-opus-4-6",
    "claude-haiku-4-5",
];

/// Resolves a model id to its context window in tokens. Mirrors
/// `contextWindow.ts::windowFor`'s ACTUAL behavior (only a trailing `[1m]`
/// suffix is stripped — that TS function does no provider-prefix
/// stripping, despite what an earlier draft of this feature's spec
/// claimed).
///
/// Returns `None` when the window can't be confidently resolved, exactly
/// like the frontend's `windowFor()` — this used to default unknown models
/// to 200K on the theory that a notification should "warn early rather
/// than never", but that guess is factually wrong for real, first-class
/// provider presets already in this codebase (see
/// `providers/presets.rs`'s `CLAUDE_CODE_MAX_CONTEXT_TOKENS`): GLM's real
/// window is 1,000,000 (guessing 200K fires a false alarm at just 16% of
/// the real window), and DeepSeek's real window is 131,072 (guessing
/// 200,000 means the 80%-of-guess trigger point is never reached, so the
/// warning silently never fires at all). The real window depends on which
/// provider preset is active — information this function, which only sees
/// a model name string from the transcript, doesn't have access to. No
/// notification is strictly better than a wrong one, so an unresolvable
/// window now skips the warning entirely instead of guessing.
///
/// The `[1m]` suffix is its OWN independent trigger for 1M — not merely
/// stripped before a `NATIVE_1M_MODELS` lookup. This mirrors the TS
/// reference's `NATIVE_1M_MODELS.has(bare) || /\[1m\]$/i.test(model)`: a
/// model id carrying the suffix (e.g. from a provider config) resolves to
/// 1M even if its bare name isn't itself in the known-1M list.
fn context_window_for(model: &str) -> Option<u64> {
    let lower = model.to_ascii_lowercase();
    match lower.strip_suffix("[1m]") {
        // A `[1m]` suffix means 1M unconditionally, regardless of whether
        // the stripped bare name is itself a recognized model.
        Some(_) => Some(1_000_000),
        None if NATIVE_1M_MODELS.contains(&lower.as_str()) => Some(1_000_000),
        None if KNOWN_200K_PREFIXES.iter().any(|p| lower.starts_with(p)) => Some(200_000),
        None => None,
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

/// One row of `get_live_sessions`. Deliberately structured so future
/// features (session-finished notifications, context-window warnings) can
/// add fields without reshaping this one — see the plan's Global
/// Constraints.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LiveSessionInfo {
    pub session_id: String,
    pub source_file: String,
    pub project: String,
    pub model: String,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    pub context_tokens: u64,
    /// Unix seconds — when the registry first saw this session THIS APP
    /// RUN (not the transcript's real age; the registry is in-memory only
    /// and starts empty on every launch, by design).
    pub first_seen: i64,
    /// Unix seconds of the most recent ingest touch.
    pub last_activity: i64,
}

/// Tracks which Claude Code sessions are actively writing to their JSONL
/// transcript right now. Fed exclusively by the watcher's per-file touch
/// loop (never by the startup backfill — see `note_ingest`'s caller in
/// `lib.rs`), so only post-launch activity ever registers.
#[derive(Default)]
pub struct LiveSessionRegistry {
    sessions: RwLock<HashMap<String, LiveEntry>>,
    /// session_ids already reported as finished this app run — see `prune`.
    notified: RwLock<std::collections::HashSet<String>>,
}

/// Folds a subagent transcript path onto its parent's key, and derives the
/// bare session id — same `/subagents/` convention as `Db::session_totals`,
/// but derived via `Path::components()` rather than a forward-slash string
/// search, so it works on Windows where paths use backslashes (mirrors the
/// idiom already used by `sessions::scan::is_subagent_path`). `touched_file`
/// and `projects_root` are both absolute paths. The returned `parent_key`
/// still renders with `to_string_lossy()` — same as `walker.rs::ingest_file`'s
/// derivation of `source_file` (`strip_prefix(projects_root)` then
/// `to_string_lossy()`, no separator normalization) — so it reproduces
/// whatever separator convention is actually stored in the DB on the current
/// OS, without ever hardcoding `/`.
fn registry_key(touched_file: &Path, projects_root: &Path) -> Option<(String, String)> {
    let rel = touched_file.strip_prefix(projects_root).ok()?;
    let components: Vec<_> = rel.components().collect();
    let subagents_idx = components
        .iter()
        .position(|c| c.as_os_str() == "subagents");
    let (parent_key, session_id) = match subagents_idx {
        Some(i) => {
            // components[i-1] is the session-id directory the subagent
            // lives under; components[..i] is the path down to (not
            // including) "subagents" — rebuild the parent's own ".jsonl"
            // key from it.
            let session_component = components.get(i.checked_sub(1)?)?;
            let session_id = session_component.as_os_str().to_string_lossy().into_owned();
            let parent_rel: std::path::PathBuf = components[..i].iter().collect();
            let parent_key = format!("{}.jsonl", parent_rel.to_string_lossy());
            (parent_key, session_id)
        }
        None => {
            let parent_key = rel.to_string_lossy().into_owned();
            let session_id = rel
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| parent_key.clone());
            (parent_key, session_id)
        }
    };
    Some((parent_key, session_id))
}

impl LiveSessionRegistry {
    /// Called once per watcher-touched file (parent or subagent). Refreshes
    /// the PARENT session's entry — subagent touches update the parent's
    /// totals but the parent's own `latest_event_for_file` (project/model/
    /// context) is untouched by a subagent write, since that query is
    /// scoped to the parent file only.
    pub fn note_ingest(
        &self,
        db: &Db,
        touched_file: &Path,
        projects_root: &Path,
        now: DateTime<Utc>,
    ) -> Option<ContextWarning> {
        let (parent_key, session_id) = registry_key(touched_file, projects_root)?;
        let Ok((total_tokens, total_cost_usd)) = db.live_session_totals(&parent_key) else {
            return None;
        };
        // The parent file's own latest event may not exist yet on the very
        // first subagent touch (main file written after its first
        // subagent in rare orderings) — fall back to sensible empties
        // rather than dropping the touch entirely.
        let latest = db.latest_event_for_file(&parent_key).ok().flatten();
        let project = latest.as_ref().map(|l| l.project.clone()).unwrap_or_default();
        let model = latest.as_ref().map(|l| l.model.clone()).unwrap_or_default();
        let context_tokens = latest.map(|l| l.context_tokens).unwrap_or(0);

        let mut sessions = self.sessions.write();
        let is_new_live_period = !sessions.contains_key(&parent_key);
        let first_seen = sessions
            .get(&parent_key)
            .map(|e| e.first_seen)
            .unwrap_or(now);
        let was_armed = sessions
            .get(&parent_key)
            .map(|e| e.context_warning_armed)
            .unwrap_or(true);
        let context_window = context_window_for(&model);
        let (warn, now_armed, pct) = match context_window {
            // Window can't be confidently resolved (e.g. a third-party
            // provider model like GLM or DeepSeek) — skip the warning
            // entirely rather than guess, and leave the armed/disarmed
            // hysteresis state untouched since we have no basis to change
            // it.
            None => (false, was_armed, 0u32),
            Some(window) => {
                let pct = if context_tokens > 0 {
                    ((context_tokens as f64 / window as f64) * 100.0) as u32
                } else {
                    0
                };
                if was_armed && pct >= CONTEXT_WARN_PCT {
                    (true, false, pct)
                } else if pct < CONTEXT_REARM_PCT {
                    (false, true, pct)
                } else {
                    (false, was_armed, pct)
                }
            }
        };
        if is_new_live_period {
            // A genuinely new live period is starting for this session_id
            // (no existing entry — as opposed to a write during Cooling,
            // which reuses the entry and its first_seen). Clear any stale
            // `notified` entry left behind by a PRIOR departure of the same
            // session_id, so this run gets its own fair shot at a
            // "finished" notification later — see prune()'s dedup set.
            //
            // Lock ordering: this acquires `notified` while still holding
            // `sessions` (write). That's safe only because every other
            // acquisition of both locks — see `prune()` — acquires
            // `sessions` first and always drops it before touching
            // `notified`, never the reverse; keep that invariant if either
            // function's locking changes.
            self.notified.write().remove(&session_id);
        }
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

        if warn {
            Some(ContextWarning { project, pct: pct.min(100) as u8 })
        } else {
            None
        }
    }

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
            .filter(|(id, f)| {
                f.live_span_secs >= MIN_NOTIFY_SPAN_SECS && notified.insert(id.clone())
            })
            .map(|(_, f)| f)
            .collect();

        (before_live != after_live, finished)
    }

    /// Live entries only, sorted by most-recently-active first.
    pub fn live_snapshot(&self) -> Vec<LiveSessionInfo> {
        let sessions = self.sessions.read();
        let mut out: Vec<LiveSessionInfo> = sessions
            .values()
            .filter(|e| e.state == SessionState::Live)
            .map(|e| LiveSessionInfo {
                session_id: e.session_id.clone(),
                source_file: e.source_file.clone(),
                project: e.project.clone(),
                model: e.model.clone(),
                total_tokens: e.total_tokens,
                total_cost_usd: e.total_cost_usd,
                context_tokens: e.context_tokens,
                first_seen: e.first_seen.timestamp(),
                last_activity: e.last_activity.timestamp(),
            })
            .collect();
        // Tiebreak on session_id: HashMap iteration order isn't stable, so
        // without this, entries sharing the same last_activity second could
        // visually swap rows between renders.
        out.sort_by(|a, b| {
            b.last_activity
                .cmp(&a.last_activity)
                .then(a.session_id.cmp(&b.session_id))
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::queries::StoredSessionEvent;
    use crate::store::Db;
    use chrono::{Duration, Utc};
    use std::path::PathBuf;

    fn fresh() -> (tempfile::TempDir, Db) {
        let d = tempfile::tempdir().unwrap();
        let db = Db::open(d.path()).unwrap();
        (d, db)
    }

    fn seed(db: &Db, source_file: &str, tokens: u64, model: &str) {
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
            source_line: 0,
            event_id: format!("{source_file}:seed:{tokens}"),
        };
        db.ingest_atomic(source_file, &[ev], &[], 1, 100).unwrap();
    }

    // projects_root/touched_file relationship: note_ingest computes the
    // registry key as the path relative to projects_root, matching how the
    // watcher and walker already derive `source_file`.
    fn root_and_file() -> (PathBuf, PathBuf, &'static str) {
        let root = PathBuf::from("/home/me/.claude/projects");
        let rel = "-proj/sess1.jsonl";
        (root.clone(), root.join(rel), rel)
    }

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
            assert_eq!(context_window_for(m), Some(1_000_000), "{m} should resolve to 1M");
        }
    }

    #[test]
    fn context_window_for_known_200k_prefix_resolves() {
        assert_eq!(context_window_for("claude-sonnet-4-6"), Some(200_000), "known-but-not-1M Anthropic model");
    }

    #[test]
    fn context_window_for_unresolvable_models_return_none() {
        assert_eq!(context_window_for("glm-4.6"), None, "third-party model");
        assert_eq!(context_window_for("some-future-model-id"), None, "unrecognized model");
    }

    #[test]
    fn context_window_for_strips_1m_suffix_case_insensitively() {
        assert_eq!(context_window_for("claude-sonnet-4-6[1m]"), Some(1_000_000));
        assert_eq!(context_window_for("claude-sonnet-4-6[1M]"), Some(1_000_000));
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

    #[test]
    fn unresolvable_model_never_fires_a_context_warning_even_at_huge_token_counts() {
        // Proves the GLM false-alarm bug is fixed: glm-5.2's real window
        // (per the "glm" preset's CLAUDE_CODE_MAX_CONTEXT_TOKENS) is
        // 1,000,000, but context_window_for can't know that from the model
        // name alone. Before the fix this guessed 200K and fired a warning
        // at just 16% of the real window; now it must resolve to None and
        // never warn, no matter how large context_tokens gets.
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        let reg = LiveSessionRegistry::default();
        seed_at_line(&db, key, 5_000_000, "glm-5.2", 0); // absurdly large, would be "500%" of a 1M guess
        let w = reg.note_ingest(&db, &file, &root, Utc::now());
        assert!(
            w.is_none(),
            "an unresolvable model must never produce a ContextWarning, regardless of token count"
        );
    }

    #[test]
    fn fresh_touch_becomes_live() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let now = Utc::now();
        reg.note_ingest(&db, &file, &root, now);
        let snap = reg.live_snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].session_id, "sess1");
        assert_eq!(snap[0].model, "claude-opus-5");
        assert_eq!(snap[0].first_seen, now.timestamp());
        assert_eq!(snap[0].last_activity, now.timestamp());
    }

    #[test]
    fn subagent_touch_folds_onto_parent_key() {
        let (_d, db) = fresh();
        let root = PathBuf::from("/home/me/.claude/projects");
        let parent_key = "-proj/sess1.jsonl";
        let sub_key = "-proj/sess1/subagents/agent-a.jsonl";
        seed(&db, parent_key, 100, "claude-opus-5");
        seed(&db, sub_key, 50, "claude-haiku-4-5");
        let reg = LiveSessionRegistry::default();
        let now = Utc::now();
        reg.note_ingest(&db, &root.join(sub_key), &root, now);
        let snap = reg.live_snapshot();
        assert_eq!(
            snap.len(),
            1,
            "subagent touch must fold onto the parent, not create a second entry"
        );
        assert_eq!(snap[0].session_id, "sess1");
        assert_eq!(snap[0].total_tokens, (100 + 1) + (50 + 1)); // input+output per seeded event
    }

    #[test]
    fn quiet_120s_transitions_to_cooling_and_hides_from_snapshot() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &file, &root, t0);
        assert_eq!(reg.live_snapshot().len(), 1);
        let (changed, _) = reg.prune(t0 + Duration::seconds(121));
        assert!(changed, "Live -> Cooling must report a change");
        assert_eq!(reg.live_snapshot().len(), 0, "Cooling entries are not Live");
    }

    #[test]
    fn write_during_cooling_returns_to_live_preserving_first_seen() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &file, &root, t0);
        reg.prune(t0 + Duration::seconds(121)); // -> Cooling
        let t1 = t0 + Duration::seconds(150);
        reg.note_ingest(&db, &file, &root, t1); // write during Cooling
        let snap = reg.live_snapshot();
        assert_eq!(
            snap.len(),
            1,
            "a write during Cooling returns the entry to Live"
        );
        assert_eq!(
            snap[0].first_seen,
            t0.timestamp(),
            "first_seen must survive the Cooling round-trip"
        );
        assert_eq!(snap[0].last_activity, t1.timestamp());
    }

    #[test]
    fn quiet_300s_removes_the_entry() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &file, &root, t0);
        reg.prune(t0 + Duration::seconds(301));
        assert_eq!(reg.live_snapshot().len(), 0);
        // And it's gone entirely, not just hidden: a later write starts a
        // brand new entry with a fresh first_seen, proving removal not just
        // a permanently-hidden state.
        let t1 = t0 + Duration::seconds(1000);
        reg.note_ingest(&db, &file, &root, t1);
        assert_eq!(reg.live_snapshot()[0].first_seen, t1.timestamp());
    }

    #[test]
    fn snapshot_sorted_by_last_activity_descending() {
        let (_d, db) = fresh();
        let root = PathBuf::from("/home/me/.claude/projects");
        seed(&db, "-proj/a.jsonl", 10, "claude-opus-5");
        seed(&db, "-proj/b.jsonl", 10, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &root.join("-proj/a.jsonl"), &root, t0);
        reg.note_ingest(
            &db,
            &root.join("-proj/b.jsonl"),
            &root,
            t0 + Duration::seconds(5),
        );
        let snap = reg.live_snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].session_id, "b", "most recently touched sorts first");
        assert_eq!(snap[1].session_id, "a");
    }

    #[test]
    fn registry_key_matches_walkers_forward_slash_relative_path() {
        let root = PathBuf::from("/home/me/.claude/projects");
        let (key, session_id) = registry_key(&root.join("proj").join("sess1.jsonl"), &root)
            .expect("path under root must resolve");
        assert_eq!(key, "proj/sess1.jsonl");
        assert_eq!(session_id, "sess1");
    }

    #[test]
    fn registry_key_does_not_normalize_separators_like_walker_rs() {
        // walker.rs derives `source_file` with `to_string_lossy()` and no
        // separator normalization (see jsonl_parser/walker.rs), so on
        // Windows a real relative path would contain literal backslashes.
        // We can't fabricate a multi-component backslash path on Unix (the
        // OS treats `\` as a plain filename character, not a separator),
        // but we CAN push a single path component that itself contains a
        // literal backslash byte and confirm registry_key preserves it
        // verbatim instead of rewriting it to a forward slash. Before the
        // fix, `.replace('\\', "/")` corrupted this into "weird/name.jsonl";
        // after the fix it must survive untouched, proving the key derived
        // here can still equal what walker.rs actually stores.
        let root = PathBuf::from("/home/me/.claude/projects");
        let touched = root.join("weird\\name.jsonl");
        let (key, session_id) =
            registry_key(&touched, &root).expect("path under root must resolve");
        assert_eq!(
            key, "weird\\name.jsonl",
            "registry_key must not rewrite backslashes to forward slashes"
        );
        assert_eq!(session_id, "weird\\name");
    }

    #[test]
    fn registry_key_folds_subagent_onto_parent_via_components_not_string_search() {
        let root = PathBuf::from("/home/me/.claude/projects");
        let (key, session_id) = registry_key(
            &root.join("proj").join("sess1").join("subagents").join("agent-a.jsonl"),
            &root,
        ).expect("path under root must resolve");
        assert_eq!(key, "proj/sess1.jsonl");
        assert_eq!(session_id, "sess1");
    }

    #[test]
    fn ten_minute_session_departs_with_a_finished_entry() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &file, &root, t0);
        // 10 minutes of activity — exactly at the MIN_NOTIFY_SPAN_SECS
        // floor, which is inclusive ("span floor is exactly 600 seconds
        // (>=)"), so this must still produce a finished entry.
        reg.note_ingest(&db, &file, &root, t0 + Duration::seconds(600));
        let (_, finished) = reg.prune(t0 + Duration::seconds(600) + Duration::seconds(301));
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].live_span_secs, 600);
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
        assert!(
            finished.is_empty(),
            "under the 10-minute floor must not notify"
        );
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

    #[test]
    fn a_new_live_period_for_the_same_session_id_gets_its_own_notification() {
        let (_d, db) = fresh();
        let (root, file, key) = root_and_file();
        seed(&db, key, 100, "claude-opus-5");
        let reg = LiveSessionRegistry::default();
        let t0 = Utc::now();
        reg.note_ingest(&db, &file, &root, t0);
        reg.note_ingest(&db, &file, &root, t0 + Duration::seconds(600));
        let departed_at = t0 + Duration::seconds(600) + Duration::seconds(301);
        let (_, first) = reg.prune(departed_at);
        assert_eq!(first.len(), 1, "first departure must be reported");

        // A brand-new live period starts for the SAME session_id/registry
        // key (e.g. the user resumes the same Claude Code session after a
        // break) — this is a `None` hit in note_ingest's `sessions.get`
        // lookup, i.e. a genuinely fresh first_seen, not a Cooling
        // round-trip. It must get its own fair shot at a notification, not
        // be silently swallowed by the `notified` set entry left behind by
        // the prior departure.
        let t1 = departed_at + Duration::seconds(1000);
        reg.note_ingest(&db, &file, &root, t1);
        reg.note_ingest(&db, &file, &root, t1 + Duration::seconds(600));
        let (_, second) = reg.prune(t1 + Duration::seconds(600) + Duration::seconds(301));
        assert_eq!(
            second.len(),
            1,
            "a new live period for the same session_id must be reported again, not \
             suppressed by a stale notified-set entry from the prior departure"
        );
    }
}
