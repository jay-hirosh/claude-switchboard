use crate::sessions::{recap, scan};
use crate::store::{Db, StoredFileSnapshot};
use anyhow::Result;
use notify::RecursiveMode;
use notify_debouncer_full::{new_debouncer, DebouncedEvent};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

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

const MAX_SNAPSHOT_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Clone)]
pub enum WatchScope {
    /// A single fixed file (settings.json, CLAUDE.md, history.jsonl, ...).
    File { path: PathBuf, kind: &'static str },
    /// Any *.md file directly inside this directory (plans/, .remember/) —
    /// covers files created after startup (e.g. tomorrow's today-*.md),
    /// which a fixed file list would miss.
    MarkdownDir { dir: PathBuf, kind: &'static str },
}

/// Fixed targets under ~/.claude, independent of any repo.
pub fn fixed_scopes(home: &Path) -> Vec<WatchScope> {
    let claude = home.join(".claude");
    vec![
        WatchScope::File { path: claude.join("settings.json"), kind: "settings" },
        WatchScope::File { path: claude.join("settings.local.json"), kind: "settings" },
        WatchScope::File { path: claude.join("CLAUDE.md"), kind: "claude_md" },
        WatchScope::File { path: claude.join("history.jsonl"), kind: "misc" },
        WatchScope::File { path: claude.join("statusline-usage.json"), kind: "misc" },
        WatchScope::File { path: claude.join("mcp-needs-auth-cache.json"), kind: "misc" },
        WatchScope::MarkdownDir { dir: claude.join("plans"), kind: "plan" },
    ]
}

/// Per-repo targets: the repo's own CLAUDE.md plus every *.md file directly
/// under its .remember/.
pub fn repo_scopes(repo_root: &Path) -> Vec<WatchScope> {
    vec![
        WatchScope::File { path: repo_root.join("CLAUDE.md"), kind: "claude_md" },
        WatchScope::MarkdownDir { dir: repo_root.join(".remember"), kind: "memory" },
    ]
}

/// Every concrete file a scope currently covers. For a MarkdownDir this
/// expands to whatever *.md files exist right now; `start`'s live watcher
/// separately covers files that appear later via `matching_kind`.
fn expand(scope: &WatchScope) -> Vec<(PathBuf, &'static str)> {
    match scope {
        WatchScope::File { path, kind } => vec![(path.clone(), *kind)],
        WatchScope::MarkdownDir { dir, kind } => {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return Vec::new();
            };
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
                .map(|p| (p, *kind))
                .collect()
        }
    }
}

/// Reads and snapshots every file every scope currently covers. Cheap: both
/// `transcript_lines` and `file_snapshots` dedupe unchanged content, so
/// running this on every launch (not just once) costs one hash comparison
/// per already-seen file.
pub fn backfill(db: &Db, scopes: &[WatchScope]) {
    for scope in scopes {
        for (path, kind) in expand(scope) {
            snapshot_file(db, &path, kind);
        }
    }
}

/// Reads `path`, and if it's a file, within the size ceiling, and valid
/// UTF-8, snapshots it via `Db::insert_file_snapshot`. Anything that fails a
/// guard is logged and skipped, never propagated — one bad file must never
/// block the rest.
fn snapshot_file(db: &Db, path: &Path, kind: &'static str) {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return,
    };
    if !meta.is_file() {
        return;
    }
    if meta.len() > MAX_SNAPSHOT_BYTES {
        tracing::warn!(
            "archive: skipping oversized file (>{}MB): {}",
            MAX_SNAPSHOT_BYTES / (1024 * 1024),
            path.display()
        );
        return;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("archive: skipping unreadable/non-UTF-8 file {}: {}", path.display(), e);
            return;
        }
    };
    let content_hash = format!("{:x}", Sha256::digest(content.as_bytes()));
    let snap = StoredFileSnapshot {
        source_path: path.to_string_lossy().into_owned(),
        kind: kind.to_string(),
        content,
        content_hash,
    };
    if let Err(e) = db.insert_file_snapshot(&snap) {
        tracing::warn!("archive: failed to store snapshot for {}: {}", path.display(), e);
    }
}

/// `None` if the OS home directory can't be resolved (matches the existing
/// `jsonl_parser::walker::claude_projects_root` fallback behavior).
pub fn home_dir() -> Option<PathBuf> {
    directories::UserDirs::new().map(|u| u.home_dir().to_path_buf())
}

fn matching_kind(scopes: &[WatchScope], path: &Path) -> Option<&'static str> {
    for scope in scopes {
        match scope {
            WatchScope::File { path: p, kind } if p == path => return Some(kind),
            WatchScope::MarkdownDir { dir, kind }
                if path.parent() == Some(dir.as_path())
                    && path.extension().and_then(|e| e.to_str()) == Some("md") =>
            {
                return Some(kind);
            }
            _ => {}
        }
    }
    None
}

pub struct ArchiveWatcherHandle {
    _debouncer: notify_debouncer_full::Debouncer<
        notify::RecommendedWatcher,
        notify_debouncer_full::RecommendedCache,
    >,
}

/// Watches every scope's parent directory (or, for a MarkdownDir, the
/// directory itself) non-recursively, and snapshots on any debounced event
/// whose path matches `matching_kind`. A directory that doesn't exist yet
/// (e.g. no settings.local.json ever created, so its parent may still exist
/// but the file itself won't trigger until created — this is fine, `notify`
/// watches the directory) is simply not registered if the directory itself
/// is missing.
pub fn start(db: Arc<Db>, scopes: Vec<WatchScope>) -> Result<ArchiveWatcherHandle> {
    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<Vec<DebouncedEvent>>();
    let mut debouncer = new_debouncer(Duration::from_millis(500), None, move |res| {
        if let Ok(events) = res {
            let _ = notify_tx.send(events);
        }
    })?;

    let mut watch_dirs: HashSet<PathBuf> = HashSet::new();
    for scope in &scopes {
        let dir = match scope {
            WatchScope::File { path, .. } => path.parent().map(|p| p.to_path_buf()),
            WatchScope::MarkdownDir { dir, .. } => Some(dir.clone()),
        };
        if let Some(dir) = dir {
            watch_dirs.insert(dir);
        }
    }
    for dir in &watch_dirs {
        if dir.is_dir() {
            let _ = debouncer.watch(dir, RecursiveMode::NonRecursive);
        }
    }

    let db_clone = db.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(events) = notify_rx.recv().await {
            let mut touched = HashSet::<PathBuf>::new();
            for e in &events {
                touched.extend(e.paths.iter().cloned());
            }
            for p in touched {
                if let Some(kind) = matching_kind(&scopes, &p) {
                    snapshot_file(&db_clone, &p, kind);
                }
            }
        }
    });

    Ok(ArchiveWatcherHandle { _debouncer: debouncer })
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

    #[test]
    fn fixed_scopes_covers_expected_claude_paths() {
        let home = tempdir().unwrap();
        let scopes = fixed_scopes(home.path());
        let claude = home.path().join(".claude");
        let files: Vec<_> = scopes
            .iter()
            .filter_map(|s| match s {
                WatchScope::File { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        assert!(files.contains(&claude.join("settings.json")));
        assert!(files.contains(&claude.join("settings.local.json")));
        assert!(files.contains(&claude.join("CLAUDE.md")));
        assert!(files.contains(&claude.join("history.jsonl")));
        assert!(files.contains(&claude.join("statusline-usage.json")));
        assert!(files.contains(&claude.join("mcp-needs-auth-cache.json")));
        assert!(
            scopes.iter().any(
                |s| matches!(s, WatchScope::MarkdownDir { dir, .. } if dir == &claude.join("plans"))
            ),
            "plans/ is scanned as a directory, not named files"
        );
    }

    #[test]
    fn repo_scopes_covers_claude_md_and_remember_dir() {
        let repo = tempdir().unwrap();
        let scopes = repo_scopes(repo.path());
        assert!(scopes.iter().any(
            |s| matches!(s, WatchScope::File { path, .. } if path == &repo.path().join("CLAUDE.md"))
        ));
        assert!(scopes.iter().any(
            |s| matches!(s, WatchScope::MarkdownDir { dir, .. } if dir == &repo.path().join(".remember"))
        ));
    }

    #[test]
    fn backfill_snapshots_fixed_files_and_dynamic_markdown_dir_contents() {
        let home = tempdir().unwrap();
        let claude = home.path().join(".claude");
        std::fs::create_dir_all(claude.join("plans")).unwrap();
        std::fs::write(claude.join("settings.json"), "{}").unwrap();
        std::fs::write(claude.join("plans").join("one.md"), "# plan one").unwrap();
        std::fs::write(claude.join("plans").join("two.md"), "# plan two").unwrap();

        let db_dir = tempdir().unwrap();
        let db = Db::open(db_dir.path()).unwrap();

        backfill(&db, &fixed_scopes(home.path()));

        let settings_path = claude.join("settings.json").to_string_lossy().into_owned();
        assert_eq!(db.file_snapshots_for_path(&settings_path).unwrap().len(), 1);

        let plan_one = claude.join("plans").join("one.md").to_string_lossy().into_owned();
        let plan_two = claude.join("plans").join("two.md").to_string_lossy().into_owned();
        assert_eq!(db.file_snapshots_for_path(&plan_one).unwrap().len(), 1);
        assert_eq!(db.file_snapshots_for_path(&plan_two).unwrap().len(), 1);
    }

    #[test]
    fn snapshot_file_skips_oversized_and_binary_content() {
        let dir = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let db = Db::open(db_dir.path()).unwrap();

        let huge = dir.path().join("huge.md");
        std::fs::write(&huge, vec![b'a'; (MAX_SNAPSHOT_BYTES + 1) as usize]).unwrap();
        snapshot_file(&db, &huge, "misc");
        assert!(db.file_snapshots_for_path(&huge.to_string_lossy()).unwrap().is_empty());

        let binary = dir.path().join("binary.md");
        std::fs::write(&binary, [0xFF, 0xFE, 0x00, 0xD8]).unwrap();
        snapshot_file(&db, &binary, "misc");
        assert!(db.file_snapshots_for_path(&binary.to_string_lossy()).unwrap().is_empty());

        let normal = dir.path().join("normal.md");
        std::fs::write(&normal, "hello").unwrap();
        snapshot_file(&db, &normal, "misc");
        assert_eq!(db.file_snapshots_for_path(&normal.to_string_lossy()).unwrap().len(), 1);
    }

    #[test]
    fn matching_kind_covers_new_files_in_a_markdown_dir() {
        let scopes = vec![
            WatchScope::File {
                path: PathBuf::from("/home/.claude/settings.json"),
                kind: "settings",
            },
            WatchScope::MarkdownDir { dir: PathBuf::from("/repo/.remember"), kind: "memory" },
        ];
        assert_eq!(
            matching_kind(&scopes, Path::new("/repo/.remember/today-2026-08-22.md")),
            Some("memory"),
            "a file created after scopes were built must still match by directory + extension"
        );
        assert_eq!(
            matching_kind(&scopes, Path::new("/home/.claude/settings.json")),
            Some("settings")
        );
        assert_eq!(
            matching_kind(&scopes, Path::new("/repo/.remember/logs/x.md")),
            None,
            "nested paths are not the watched directory itself"
        );
        assert_eq!(
            matching_kind(&scopes, Path::new("/repo/.remember/not-markdown.txt")),
            None
        );
        assert_eq!(matching_kind(&scopes, Path::new("/unrelated/file.md")), None);
    }
}
