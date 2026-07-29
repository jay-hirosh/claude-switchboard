use super::record::{parse_compaction_line, parse_event_line};
use super::pricing::PricingTable;
use crate::store::{Db, StoredCompaction, StoredSessionEvent};
use anyhow::{Context, Result, anyhow};
use std::fs::{self, File};
use std::io::{BufRead, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;

pub fn claude_projects_root() -> Option<PathBuf> {
    directories::UserDirs::new()
        .map(|u| u.home_dir().join(".claude").join("projects"))
}

pub fn discover_jsonl_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    for entry in fs::read_dir(root).context("read projects dir")? {
        let entry = entry?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if !meta.is_dir() {
            continue;
        }
        let project_dir = path;
        for f in fs::read_dir(&project_dir)? {
            let f = f?;
            let fpath = f.path();
            let fmeta = fs::symlink_metadata(&fpath)?;
            if fmeta.file_type().is_symlink() {
                continue;
            }
            // A session directory (`<sessionId>/`) may hold subagent
            // transcripts at `<sessionId>/subagents/agent-*.jsonl`. The
            // watcher already picks these up live (RecursiveMode::Recursive),
            // so skipping them here made the backfill disagree with the
            // watcher — 125 of 138 files were never ingested, and their API
            // calls are real spend that the Cost tab is built to display.
            if fmeta.is_dir() {
                let subagents = fpath.join("subagents");
                let Ok(entries) = fs::read_dir(&subagents) else {
                    continue;
                };
                for s in entries {
                    let s = s?;
                    let spath = s.path();
                    let smeta = fs::symlink_metadata(&spath)?;
                    if smeta.file_type().is_symlink() || !smeta.is_file() {
                        continue;
                    }
                    if spath.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                        continue;
                    }
                    if smeta.len() > MAX_FILE_BYTES {
                        tracing::warn!("skipping oversized file (>100MB): {}", spath.display());
                        continue;
                    }
                    files.push(spath);
                }
                continue;
            }
            if !fmeta.is_file() {
                continue;
            }
            if fpath.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if fmeta.len() > MAX_FILE_BYTES {
                tracing::warn!("skipping oversized file (>100MB): {}", fpath.display());
                continue;
            }
            files.push(fpath);
        }
    }
    Ok(files)
}

pub fn ingest_file(
    db: &Db,
    pricing: &PricingTable,
    path: &Path,
    projects_root: &Path,
) -> Result<usize> {
    let meta = fs::metadata(path)?;
    let file_len = meta.len() as i64;
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0);

    // P1-20: use to_str() so non-UTF-8 paths surface an error rather than
    // producing a lossy key that silently never matches the cursor store.
    let key = path
        .to_str()
        .ok_or_else(|| anyhow!("non-UTF-8 path: {:?}", path))?
        .to_owned();
    let (prev_mtime, mut offset) = db.get_cursor(&key)?.unwrap_or((0, 0));

    if file_len < offset {
        tracing::info!("truncation detected, resetting cursor for {}", key);
        offset = 0;
    } else if prev_mtime == mtime_ns && file_len == offset {
        return Ok(0);
    }

    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(offset as u64))?;

    let project = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    // P1-7: store relative path from the Claude projects root so that
    // source_file values are not machine-specific absolute paths.
    let source_file_path = path
        .strip_prefix(projects_root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned();

    let mut reader = std::io::BufReader::new(f);
    let mut buf = Vec::new();
    let mut stored = Vec::<StoredSessionEvent>::new();
    let mut compactions = Vec::<StoredCompaction>::new();
    let mut consumed: i64 = offset;

    loop {
        buf.clear();
        let line_start = consumed;
        let n = reader.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        if *buf.last().unwrap() != b'\n' {
            break;
        }
        consumed += n as i64;
        let text = match std::str::from_utf8(&buf) {
            Ok(t) => t.trim(),
            Err(_) => continue,
        };
        if text.is_empty() {
            continue;
        }
        // Claude Code JSONLs interleave many non-usage record types
        // (`user`, `permission-mode`, `attachment`, `system`, `last-prompt`, …);
        // only `assistant` lines carry a `message.usage` payload. parse_event_line
        // returns None for everything else, so silent skip is correct here.
        if let Some(ev) = parse_event_line(text, &project) {
            let cost = pricing.cost_for(
                &ev.model,
                ev.input_tokens,
                ev.output_tokens,
                ev.cache_read_tokens,
                ev.cache_creation_5m_tokens,
                ev.cache_creation_1h_tokens,
            );
            // Structural fallback only when the JSONL line lacked the
            // canonical Claude identifiers (older Claude Code versions or
            // hand-written test fixtures). For modern Claude Code, this
            // path isn't taken — the parser produces a real
            // "{requestId}:{message.id}" key that survives Claude Code
            // re-writing the same usage block to multiple offsets.
            let event_id = ev
                .event_id
                .unwrap_or_else(|| format!("{}:{}", key, line_start));
            stored.push(StoredSessionEvent {
                ts: ev.ts,
                project: ev.project,
                model: ev.model,
                input_tokens: ev.input_tokens,
                output_tokens: ev.output_tokens,
                cache_read_tokens: ev.cache_read_tokens,
                cache_creation_5m_tokens: ev.cache_creation_5m_tokens,
                cache_creation_1h_tokens: ev.cache_creation_1h_tokens,
                cost_usd: cost,
                source_file: source_file_path.clone(),
                source_line: line_start,
                event_id,
            });
        } else if let Some(c) = parse_compaction_line(text) {
            // `else if`: a compaction is a `system` line, so it can never also
            // be a usage-bearing assistant line — no need to test both.
            compactions.push(StoredCompaction {
                ts: c.ts,
                source_file: source_file_path.clone(),
                trigger: c.trigger,
                pre_tokens: c.pre_tokens,
                post_tokens: c.post_tokens,
                uuid: c.uuid,
            });
        }
    }
    let inserted = db.ingest_atomic(&key, &stored, &compactions, mtime_ns, consumed)?;
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Db;
    use chrono::Utc;
    use std::io::Write;
    use std::sync::Arc;
    use tempfile::tempdir;

    /// Build a minimal assistant JSONL line that parse_event_line will accept.
    fn assistant_line(req_id: &str, msg_id: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{}","cwd":"/tmp/project","requestId":"{}","message":{{"id":"{}","model":"claude-sonnet-4-6","usage":{{"input_tokens":1,"output_tokens":1}}}}}}"#,
            Utc::now().to_rfc3339(),
            req_id,
            msg_id,
        )
    }

    /// Write `lines` to a temp file inside a project sub-directory so the
    /// walker can derive a project name from the parent directory.
    fn write_jsonl(dir: &tempfile::TempDir, lines: &[String]) -> PathBuf {
        let project_dir = dir.path().join("my-project");
        std::fs::create_dir_all(&project_dir).unwrap();
        let path = project_dir.join("session.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(f, "{}", line).unwrap();
        }
        path
    }

    /// Verify that two concurrent `ingest_file` calls on the same file produce
    /// no duplicate rows and leave the cursor at the file's end. This is the
    /// regression test for the race where two separate `insert_events` +
    /// `set_cursor` calls could let the slower caller regress the cursor.
    #[test]
    fn concurrent_ingest_no_duplicates_and_correct_cursor() {
        let dir = tempdir().unwrap();
        // Use a separate directory for the DB so it doesn't collide with the
        // JSONL project sub-directory.
        let db_dir = dir.path().join("db");
        std::fs::create_dir_all(&db_dir).unwrap();
        let db = Arc::new(Db::open(&db_dir).unwrap());
        let pricing = Arc::new(PricingTable::bundled().unwrap());

        let lines: Vec<String> = (0..10)
            .map(|i| assistant_line(&format!("req_{i}"), &format!("msg_{i}")))
            .collect();
        let path = write_jsonl(&dir, &lines);
        let file_len = std::fs::metadata(&path).unwrap().len() as i64;

        // The projects root is the temp dir itself (the JSONL lives in
        // <dir>/my-project/session.jsonl, so dir.path() is the root).
        let projects_root = dir.path().to_path_buf();

        // Spawn two threads; both race to ingest the same file.
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let db = Arc::clone(&db);
                let pricing = Arc::clone(&pricing);
                let path = path.clone();
                let root = projects_root.clone();
                std::thread::spawn(move || ingest_file(&db, &pricing, &path, &root))
            })
            .collect();

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        // Both threads must succeed (the Mutex ensures they serialize).
        for r in &results {
            assert!(r.is_ok(), "ingest_file failed: {:?}", r);
        }
        // Total inserted across both threads must equal exactly 10 — no dups.
        let total_inserted: usize = results.iter().map(|r| r.as_ref().unwrap()).sum();
        assert_eq!(total_inserted, 10, "expected exactly 10 unique events");

        // Cursor byte_offset must be at the file end (not regressed by the
        // slower thread). The cursor key is the absolute path string.
        let key = path.to_str().expect("test path must be UTF-8").to_owned();
        let (_mtime, offset) = db.get_cursor(&key).unwrap().expect("cursor missing");
        assert_eq!(offset, file_len, "cursor must be at end-of-file");
    }

    #[test]
    fn discovers_subagent_transcripts_one_level_deeper() {
        let root = tempdir().unwrap();
        let project = root.path().join("-Users-me-proj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(project.join("sess-1.jsonl"), "{}\n").unwrap();

        let subagents = project.join("sess-1").join("subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        std::fs::write(subagents.join("agent-aaa.jsonl"), "{}\n").unwrap();
        std::fs::write(subagents.join("agent-bbb.jsonl"), "{}\n").unwrap();

        let found = discover_jsonl_files(root.path()).unwrap();
        assert_eq!(found.len(), 3, "top-level transcript plus both subagents");
        assert!(
            found.iter().any(|p| p.ends_with("agent-aaa.jsonl")),
            "subagent transcripts must be backfilled — their API calls cost money"
        );
    }

    #[test]
    fn ignores_non_subagent_subdirectories() {
        let root = tempdir().unwrap();
        let project = root.path().join("-Users-me-proj");
        std::fs::create_dir_all(project.join("sess-1").join("something-else")).unwrap();
        std::fs::write(
            project.join("sess-1").join("something-else").join("x.jsonl"),
            "{}\n",
        )
        .unwrap();
        let found = discover_jsonl_files(root.path()).unwrap();
        assert!(found.is_empty(), "only subagents/ is descended into");
    }
}
