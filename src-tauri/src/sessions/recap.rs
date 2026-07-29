use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

const MAX_TOUCHED: usize = 4;
const TITLE_MAX_CHARS: usize = 80;

/// Markers Claude Code injects into `type: "user"` records that are not the
/// human speaking. Matched exactly rather than by a bare `<` prefix, which
/// would misfire on pasted markup or generics such as `Vec<T>`.
const INJECTION_MARKERS: [&str; 4] = [
    "<system-reminder>",
    "<command-name>",
    "<local-command-stdout>",
    "<command-message>",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct SessionSummary {
    pub session_id: String,
    pub cwd: String,
    pub project_name: String,
    pub git_branch: Option<String>,
    pub title: String,
    /// Claude Code's own end-of-session summary (the `※ recap:` line). The
    /// single most identifying signal in a transcript — it states goal,
    /// state and next action — but present in only 50% of sessions.
    pub recap: Option<String>,
    pub asked: String,
    pub left_off: Option<String>,
    pub touched_files: Vec<String>,
    pub touched_overflow: usize,
    pub model: Option<String>,
    pub turns: u32,
    pub started_at: String,
    pub ended_at: String,
}

/// The text of a *real* user message, or `None`.
///
/// A `type: "user"` record may carry only `tool_result` blocks — 58% of
/// listed sessions end on one. Those are not the user speaking and must never
/// surface as `asked` or `left_off`.
pub fn is_real_user_text(message: &Value) -> Option<String> {
    if message.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let text = match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => return None,
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if INJECTION_MARKERS.iter().any(|m| trimmed.starts_with(m)) {
        return None;
    }
    Some(trimmed.to_string())
}

/// Removes the trailing interface hint Claude Code appends to every recap.
/// Present on all 35 recaps in the corpus; it is chrome, not content.
fn strip_recap_chrome(s: &str) -> String {
    s.trim()
        .trim_end_matches("(disable recaps in /config)")
        .trim()
        .to_string()
}

fn truncate(s: &str, max: usize) -> String {
    let one_line = s.split('\n').next().unwrap_or(s).trim();
    if one_line.chars().count() <= max {
        return one_line.to_string();
    }
    let cut: String = one_line.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

/// `None` when the transcript does not qualify as a session (spec §3).
pub fn parse_session(path: &Path) -> Option<SessionSummary> {
    if crate::sessions::scan::is_subagent_path(path) {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;

    let mut cwd: Option<String> = None;
    let mut git_branch: Option<String> = None;
    let mut ai_title: Option<String> = None;
    let mut away_summary: Option<String> = None;
    let mut model: Option<String> = None;
    let mut first_user: Option<String> = None;
    let mut last_user: Option<String> = None;
    let mut turns: u32 = 0;
    let mut timestamps: Vec<String> = Vec::new();
    let mut touched: HashMap<String, usize> = HashMap::new();

    for line in text.lines() {
        // A malformed line is skipped, never fatal.
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        if let Some(c) = v.get("cwd").and_then(Value::as_str) {
            if !c.is_empty() {
                cwd = Some(c.to_string());
            }
        }
        if let Some(b) = v.get("gitBranch").and_then(Value::as_str) {
            if !b.is_empty() {
                git_branch = Some(b.to_string());
            }
        }
        if let Some(t) = v.get("aiTitle").and_then(Value::as_str) {
            if !t.is_empty() {
                ai_title = Some(t.to_string());
            }
        }
        if let Some(ts) = v.get("timestamp").and_then(Value::as_str) {
            timestamps.push(ts.to_string());
        }

        // Claude Code rewrites the recap as the session moves — 20 of 70
        // sessions carry more than one record, one carries 16. Overwriting
        // keeps the last, which is the current one.
        if v.get("type").and_then(Value::as_str) == Some("system")
            && v.get("subtype").and_then(Value::as_str) == Some("away_summary")
        {
            if let Some(c) = v.get("content").and_then(Value::as_str) {
                if !c.trim().is_empty() {
                    away_summary = Some(c.to_string());
                }
            }
        }

        let Some(message) = v.get("message") else {
            continue;
        };

        // `<synthetic>` is Claude Code's placeholder for locally generated
        // messages, not a model the session ran on.
        if let Some(m) = message.get("model").and_then(Value::as_str) {
            if !m.is_empty() && m != "<synthetic>" {
                model = Some(m.to_string());
            }
        }

        if let Some(t) = is_real_user_text(message) {
            turns += 1;
            if first_user.is_none() {
                first_user = Some(t.clone());
            }
            last_user = Some(t);
        }

        if message.get("role").and_then(Value::as_str) == Some("assistant") {
            if let Some(Value::Array(blocks)) = message.get("content") {
                for b in blocks {
                    if b.get("type").and_then(Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    let Some(fp) = b.pointer("/input/file_path").and_then(Value::as_str) else {
                        continue;
                    };
                    let name = Path::new(fp)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| fp.to_string());
                    *touched.entry(name).or_insert(0) += 1;
                }
            }
        }
    }

    let cwd = cwd?;
    let project_name = Path::new(&cwd).file_name()?.to_string_lossy().to_string();
    if project_name == "-" || turns == 0 {
        return None;
    }
    let asked = first_user?;

    let mut ranked: Vec<(String, usize)> = touched.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let touched_overflow = ranked.len().saturating_sub(MAX_TOUCHED);
    let touched_files: Vec<String> = ranked.into_iter().take(MAX_TOUCHED).map(|(n, _)| n).collect();

    let left_off = last_user.filter(|l| l != &asked).map(|l| truncate(&l, 160));

    Some(SessionSummary {
        session_id: path.file_stem()?.to_string_lossy().to_string(),
        cwd,
        project_name,
        git_branch,
        title: ai_title.unwrap_or_else(|| truncate(&asked, TITLE_MAX_CHARS)),
        recap: away_summary.as_deref().map(strip_recap_chrome),
        asked: truncate(&asked, 160),
        left_off,
        touched_files,
        touched_overflow,
        model,
        turns,
        started_at: timestamps.first().cloned().unwrap_or_default(),
        ended_at: timestamps.last().cloned().unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn write_session(dir: &Path, name: &str, lines: &[Value]) -> std::path::PathBuf {
        let p = dir.join(name);
        let body: String = lines
            .iter()
            .map(|l| serde_json::to_string(l).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&p, body).unwrap();
        p
    }

    fn user(text: &str) -> Value {
        json!({"cwd":"/w/proj","gitBranch":"main","timestamp":"2026-07-29T10:00:00Z",
               "message":{"role":"user","content":[{"type":"text","text":text}]}})
    }

    fn tool_result(text: &str) -> Value {
        json!({"cwd":"/w/proj","timestamp":"2026-07-29T10:05:00Z",
               "message":{"role":"user","content":[
                   {"type":"tool_result","tool_use_id":"toolu_1","content":text}]}})
    }

    fn assistant_edit(file: &str) -> Value {
        json!({"cwd":"/w/proj","timestamp":"2026-07-29T10:02:00Z",
               "message":{"role":"assistant","model":"claude-opus-5","content":[
                   {"type":"tool_use","name":"Edit","input":{"file_path":file}}]}})
    }

    /// H1 regression: 58% of real sessions end on a tool_result.
    #[test]
    fn tool_results_never_become_asked_or_left_off() {
        let d = tempdir().unwrap();
        let p = write_session(
            d.path(),
            "s.jsonl",
            &[
                tool_result("This command requires approval"),
                user("build the thing"),
                assistant_edit("/w/proj/src/main.rs"),
                user("now ship it"),
                tool_result("Applications\nLibrary\nSystem"),
            ],
        );
        let s = parse_session(&p).expect("session");
        assert_eq!(
            s.asked, "build the thing",
            "a leading tool_result must not become asked"
        );
        assert_eq!(
            s.left_off.as_deref(),
            Some("now ship it"),
            "a trailing tool_result must not become left_off"
        );
        assert_eq!(s.turns, 2, "tool results are not turns");
    }

    #[test]
    fn injection_markers_are_not_user_messages() {
        let d = tempdir().unwrap();
        let p = write_session(
            d.path(),
            "s.jsonl",
            &[
                user("<system-reminder>be nice</system-reminder>"),
                user("real question"),
            ],
        );
        let s = parse_session(&p).expect("session");
        assert_eq!(s.asked, "real question");
        assert_eq!(s.turns, 1);
    }

    #[test]
    fn generic_angle_brackets_are_still_real_messages() {
        let d = tempdir().unwrap();
        let p = write_session(d.path(), "s.jsonl", &[user("<T> in Vec<T> confuses me")]);
        let s = parse_session(&p).expect("session");
        assert_eq!(
            s.asked, "<T> in Vec<T> confuses me",
            "bare '<' must not be a filter"
        );
    }

    #[test]
    fn title_prefers_ai_title_then_falls_back_to_first_message() {
        let d = tempdir().unwrap();
        let p1 = write_session(
            d.path(),
            "a.jsonl",
            &[
                user("some long question about the thing"),
                json!({"aiTitle":"Fix the parser","cwd":"/w/proj"}),
            ],
        );
        assert_eq!(parse_session(&p1).unwrap().title, "Fix the parser");

        let p2 = write_session(
            d.path(),
            "b.jsonl",
            &[user("some long question about the thing")],
        );
        assert_eq!(
            parse_session(&p2).unwrap().title,
            "some long question about the thing"
        );
    }

    fn away(text: &str) -> Value {
        json!({"type":"system","subtype":"away_summary","cwd":"/w/proj","content":text})
    }

    #[test]
    fn recap_takes_the_last_away_summary_and_strips_chrome() {
        let d = tempdir().unwrap();
        let p = write_session(
            d.path(),
            "s.jsonl",
            &[
                user("go"),
                away("Goal: an early state. (disable recaps in /config)"),
                away("Goal: the current state. Next: ship it. (disable recaps in /config)"),
            ],
        );
        let s = parse_session(&p).unwrap();
        assert_eq!(
            s.recap.as_deref(),
            Some("Goal: the current state. Next: ship it."),
            "the last recap wins and the /config hint is stripped"
        );
    }

    #[test]
    fn recap_is_none_when_absent_rather_than_empty() {
        let d = tempdir().unwrap();
        let p = write_session(d.path(), "s.jsonl", &[user("go")]);
        assert!(parse_session(&p).unwrap().recap.is_none());
    }

    #[test]
    fn away_summary_is_not_mistaken_for_a_user_turn() {
        let d = tempdir().unwrap();
        let p = write_session(d.path(), "s.jsonl", &[user("go"), away("Goal: x.")]);
        let s = parse_session(&p).unwrap();
        assert_eq!(s.turns, 1, "a system record is not a turn");
        assert_eq!(s.asked, "go");
    }

    #[test]
    fn left_off_is_none_when_it_equals_asked() {
        let d = tempdir().unwrap();
        let p = write_session(d.path(), "s.jsonl", &[user("only one turn")]);
        assert!(parse_session(&p).unwrap().left_off.is_none());
    }

    #[test]
    fn touched_files_rank_by_frequency_and_cap_at_four() {
        let d = tempdir().unwrap();
        let mut lines = vec![user("go")];
        for _ in 0..3 {
            lines.push(assistant_edit("/w/proj/a.rs"));
        }
        for _ in 0..2 {
            lines.push(assistant_edit("/w/proj/b.rs"));
        }
        for f in ["c.rs", "d.rs", "e.rs"] {
            lines.push(assistant_edit(&format!("/w/proj/{f}")));
        }
        let p = write_session(d.path(), "s.jsonl", &lines);
        let s = parse_session(&p).unwrap();
        assert_eq!(s.touched_files, vec!["a.rs", "b.rs", "c.rs", "d.rs"]);
        assert_eq!(s.touched_overflow, 1);
    }

    #[test]
    fn touched_is_empty_rather_than_placeholder_when_absent() {
        let d = tempdir().unwrap();
        let p = write_session(d.path(), "s.jsonl", &[user("just talking")]);
        let s = parse_session(&p).unwrap();
        assert!(s.touched_files.is_empty());
        assert_eq!(s.touched_overflow, 0);
    }

    #[test]
    fn synthetic_model_is_not_treated_as_a_model() {
        let d = tempdir().unwrap();
        let p = write_session(
            d.path(),
            "s.jsonl",
            &[
                user("hi"),
                json!({"cwd":"/w/proj","message":{"role":"assistant","model":"<synthetic>","content":[]}}),
            ],
        );
        assert!(parse_session(&p).unwrap().model.is_none());
    }

    #[test]
    fn headless_and_turnless_transcripts_are_excluded() {
        let d = tempdir().unwrap();
        // No cwd at all.
        let p1 = write_session(
            d.path(),
            "a.jsonl",
            &[json!({"message":{"role":"user","content":[{"type":"text","text":"hi"}]}})],
        );
        assert!(parse_session(&p1).is_none());
        // cwd resolves to project "-".
        let p2 = write_session(
            d.path(),
            "b.jsonl",
            &[json!({"cwd":"/-","message":{"role":"user","content":[{"type":"text","text":"hi"}]}})],
        );
        assert!(parse_session(&p2).is_none());
        // Real cwd but no real user turn.
        let p3 = write_session(d.path(), "c.jsonl", &[tool_result("x")]);
        assert!(parse_session(&p3).is_none());
    }

    #[test]
    fn malformed_lines_are_skipped_not_fatal() {
        let d = tempdir().unwrap();
        let p = d.path().join("s.jsonl");
        std::fs::write(
            &p,
            format!(
                "not json at all\n{}\n{{ broken\n",
                serde_json::to_string(&user("hi")).unwrap()
            ),
        )
        .unwrap();
        let s = parse_session(&p).expect("one good line is enough");
        assert_eq!(s.asked, "hi");
    }

    #[test]
    fn subagent_paths_are_refused_even_when_content_qualifies() {
        let d = tempdir().unwrap();
        let sub = d.path().join("sess").join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        let p = write_session(&sub, "agent-x.jsonl", &[user("do the subtask")]);
        assert!(
            parse_session(&p).is_none(),
            "content qualifies; the path must still exclude it"
        );
    }
}
