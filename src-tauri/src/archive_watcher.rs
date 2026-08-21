use crate::sessions::{recap, scan};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Distinct, still-existing repo roots derived from every session
/// transcript's own `cwd` field — the same resolution the Repo tab already
/// relies on. Deliberately not derived by de-slugifying
/// `~/.claude/projects/<slug>` directory names: that's lossy wherever a real
/// path component contains a literal `-`, which `cwd` never is (it's read
/// straight from the JSONL, not reconstructed).
pub fn discover_project_roots(claude_projects_root: &Path) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    for f in scan::discover_session_files(claude_projects_root) {
        let Some(summary) = recap::parse_session(&f) else {
            continue;
        };
        if summary.cwd.is_empty() || !seen.insert(summary.cwd.clone()) {
            continue;
        }
        let path = PathBuf::from(&summary.cwd);
        if path.is_dir() {
            roots.push(path);
        }
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_session(project_dir: &Path, file_name: &str, cwd: &str) {
        std::fs::create_dir_all(project_dir).unwrap();
        let line = format!(
            r#"{{"type":"user","timestamp":"2026-01-01T00:00:00Z","cwd":"{cwd}","message":{{"role":"user","content":"hi"}}}}"#
        );
        std::fs::write(project_dir.join(file_name), line + "\n").unwrap();
    }

    #[test]
    fn discovers_distinct_existing_repo_roots() {
        let root = tempdir().unwrap();
        let repo_a = tempdir().unwrap();
        let repo_b = tempdir().unwrap();

        let proj_a = root.path().join("-slug-a");
        write_session(&proj_a, "sess-1.jsonl", repo_a.path().to_str().unwrap());
        write_session(&proj_a, "sess-2.jsonl", repo_a.path().to_str().unwrap());

        let proj_b = root.path().join("-slug-b");
        write_session(&proj_b, "sess-1.jsonl", repo_b.path().to_str().unwrap());

        let found = discover_project_roots(root.path());
        assert_eq!(found.len(), 2, "two distinct repos, deduplicated");
        assert!(found.contains(&repo_a.path().to_path_buf()));
        assert!(found.contains(&repo_b.path().to_path_buf()));
    }

    #[test]
    fn skips_repos_that_no_longer_exist_on_disk() {
        let root = tempdir().unwrap();
        let proj = root.path().join("-slug-gone");
        write_session(&proj, "sess-1.jsonl", "/this/path/does/not/exist/anywhere");
        let found = discover_project_roots(root.path());
        assert!(found.is_empty(), "a deleted/moved repo must not be watched");
    }
}
