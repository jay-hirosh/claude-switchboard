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
    ) {
        let Some((parent_key, session_id)) = registry_key(touched_file, projects_root) else {
            return;
        };
        let Ok((total_tokens, total_cost_usd)) = db.live_session_totals(&parent_key) else {
            return;
        };
        // The parent file's own latest event may not exist yet on the very
        // first subagent touch (main file written after its first
        // subagent in rare orderings) — fall back to sensible empties
        // rather than dropping the touch entirely.
        let latest = db.latest_event_for_file(&parent_key).ok().flatten();

        let mut sessions = self.sessions.write();
        let first_seen = sessions
            .get(&parent_key)
            .map(|e| e.first_seen)
            .unwrap_or(now);
        sessions.insert(
            parent_key.clone(),
            LiveEntry {
                session_id,
                source_file: parent_key,
                project: latest
                    .as_ref()
                    .map(|l| l.project.clone())
                    .unwrap_or_default(),
                model: latest.as_ref().map(|l| l.model.clone()).unwrap_or_default(),
                total_tokens,
                total_cost_usd,
                context_tokens: latest.map(|l| l.context_tokens).unwrap_or(0),
                first_seen,
                last_activity: now,
                state: SessionState::Live,
            },
        );
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
}
