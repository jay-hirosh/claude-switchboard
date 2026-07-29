use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

const MAX_TOUCHED: usize = 4;
const TITLE_MAX_CHARS: usize = 80;

/// Markers Claude Code injects into `type: "user"` records that are not the
/// human speaking. Matched exactly rather than by a bare `<` prefix, which
/// would misfire on pasted markup or generics such as `Vec<T>`.
///
/// The full set was taken by scanning the local corpus for user-role text
/// starting with `<`; every distinct opening tag found is listed here. The
/// four that were originally missing accounted for 102 of 202 injected
/// records, and each one surfaced verbatim as a session title — a row reading
/// "<local-command-caveat>Caveat: The messages below…" is the visible symptom.
/// Each of these is a standalone record in the corpus (no real prompt is
/// appended after the closing tag), so refusing the whole record is correct
/// and loses nothing.
const INJECTION_MARKERS: [&str; 8] = [
    "<system-reminder>",
    "<command-name>",
    "<local-command-stdout>",
    "<command-message>",
    "<local-command-caveat>",
    "<task-notification>",
    "<bash-input>",
    "<bash-stdout>",
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
    /// Largest context the session ever held, in tokens: the peak of
    /// `input + cache_read + cache_creation` across assistant turns.
    ///
    /// Claude Code strips the `[1m]` suffix before writing a transcript
    /// (`claude-opus-5[1m]` is recorded as `claude-opus-5`), so the
    /// *configured* window is not recoverable from the file. The peak is,
    /// and it is the more useful number anyway — it says how full the
    /// conversation actually got. A peak above the 200K standard window is
    /// also positive proof the session ran on a 1M one.
    ///
    /// `None` for a transcript with no usage-bearing assistant turn.
    pub peak_context_tokens: Option<u64>,
    pub turns: u32,
    pub started_at: String,
    pub ended_at: String,
    /// Lifetime input + output tokens for this session, subagents included.
    ///
    /// Deliberately not computed here: `parse_session` reads one transcript
    /// in isolation, so it can neither dedupe an API call written to several
    /// lines nor price it. Both already happen during ingestion, so
    /// `list_resumable_sessions` fills these in from `session_events` — which
    /// also guarantees a session's number here equals what the Cost tab shows
    /// for the same conversation.
    pub total_tokens: u64,
    /// Lifetime cost in USD, subagents included. Zero when the session has no
    /// ingested usage (a transcript with no assistant turns, or one that
    /// predates the store).
    pub total_cost_usd: f64,
    /// Whether `cwd` still exists. Resuming is a `cd` into that directory, so
    /// a session whose project folder has been deleted cannot be resumed by
    /// any route — Claude Code offers no way to name a transcript directly.
    /// Surfaced so the button can be disabled with a reason rather than
    /// opening a terminal that immediately fails.
    pub cwd_exists: bool,
}

/// Mirrors upstream `sanitizePath` (`sessionStoragePortable.ts:311`):
/// `name.replace(/[^a-zA-Z0-9]/g, '-')`.
///
/// The JS regex runs over UTF-16 code units, so one astral character becomes
/// *two* dashes; `len_utf16` reproduces that rather than approximating it.
pub fn sanitize_path(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else {
            for _ in 0..c.len_utf16() {
                out.push('-');
            }
        }
    }
    out
}

/// The directory a session must be resumed from.
///
/// Claude Code stores a transcript at
/// `~/.claude/projects/<sanitize_path(cwd)>/<session_id>.jsonl` and resolves
/// `--resume <id>` *only* against the directory derived from the cwd it is
/// launched in — there is no session-id→directory index and no parent walk
/// (`sessionStoragePortable.ts:329`, and the explicit note at
/// `sessionStorage.ts:216`: "we don't track a sessionId→projectDir map").
///
/// A session's cwd changes mid-run whenever the user cds elsewhere, which a
/// git worktree makes routine. Taking the *last* cwd therefore points at a
/// directory whose slug owns no transcript, and `claude --resume` exits with
/// "No conversation found with session ID" — the feature dead for exactly the
/// sessions most worth resuming.
///
/// Matching is forward-only: the slug is lossy (`\` and `:` and `.` all become
/// `-`), so it can never be decoded back into a path.
fn owning_cwd(cwds: &[String], path: &Path) -> Option<String> {
    let dir = path.parent()?.file_name()?.to_string_lossy().to_string();
    cwds.iter()
        .find(|c| sanitize_path(c) == dir)
        .cloned()
        // Paths past upstream's 200-char cap get a Bun-specific hash suffix we
        // cannot reproduce. The first cwd is the original one in every other
        // case, so it is a strictly better guess than the last.
        .or_else(|| cwds.first().cloned())
}

/// Total context held by one assistant turn — everything the model had to
/// read, whether fresh, cached, or being written to cache. Mirrors what the
/// status line reports as context usage.
fn context_of(usage: &Value) -> u64 {
    ["input_tokens", "cache_read_input_tokens", "cache_creation_input_tokens"]
        .iter()
        .filter_map(|k| usage.get(*k).and_then(Value::as_u64))
        .sum()
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

    // Every cwd the session held, in order and de-duplicated. Not a single
    // overwritten value: the last one is usually not the one that owns the
    // transcript — see `owning_cwd`.
    let mut cwds: Vec<String> = Vec::new();
    let mut git_branch: Option<String> = None;
    let mut ai_title: Option<String> = None;
    let mut away_summary: Option<String> = None;
    let mut model: Option<String> = None;
    let mut first_user: Option<String> = None;
    let mut last_user: Option<String> = None;
    let mut turns: u32 = 0;
    let mut peak_context: u64 = 0;
    let mut timestamps: Vec<String> = Vec::new();
    let mut touched: HashMap<String, usize> = HashMap::new();

    for line in text.lines() {
        // A malformed line is skipped, never fatal.
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        if let Some(c) = v.get("cwd").and_then(Value::as_str) {
            if !c.is_empty() && !cwds.iter().any(|e| e == c) {
                cwds.push(c.to_string());
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
            if let Some(usage) = message.get("usage") {
                peak_context = peak_context.max(context_of(usage));
            }
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

    let cwd = owning_cwd(&cwds, path)?;
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
        cwd_exists: Path::new(&cwd).is_dir(),
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
        peak_context_tokens: (peak_context > 0).then_some(peak_context),
        turns,
        started_at: timestamps.first().cloned().unwrap_or_default(),
        ended_at: timestamps.last().cloned().unwrap_or_default(),
        // Filled in by the caller from the event store — see the field docs.
        total_tokens: 0,
        total_cost_usd: 0.0,
    })
}

#[cfg(test)]
mod resume_directory_tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn at(cwd: &str, text: &str) -> Value {
        json!({"cwd": cwd, "timestamp": "2026-07-29T10:00:00Z",
               "message": {"role": "user", "content": [{"type": "text", "text": text}]}})
    }

    /// Writes a transcript into the project folder Claude Code would have
    /// created for `original_cwd` — i.e. the real on-disk layout.
    fn transcript_owned_by(root: &Path, original_cwd: &str, records: &[Value]) -> PathBuf {
        let project = root.join(sanitize_path(original_cwd));
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join("11111111-2222-3333-4444-555555555555.jsonl");
        let body: String = records
            .iter()
            .map(|r| format!("{r}\n"))
            .collect::<Vec<_>>()
            .concat();
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn slug_matches_upstream_sanitize_path() {
        // The exact strings this machine has on disk.
        assert_eq!(
            sanitize_path(r"C:\Users\xue\nextGenRepo"),
            "C--Users-xue-nextGenRepo"
        );
        assert_eq!(
            sanitize_path(r"C:\Users\xue\nextGenRepo\.worktrees\Eric_copilot_integration\ZooKeeperTesting"),
            "C--Users-xue-nextGenRepo--worktrees-Eric-copilot-integration-ZooKeeperTesting"
        );
        assert_eq!(sanitize_path("/Users/foo/my-project"), "-Users-foo-my-project");
    }

    /// The regression. A session that starts in the repo root and cds into a
    /// worktree must still be resumed from the root: `claude --resume` derives
    /// the project folder from the launch cwd, so launching in the worktree
    /// fails with "No conversation found with session ID".
    #[test]
    fn cwd_is_the_directory_that_owns_the_transcript_not_the_last_one() {
        let root = tempdir().unwrap();
        let repo = r"C:\Users\xue\nextGenRepo";
        let worktree = r"C:\Users\xue\nextGenRepo\.worktrees\Eric_copilot_integration\ZooKeeperTesting";

        let path = transcript_owned_by(
            root.path(),
            repo,
            &[at(repo, "start here"), at(worktree, "now in the worktree")],
        );

        let s = parse_session(&path).expect("session parses");
        assert_eq!(
            s.cwd, repo,
            "must launch from the transcript's owning directory, not the last cwd"
        );
        assert_eq!(
            sanitize_path(&s.cwd),
            path.parent().unwrap().file_name().unwrap().to_string_lossy(),
            "the chosen cwd must slug back to the folder holding the transcript"
        );
    }

    /// A session that never moves is unaffected.
    #[test]
    fn a_single_cwd_session_is_unchanged() {
        let root = tempdir().unwrap();
        let dir = "/w/proj";
        let path = transcript_owned_by(root.path(), dir, &[at(dir, "hello")]);
        assert_eq!(parse_session(&path).unwrap().cwd, dir);
    }

    /// When no recorded cwd slugs to the folder name (upstream truncates and
    /// hashes paths over 200 chars), the first cwd still beats the last.
    #[test]
    fn falls_back_to_the_first_cwd_when_no_slug_matches() {
        let root = tempdir().unwrap();
        let project = root.path().join("totally-unrelated-folder-name");
        std::fs::create_dir_all(&project).unwrap();
        let path = project.join("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl");
        std::fs::write(
            &path,
            format!("{}\n{}\n", at("/first", "a"), at("/second", "b")),
        )
        .unwrap();
        assert_eq!(parse_session(&path).unwrap().cwd, "/first");
    }

    /// Drives the disabled state of the Resume button.
    #[test]
    fn cwd_existence_is_reported() {
        let root = tempdir().unwrap();
        let real = root.path().join("live-project");
        std::fs::create_dir_all(&real).unwrap();
        let real_s = real.to_string_lossy().to_string();

        let present = transcript_owned_by(root.path(), &real_s, &[at(&real_s, "hi")]);
        assert!(parse_session(&present).unwrap().cwd_exists);

        let gone = "/definitely/not/here";
        let missing = transcript_owned_by(root.path(), gone, &[at(gone, "hi")]);
        assert!(
            !parse_session(&missing).unwrap().cwd_exists,
            "a deleted project folder must be reported so Resume can be disabled"
        );
    }
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

    fn assistant_usage(input: u64, cache_read: u64, cache_create: u64) -> Value {
        json!({"cwd":"/w/proj","timestamp":"2026-07-29T10:03:00Z",
               "message":{"role":"assistant","model":"claude-opus-5","content":[],
                          "usage":{"input_tokens":input,
                                   "cache_read_input_tokens":cache_read,
                                   "cache_creation_input_tokens":cache_create,
                                   "output_tokens":500}}})
    }

    /// Context is the sum of everything the model had to read — fresh input
    /// plus cache reads plus cache writes. Counting only `input_tokens` would
    /// report ~2K for a session actually holding 400K, because almost all of
    /// a long conversation arrives as a cache read.
    #[test]
    fn peak_context_sums_all_input_categories_and_takes_the_maximum() {
        let d = tempdir().unwrap();
        let p = write_session(
            d.path(),
            "s.jsonl",
            &[
                user("go"),
                assistant_usage(1_000, 20_000, 4_000), // 25_000
                assistant_usage(2_000, 400_000, 8_000), // 410_000 ← peak
                assistant_usage(1_500, 300_000, 2_000), // 303_500 (after a compact)
            ],
        );
        let s = parse_session(&p).unwrap();
        assert_eq!(
            s.peak_context_tokens,
            Some(410_000),
            "peak must be the max over turns, not the last or the sum"
        );
    }

    /// Output tokens are what the model wrote, not what it had to read. Adding
    /// them would inflate every long session's context by the length of the
    /// replies.
    #[test]
    fn peak_context_excludes_output_tokens() {
        let d = tempdir().unwrap();
        let p = write_session(
            d.path(),
            "s.jsonl",
            &[user("go"), assistant_usage(1_000, 0, 0)],
        );
        assert_eq!(parse_session(&p).unwrap().peak_context_tokens, Some(1_000));
    }

    #[test]
    fn peak_context_is_none_when_no_turn_reports_usage() {
        let d = tempdir().unwrap();
        let p = write_session(
            d.path(),
            "s.jsonl",
            &[user("go"), assistant_edit("/w/proj/a.rs")],
        );
        assert!(
            parse_session(&p).unwrap().peak_context_tokens.is_none(),
            "absent usage must be None, never a misleading 0"
        );
    }

    /// A partial usage object must not be fatal or silently zero the others.
    #[test]
    fn peak_context_tolerates_missing_usage_fields() {
        let d = tempdir().unwrap();
        let p = write_session(
            d.path(),
            "s.jsonl",
            &[
                user("go"),
                json!({"cwd":"/w/proj","message":{"role":"assistant","content":[],
                       "usage":{"cache_read_input_tokens":50_000}}}),
            ],
        );
        assert_eq!(parse_session(&p).unwrap().peak_context_tokens, Some(50_000));
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

    /// Every marker in the corpus, not just the four originally listed. A
    /// session whose only "user" record is one of these has no real turn and
    /// must drop out entirely rather than render its own scaffolding as a
    /// title.
    #[test]
    fn every_injected_wrapper_is_refused_and_leaves_no_session() {
        let d = tempdir().unwrap();
        for (i, marker) in [
            "<system-reminder>be nice</system-reminder>",
            "<command-name>/compact</command-name>",
            "<local-command-stdout>ok</local-command-stdout>",
            "<command-message>compact</command-message>",
            "<local-command-caveat>Caveat: The messages below were generated by the user \
             while running local commands.</local-command-caveat>",
            "<task-notification>Agent bz13h5hgv finished</task-notification>",
            "<bash-input>ls -la</bash-input>",
            "<bash-stdout>total 0</bash-stdout>",
        ]
        .iter()
        .enumerate()
        {
            let p = write_session(d.path(), &format!("only-{i}.jsonl"), &[user(marker)]);
            assert!(
                parse_session(&p).is_none(),
                "a transcript containing only {marker} is not a session"
            );

            let p = write_session(
                d.path(),
                &format!("mixed-{i}.jsonl"),
                &[user(marker), user("the real question")],
            );
            let s = parse_session(&p).expect("session");
            assert_eq!(s.asked, "the real question");
            assert_eq!(s.title, "the real question", "{marker} must not title a row");
            assert_eq!(s.turns, 1, "{marker} is not a turn");
        }
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
