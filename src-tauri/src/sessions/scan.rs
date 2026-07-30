use std::path::{Path, PathBuf};

/// Subagent transcripts live at `<project>/<sessionId>/subagents/agent-*.jsonl`.
/// They pass every content-based inclusion test, so only the path distinguishes
/// them — and resuming one would run `claude --resume <agentId>` against an id
/// that is not a session.
pub fn is_subagent_path(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "subagents")
}

/// Top-level transcripts only, newest first. Deliberately a two-level
/// `read_dir` rather than a recursive walk: recursion would pull in the 138
/// subagent transcripts, which outnumber real sessions in some projects.
pub fn discover_session_files(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    let Ok(projects) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    for project in projects.flatten() {
        let ppath = project.path();
        let Ok(pmeta) = std::fs::symlink_metadata(&ppath) else {
            continue;
        };
        if pmeta.file_type().is_symlink() || !pmeta.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&ppath) else {
            continue;
        };
        for f in files.flatten() {
            let fpath = f.path();
            let Ok(fmeta) = std::fs::symlink_metadata(&fpath) else {
                continue;
            };
            if fmeta.file_type().is_symlink() || !fmeta.is_file() {
                continue;
            }
            if fpath.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let mtime = fmeta.modified().unwrap_or(std::time::UNIX_EPOCH);
            out.push((mtime, fpath));
        }
    }
    // Newest first, so a truncated scan keeps the sessions most likely wanted.
    out.sort_by_key(|(mtime, _)| std::cmp::Reverse(*mtime));
    out.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn corpus() -> tempfile::TempDir {
        let root = tempdir().unwrap();
        let project = root.path().join("-Users-me-proj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("sess-1.jsonl"), "{}\n").unwrap();
        std::fs::write(project.join("sess-2.jsonl"), "{}\n").unwrap();
        let sub = project.join("sess-1").join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        for i in 0..5 {
            std::fs::write(sub.join(format!("agent-{i}.jsonl")), "{}\n").unwrap();
        }
        root
    }

    #[test]
    fn subagent_transcripts_are_never_discovered() {
        let root = corpus();
        let found = discover_session_files(root.path());
        assert_eq!(found.len(), 2, "only the two top-level transcripts");
        assert!(
            !found.iter().any(|p| is_subagent_path(p)),
            "a subagent transcript reached the browser"
        );
    }

    #[test]
    fn is_subagent_path_matches_the_segment_anywhere() {
        assert!(is_subagent_path(Path::new(
            "/a/proj/sess/subagents/agent-x.jsonl"
        )));
        assert!(!is_subagent_path(Path::new("/a/proj/sess-1.jsonl")));
        // Not fooled by a similarly-named file.
        assert!(!is_subagent_path(Path::new("/a/proj/subagents-notes.jsonl")));
    }

    #[test]
    fn results_are_newest_first() {
        let root = tempdir().unwrap();
        let project = root.path().join("p");
        std::fs::create_dir_all(&project).unwrap();
        let old = project.join("old.jsonl");
        let new = project.join("new.jsonl");
        std::fs::write(&old, "{}\n").unwrap();
        std::fs::write(&new, "{}\n").unwrap();
        filetime::set_file_mtime(
            &old,
            filetime::FileTime::from_system_time(
                std::time::SystemTime::now() - std::time::Duration::from_secs(7200),
            ),
        )
        .unwrap();
        let found = discover_session_files(root.path());
        assert!(found[0].ends_with("new.jsonl"), "newest first");
    }

    #[test]
    fn missing_root_is_not_an_error() {
        assert!(discover_session_files(Path::new("/definitely/not/here")).is_empty());
    }
}
