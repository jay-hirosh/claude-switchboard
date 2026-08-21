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
- Capture of the smaller but still valuable surface: `~/.claude/settings.json`, `~/.claude/settings.local.json`, `~/.claude/CLAUDE.md` and every project's `CLAUDE.md`, `~/.claude/plans/*`, `~/.claude/history.jsonl`, `~/.claude/statusline-usage.json`, `~/.claude/mcp-needs-auth-cache.json`, and `~/.remember/*.md` (the cross-session memory files: `now.md`, `today-*.md`, `recent.md`, `archive.md`, `core-memories.md`).
- Once ingested, this data survives `~/.claude` and/or `~/.remember` being deleted entirely.
- Reuse the existing `store.db` connection, migration chain, and file lock rather than introducing a second database.

**Non-goals (this phase)**
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

- **`archive/schema.rs`** — the two new table migrations, appended to the existing migration chain in `store/mod.rs`.
- **`archive/transcript_ingest.rs`** — extends the existing `jsonl_parser` watcher/walker. Every time it tails a new line from `~/.claude/projects/<slug>/*.jsonl` for analytics purposes, it also writes the raw line verbatim into `transcript_lines`, using the same cursor and the same file-truncation handling already built for the analytics path. This is an additional write alongside the existing one, not a second watcher.
- **`archive/file_watcher.rs`** — a new lightweight watcher, same `notify` crate and pattern as the JSONL watcher, covering the small-file surface listed in §2. On any change it hashes the new content and inserts into `file_snapshots` only if the hash differs from the last snapshot for that path.
- **Startup backfill** — an initial full sweep of everything in scope (reusing the `backfill(days)` pattern already planned for `jsonl_parser`), so pre-existing history isn't missed the first time this ships. Runs once, tracked via a marker row (same pattern as `notification_state`'s crossing-memory) so it doesn't re-scan on every launch.

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
