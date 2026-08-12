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
}

/// Folds a subagent transcript path onto its parent's key, and derives the
/// bare session id (file stem) — same `/subagents/` convention as
/// `Db::session_totals`. `touched_file` and `projects_root` are both
/// absolute paths; the returned key is projects-root-relative, matching
/// `source_file` as stored in `session_events`.
fn registry_key(touched_file: &Path, projects_root: &Path) -> Option<(String, String)> {
    let rel = touched_file.strip_prefix(projects_root).ok()?;
    let rel_str = rel.to_str()?.replace('\\', "/"); // Windows path separators
    let parent_key = match rel_str.find("/subagents/") {
        Some(i) => format!("{}.jsonl", &rel_str[..i]),
        None => rel_str,
    };
    let session_id = parent_key
        .rsplit('/')
        .next()
        .unwrap_or(&parent_key)
        .strip_suffix(".jsonl")
        .unwrap_or(&parent_key)
        .to_string();
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
    pub fn prune(&self, now: DateTime<Utc>) {
        let mut sessions = self.sessions.write();
        sessions.retain(|_, e| {
            let quiet = (now - e.last_activity).num_seconds();
            if quiet >= COOLING_QUIET_SECS {
                false
            } else {
                if quiet >= LIVE_QUIET_SECS {
                    e.state = SessionState::Cooling;
                }
                true
            }
        });
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
        out.sort_by_key(|e| std::cmp::Reverse(e.last_activity));
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
        reg.prune(t0 + Duration::seconds(121));
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
}
