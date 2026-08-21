# Durable Claude archive — persist ~/.claude + ~/.remember beyond deletion

**Status:** Design ready for review
**Date:** 2026-08-21
**Tracking PR:** TBD

## 1. Problem

Everything the app knows about a user's Claude Code history currently lives in two places: the operational `store.db` (which deliberately prunes `api_snapshots` after 30 days and `session_events` after 90, for query-speed reasons) and the raw `~/.claude` directory itself, which the user fully controls and can delete at any time (reinstall, cleanup, disk issue). If either happens, that history is gone for good — there is no copy of it anywhere else.

The user wants a durable local archive: read the data that matters out of `~/.claude` and `~/.remember`, and store it in the app's own database, so deleting either source directory afterward loses nothing. This is phase 1 of a two-phase effort — phase 2 (syncing this archive across machines via a user-built backend API) is an explicit follow-on, out of scope here, and will get its own spec once this data model is settled.

## 2. Goals / non-goals

**Goals**
- Full-fidelity capture of raw session transcripts (`~/.claude/projects/<slug>/*.jsonl`), not just the aggregated stats the app already computes from them.
- Capture of the smaller but still valuable surface under the fixed `~/.claude` root: `settings.json`, `settings.local.json`, the global `CLAUDE.md`, `plans/*.md`, `history.jsonl`, `statusline-usage.json`, `mcp-needs-auth-cache.json`.
- Capture of the per-project surface: each repo's own `CLAUDE.md` and its `.remember/*.md` memory files (`now.md`, `today-*.md`, `recent.md`, `archive.md`, `core-memories.md`, rotated `archive-*.md`). **Correction from the original brainstorm:** these do not live under a fixed path — `~/.remember` itself is empty scaffolding (`logs/`, `run/`, `tmp/`); the real content lives at each repo's own root (e.g. `<repo>/.remember/archive.md`), the same way `CLAUDE.md` does. The repo list is derived by reusing the existing `sessions::recap::parse_session` cwd resolution (already correct, since it reads the JSONL's own `cwd` field) rather than by de-slugifying `~/.claude/projects/<slug>` directory names, which is lossy wherever a real path component contains a literal `-`.
- Once ingested, this data survives `~/.claude`, per-repo `.remember/`, and/or the repos themselves being deleted.
- Reuse the existing `store.db` connection, migration chain, and file lock rather than introducing a second database.

**Non-goals (this phase)**
- No live discovery of brand-new repos mid-session: the repo list (and therefore per-repo `CLAUDE.md`/`.remember/` watching) is (re)computed at app startup, alongside the existing JSONL backfill. A repo touched for the first time in a running session gets its files archived starting from the next app launch, not immediately. Revisit only if this proves to matter in practice.
- No cross-machine sync — that's phase 2, a separate spec.
- No search/browse UI over the archive — storage only. A future feature can add FTS5 or a browsing view once there's an actual need for it.
- No encryption at rest beyond what the app already relies on (FileVault/disk encryption + OS file permissions) — consistent with the existing rationale in §2.5.1 of the main design doc for not using the OS keychain.
- No change to the existing `api_snapshots`/`session_events` retention windows — they keep pruning at 30/90 days for query performance; their source material is now recoverable from `transcript_lines` if ever needed.
- No capture of transient/OS-level scaffolding directories under `~/.claude`: `security/`, `session-env/`, `shell-snapshots/`, `file-history/`, `backups/`, `ide/`. These are ephemeral session/audit state, not durable history, and capturing 1200+ files of it adds noise and disk cost with no archival value.
- No capture of the app's own timestamped settings backups (`settings.json.switchboard-*`) — only the canonical `settings.json` is snapshotted.

## 3. Architecture

A new `archive` module alongside the existing `store`, `jsonl_parser`, and `sessions` modules in `src-tauri/src/`, sharing the existing `store.db` file, `Db` handle, and file lock — no new connection.

```
store.db
├── api_snapshots     (existing, 30d prune)
├── session_events     (existing, 90d prune)
├── notification_state (existing)
├── transcript_lines   (new, never pruned)
└── file_snapshots     (new, never pruned)
```

- **`store/migrations/0013_archive_tables.sql`** — the two new table migrations, appended to the existing migration chain in `store/mod.rs`.
- **Transcript archiving, inline in `jsonl_parser/walker.rs`** — `ingest_file` already reads every raw line in its loop to decide if it's an event or a compaction; it additionally collects every non-empty raw line into a `Vec<StoredTranscriptLine>` and writes them via a new `Db::insert_transcript_lines`. This call happens *before* the existing `db.ingest_atomic(...)` (which is what advances the cursor), and its errors are logged and swallowed rather than propagated — so a transcript-archive failure can never block analytics ingestion, and (because the cursor hasn't advanced yet) a crash between the two calls just re-does both on the next read rather than losing the lines. No change to `ingest_atomic`'s signature or its existing call sites.
- **`archive_watcher.rs`** (new top-level module) — two responsibilities:
  1. **Repo discovery**: `discover_project_roots` walks every session file via the existing `sessions::scan::discover_session_files` + `sessions::recap::parse_session`, collects the distinct `.cwd` values into a deduped list of existing directories. This is the same cwd resolution the Repo tab already relies on — reused here specifically because de-slugifying directory names would misfire on repo paths containing a literal `-`.
  2. **File watching**: a `notify-debouncer-full` watcher (same pattern as `jsonl_parser::watcher`) covering the fixed `~/.claude` targets (`settings.json`, `settings.local.json`, `CLAUDE.md`, `history.jsonl`, `statusline-usage.json`, `mcp-needs-auth-cache.json`, and non-recursively `plans/*.md`) plus, for each discovered repo root, `<repo>/CLAUDE.md` and non-recursively `<repo>/.remember/*.md` (a directory scan rather than hardcoded filenames, since `today-*.md` and rotated `archive-*.md` are variably named). On any change, hashes the new content and inserts into `file_snapshots` via a new `Db::insert_file_snapshot` only if the hash differs from the path's last snapshot.
- **Startup backfill** — repo discovery and a full sweep of every fixed + per-repo target run on every launch, not just once. Unlike the JSONL backfill (genuinely expensive — scans potentially 100s of MB of transcripts), this sweep is cheap: it's a few config files plus a handful of small `.md` files per repo, and both `transcript_lines` (offset-keyed) and `file_snapshots` (hash-keyed) already dedupe unchanged content to a no-op. Running it every launch, rather than gating it behind a one-time-completed flag, is what makes a newly-touched repo's `CLAUDE.md`/`.remember/` start being archived the next time the app opens, with no extra bookkeeping needed.

## 4. Data model

```sql
-- Append-only, one row per JSONL line ever observed. Source of truth for
-- everything session_events summarizes, kept forever regardless of pruning
-- or ~/.claude deletion.
CREATE TABLE transcript_lines (
    id            INTEGER PRIMARY KEY,
    project_slug  TEXT NOT NULL,
    session_id    TEXT NOT NULL,
    jsonl_path    TEXT NOT NULL,
    line_no       INTEGER NOT NULL,
    raw_line      TEXT NOT NULL,
    ingested_at   INTEGER NOT NULL,
    UNIQUE(jsonl_path, line_no)
);

-- Snapshot-on-change for every other file in scope. One row per distinct
-- content version; identical content is never duplicated.
CREATE TABLE file_snapshots (
    id           INTEGER PRIMARY KEY,
    source_path  TEXT NOT NULL,
    kind         TEXT NOT NULL,   -- 'settings' | 'claude_md' | 'memory' | 'plan' | 'misc'
    content      TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    captured_at  INTEGER NOT NULL,
    UNIQUE(source_path, content_hash)
);
```

`kind` is a plain label for future filtering/UI — there's no per-kind schema branching, so adding a new watched file later (e.g. a new plugin's state file) is a config change to the watcher's path list, not a migration.

## 5. Ingestion mechanics & error handling

- **JSONL tailing:** identical semantics to the existing analytics watcher — cursor per file, truncation-to-0 detection, symlinks skipped, one level deep under `projects/`.
- **Small-file watcher:** debounced (~500ms) since editors/tools can write files in bursts; reads the whole file on a change event, hashes it, inserts only if the hash is new. A file that doesn't exist yet (e.g. no `settings.local.json`) simply isn't captured until `notify` reports its creation.
- **Binary/oversized-file guard:** any file over a size ceiling (5MB) or that fails UTF-8 validation is skipped with a logged warning rather than stored. Everything in this surface is expected to be small text/config, so a hit here signals something unexpected, not a case to silently truncate or store as a blob.
- **Permission errors:** logged and skipped per-path; one unreadable file never blocks ingestion of the rest.
- **Failure isolation:** archive ingestion is independent of the existing analytics pipeline — a failure writing to `transcript_lines`/`file_snapshots` never affects `session_events`/`api_snapshots`, and vice versa.

## 6. Testing

- Fixture-based tests mirroring the existing `tests/fixtures/jsonl/` pattern: a fake `~/.claude`/`~/.remember`-shaped tree (injectable root path) with sample settings/CLAUDE.md/memory/plan files, verifying correct rows land in both tables.
- Dedup test: writing identical content twice produces one `file_snapshots` row, not two.
- Truncation/rotation test: reusing the existing JSONL cursor-reset test pattern, extended to assert `transcript_lines` also recovers correctly.
- Pruning-exemption test: run the existing prune routine, assert `transcript_lines`/`file_snapshots` row counts are unchanged.
- Binary/oversized-file test: confirms skip-with-log behavior rather than a crash or garbage row.
- Exclusion test: confirms files under `security/`, `session-env/`, `shell-snapshots/`, `file-history/`, `backups/`, `ide/`, and `settings.json.switchboard-*` are never captured.

## 7. Relationship to phase 2 (sync)

This spec deliberately stops at "durable and queryable on one machine." Once this ships and the schema has proven itself, phase 2 will design the sync protocol (what gets pushed/pulled, conflict resolution across machines, the API contract against the user's own backend) against `transcript_lines`/`file_snapshots` as the stable local source of truth. That design is out of scope here.
