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
///
/// Unlike `list_resumable_sessions` (which calls `parse_session` per file it
/// shows, capped at `MAX_SESSIONS`), this needs only ~1 string (the repo
/// path) per *distinct project directory* — every transcript under
/// `~/.claude/projects/<slug>/` shares one owning directory, since the slug
/// is a 1:1 encoding of one repo. So `resolved_dirs` tracks which
/// directories have already yielded a resolved root and skips re-parsing any
/// further file in that directory — without simply taking the first file
/// per directory and stopping, since that file might fail `parse_session`
/// (e.g. one with no real user turns) while a later file in the same
/// directory succeeds. This still costs one `parse_session` call per
/// directory in the worst case (every file in it fails), but typically far
/// fewer than one per file.
pub fn discover_project_roots(claude_projects_root: &Path) -> Vec<PathBuf> {
    let mut seen_cwds = HashSet::new();
    let mut resolved_dirs: HashSet<PathBuf> = HashSet::new();
    let mut roots = Vec::new();
    for f in scan::discover_session_files(claude_projects_root) {
        let dir = f.parent().map(Path::to_path_buf);
        if let Some(d) = &dir {
            if resolved_dirs.contains(d) {
                continue;
            }
        }
        let Some(summary) = recap::parse_session(&f) else {
            // This file failed to parse (e.g. no real user turns) — a later
            // file in the same directory may still succeed, so the
            // directory itself is not marked resolved.
            continue;
        };
        if let Some(d) = dir {
            resolved_dirs.insert(d);
        }
        if summary.cwd.is_empty() || !seen_cwds.insert(summary.cwd.clone()) {
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

/// Reads `path`, and if it's a file (not a symlink), within the size
/// ceiling, and valid UTF-8, snapshots it via `Db::insert_file_snapshot`.
/// Anything that fails a guard is logged and skipped, never propagated — one
/// bad file must never block the rest.
///
/// Uses `symlink_metadata` (does not follow symlinks) rather than
/// `std::fs::metadata`, consistent with `discover_session_files` and
/// `discover_jsonl_files` elsewhere in this codebase, which deliberately
/// skip symlinks — the design spec states "symlinks skipped". A symlink
/// inside a watched directory could point at an arbitrary file (e.g. an SSH
/// key); following it would read and archive that file's content verbatim,
/// forever.
fn snapshot_file(db: &Db, path: &Path, kind: &'static str) {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return,
    };
    if meta.file_type().is_symlink() {
        tracing::warn!("archive: skipping symlink (not archived): {}", path.display());
        return;
    }
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

/// Canonicalizes each scope's comparison path so it can be matched against
/// event paths the OS reports — which on macOS (FSEvents) are
/// canonicalized/realpath'd. Without this, any symlink component anywhere in
/// the chain (a symlinked `$HOME`, a repo checked out through a symlinked
/// mount, `/tmp` vs `/private/tmp`, ...) makes `matching_kind` silently
/// return `None` for a file that should have matched — nothing gets
/// archived, with no error anywhere.
///
/// A scope whose containing directory doesn't exist yet is dropped entirely
/// (there's nothing to watch until it's created) — consistent with the
/// existing accepted behavior that a `MarkdownDir` created after startup
/// isn't picked up until the next launch.
fn canonicalize_scopes(scopes: Vec<WatchScope>) -> Vec<WatchScope> {
    scopes
        .into_iter()
        .filter_map(|scope| match scope {
            WatchScope::File { path, kind } => {
                let parent = path.parent()?;
                let canonical_parent = std::fs::canonicalize(parent).ok()?;
                let file_name = path.file_name()?;
                Some(WatchScope::File { path: canonical_parent.join(file_name), kind })
            }
            WatchScope::MarkdownDir { dir, kind } => {
                let canonical_dir = std::fs::canonicalize(&dir).ok()?;
                Some(WatchScope::MarkdownDir { dir: canonical_dir, kind })
            }
        })
        .collect()
}

/// The concrete filesystem path to hand to the underlying OS watcher for
/// each scope.
///
/// A `File` scope is watched at its own path directly, not its parent
/// directory: on macOS, `notify`'s FSEvents backend is inherently recursive
/// at the OS level and filters non-recursive matches in userspace, so
/// watching a directory (e.g. an entire repo root, just for `CLAUDE.md`)
/// actually subscribes to every filesystem event anywhere under it,
/// discarding nearly all of them client-side — real, avoidable CPU/battery
/// cost, especially since one such watch would be registered per repo the
/// user has ever run Claude Code in. Watching the file's own path is
/// standard, documented, cross-platform-safe `notify` usage. A `File` scope
/// whose file doesn't exist yet is omitted — nothing to watch until it's
/// created, same accepted behavior as an as-yet-nonexistent `MarkdownDir`.
///
/// A `MarkdownDir` scope is always watched at its own directory, since new
/// files being created inside it (tomorrow's `today-*.md`) must still be
/// caught.
fn watch_targets(scopes: &[WatchScope]) -> Vec<PathBuf> {
    scopes
        .iter()
        .filter_map(|scope| match scope {
            WatchScope::File { path, .. } => path.is_file().then(|| path.clone()),
            WatchScope::MarkdownDir { dir, .. } => Some(dir.clone()),
        })
        .collect()
}

pub struct ArchiveWatcherHandle {
    _debouncer: notify_debouncer_full::Debouncer<
        notify::RecommendedWatcher,
        notify_debouncer_full::RecommendedCache,
    >,
}

/// Canonicalizes every scope's comparison path (see `canonicalize_scopes`),
/// then watches each `File` scope at its own path and each `MarkdownDir`
/// scope at its directory (see `watch_targets`), and snapshots on any
/// debounced event whose path matches `matching_kind`.
pub fn start(db: Arc<Db>, scopes: Vec<WatchScope>) -> Result<ArchiveWatcherHandle> {
    // Canonicalized once, up front — used for both the watch registration
    // below and the event-matching closure at the end of this function.
    let scopes = canonicalize_scopes(scopes);

    let (notify_tx, mut notify_rx) = mpsc::unbounded_channel::<Vec<DebouncedEvent>>();
    let mut debouncer = new_debouncer(Duration::from_millis(500), None, move |res| {
        if let Ok(events) = res {
            let _ = notify_tx.send(events);
        }
    })?;

    let targets: HashSet<PathBuf> = watch_targets(&scopes).into_iter().collect();
    for target in &targets {
        if let Err(e) = debouncer.watch(target, RecursiveMode::NonRecursive) {
            tracing::warn!("archive: failed to watch {}: {}", target.display(), e);
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

    /// The memoization must not simply take the first file per directory and
    /// give up: a directory can have a file that fails `parse_session` (e.g.
    /// zero real user turns) discovered before one that succeeds.
    /// `discover_session_files` returns newest-mtime-first, so the "first
    /// discovered" file is made the newest here.
    #[test]
    fn falls_back_to_a_later_file_in_the_same_directory_when_the_first_fails_to_parse() {
        let root = tempdir().unwrap();
        let repo = tempdir().unwrap();
        let proj = root.path().join("-slug");
        std::fs::create_dir_all(&proj).unwrap();

        // Discovered first (newest mtime): has a cwd but zero real user
        // turns (only an assistant message), so parse_session returns None
        // (turns == 0).
        let bad = proj.join("bad-session.jsonl");
        std::fs::write(
            &bad,
            format!(
                r#"{{"type":"assistant","timestamp":"2026-01-01T00:00:00Z","cwd":"{}","message":{{"role":"assistant","content":[{{"type":"text","text":"hi"}}]}}}}"#,
                repo.path().to_str().unwrap()
            ) + "\n",
        )
        .unwrap();

        // Discovered second (older mtime): a real user turn, so
        // parse_session succeeds.
        let good = proj.join("good-session.jsonl");
        std::fs::write(
            &good,
            format!(
                r#"{{"type":"user","timestamp":"2026-01-01T00:00:00Z","cwd":"{}","message":{{"role":"user","content":[{{"type":"text","text":"hello"}}]}}}}"#,
                repo.path().to_str().unwrap()
            ) + "\n",
        )
        .unwrap();
        filetime::set_file_mtime(
            &good,
            filetime::FileTime::from_system_time(
                std::time::SystemTime::now() - std::time::Duration::from_secs(3600),
            ),
        )
        .unwrap();

        let found = discover_project_roots(root.path());
        assert_eq!(
            found,
            vec![repo.path().to_path_buf()],
            "must fall back past the first (unparseable) file and still find this directory's root"
        );
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

    /// Design spec §6's missing "exclusion test": nothing under these
    /// directories (or the app's own timestamped settings backups) is ever
    /// captured. This holds "by construction" — the fixed scope builder
    /// simply never references them — but there was no regression test
    /// proving it.
    #[test]
    fn fixed_scopes_backfill_never_captures_excluded_directories() {
        let home = tempdir().unwrap();
        let claude = home.path().join(".claude");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::write(claude.join("settings.json"), "{}").unwrap();

        let excluded: &[(&str, &str)] = &[
            ("security/x.json", "{}"),
            ("session-env/y.txt", "y"),
            ("shell-snapshots/z.sh", "#!/bin/sh"),
            ("file-history/w.md", "# w"),
            ("backups/v.json", "{}"),
            ("ide/u.json", "{}"),
            ("settings.json.switchboard-1234567890", "{}"),
        ];
        for (rel, content) in excluded {
            let p = claude.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
        }

        let db_dir = tempdir().unwrap();
        let db = Db::open(db_dir.path()).unwrap();
        backfill(&db, &fixed_scopes(home.path()));

        for (rel, _) in excluded {
            let p = claude.join(rel).to_string_lossy().into_owned();
            assert!(
                db.file_snapshots_for_path(&p).unwrap().is_empty(),
                "{rel} must never be archived"
            );
        }

        // Sanity check: the real settings.json DOES get archived, so an
        // empty result above is because of exclusion, not a broken test setup.
        let settings_path = claude.join("settings.json").to_string_lossy().into_owned();
        assert_eq!(
            db.file_snapshots_for_path(&settings_path).unwrap().len(),
            1,
            "settings.json must still be archived (sanity check on the test setup)"
        );
    }

    #[test]
    fn snapshot_file_skips_symlinks_rather_than_following_them() {
        let dir = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let db = Db::open(db_dir.path()).unwrap();

        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, "super-secret-content").unwrap();

        let link = dir.path().join("linked.md");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&secret, &link).unwrap();

        snapshot_file(&db, &link, "misc");
        assert!(
            db.file_snapshots_for_path(&link.to_string_lossy()).unwrap().is_empty(),
            "a symlink must be skipped, not read/archived — the design spec requires symlinks skipped"
        );
    }

    #[test]
    fn watch_targets_watches_file_scopes_at_their_own_path_not_parent_dir() {
        let dir = tempdir().unwrap();
        let claude_md = dir.path().join("CLAUDE.md");
        std::fs::write(&claude_md, "hi").unwrap();
        let scopes = vec![WatchScope::File { path: claude_md.clone(), kind: "claude_md" }];
        assert_eq!(
            watch_targets(&scopes),
            vec![claude_md],
            "a File scope must be watched at its own path, not its parent directory \
             (this is what avoids a whole-repo-root watch just for CLAUDE.md)"
        );
    }

    #[test]
    fn watch_targets_still_watches_markdown_dir_scopes_as_a_directory() {
        let dir = tempdir().unwrap();
        let remember = dir.path().join(".remember");
        std::fs::create_dir_all(&remember).unwrap();
        let scopes = vec![WatchScope::MarkdownDir { dir: remember.clone(), kind: "memory" }];
        assert_eq!(
            watch_targets(&scopes),
            vec![remember],
            "a MarkdownDir scope must still be watched as a directory, to catch new files created inside it"
        );
    }

    #[test]
    fn watch_targets_omits_a_file_scope_whose_file_does_not_exist_yet() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("settings.local.json");
        let scopes = vec![WatchScope::File { path: missing, kind: "settings" }];
        assert!(watch_targets(&scopes).is_empty());
    }

    #[test]
    fn canonicalize_scopes_drops_a_file_scope_whose_parent_dir_does_not_exist() {
        let scopes = vec![WatchScope::File {
            path: PathBuf::from("/definitely/not/here/anywhere/CLAUDE.md"),
            kind: "claude_md",
        }];
        assert!(canonicalize_scopes(scopes).is_empty());
    }

    #[test]
    fn canonicalize_scopes_drops_a_markdown_dir_scope_that_does_not_exist_yet() {
        let scopes = vec![WatchScope::MarkdownDir {
            dir: PathBuf::from("/definitely/not/here/anywhere/.remember"),
            kind: "memory",
        }];
        assert!(canonicalize_scopes(scopes).is_empty());
    }

    /// On macOS, `tempdir()` paths commonly live under `/tmp` (or
    /// `/var/folders/...`), which is itself a symlink/alias whose realpath
    /// differs (`/private/tmp`, `/private/var/folders/...`) — but FSEvents
    /// reports change events using the realpath. A scope built from the raw
    /// tempdir() path would therefore never match an OS-reported event
    /// without canonicalization. This is a convenient, portable
    /// reproduction of the general symlink-in-the-path-chain bug.
    #[test]
    fn canonicalize_scopes_resolves_tempdir_realpath_alias() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("CLAUDE.md"), "hi").unwrap();
        let raw_path = dir.path().join("CLAUDE.md");
        let os_reported_path = std::fs::canonicalize(dir.path()).unwrap().join("CLAUDE.md");

        let raw_scopes = vec![WatchScope::File { path: raw_path.clone(), kind: "claude_md" }];
        let canonical_scopes = canonicalize_scopes(raw_scopes.clone());

        match &canonical_scopes[..] {
            [WatchScope::File { path, .. }] => assert_eq!(
                path, &os_reported_path,
                "canonicalize_scopes must resolve the scope path to what the OS actually reports"
            ),
            _ => panic!("expected exactly one File scope to survive canonicalization"),
        }

        if raw_path != os_reported_path {
            assert_eq!(
                matching_kind(&raw_scopes, &os_reported_path),
                None,
                "uncanonicalized scope silently fails to match the OS-reported realpath — this is the bug"
            );
        }
        assert_eq!(
            matching_kind(&canonical_scopes, &os_reported_path),
            Some("claude_md"),
            "after canonicalizing, the scope must match the OS-reported realpath"
        );
    }

    /// The general form of the same bug via an actual symlink: a scope path
    /// built through a symlinked directory (standing in for a symlinked
    /// `$HOME`, or a repo checked out through a symlinked mount) must, after
    /// canonicalization, match the fully-resolved path the OS would report.
    #[test]
    fn canonicalize_scopes_resolves_a_real_symlink_in_the_path_chain() {
        let real_dir = tempdir().unwrap();
        let link_parent = tempdir().unwrap();
        let link_path = link_parent.path().join("home-alias");
        #[cfg(unix)]
        std::os::unix::fs::symlink(real_dir.path(), &link_path).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(real_dir.path(), &link_path).unwrap();
        std::fs::write(link_path.join("CLAUDE.md"), "hi").unwrap();

        let raw_scopes =
            vec![WatchScope::File { path: link_path.join("CLAUDE.md"), kind: "claude_md" }];
        let os_reported_path = std::fs::canonicalize(real_dir.path()).unwrap().join("CLAUDE.md");

        assert_eq!(
            matching_kind(&raw_scopes, &os_reported_path),
            None,
            "uncanonicalized scope must NOT match the OS-reported realpath through the symlink — this is the bug"
        );

        let canonical_scopes = canonicalize_scopes(raw_scopes);
        assert_eq!(
            matching_kind(&canonical_scopes, &os_reported_path),
            Some("claude_md"),
            "after canonicalizing, the scope must match the OS-reported realpath"
        );
    }
}
