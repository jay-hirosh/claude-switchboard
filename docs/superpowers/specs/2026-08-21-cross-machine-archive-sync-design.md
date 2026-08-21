# Cross-machine archive sync

**Status:** Design ready for review
**Date:** 2026-08-21
**Tracking PR:** TBD

## 1. Problem

Phase 1 (see `docs/superpowers/specs/2026-08-21-durable-claude-archive-design.md`) made `~/.claude` + per-repo history durable on a single machine — deleting the source files loses nothing already archived. It does not help across machines: a user working from two or more computers has a separate, disconnected archive on each one. This phase adds sync: each device periodically exchanges its newly-archived rows with a backend, and pulls down what every other device has archived, so every machine ends up with a full local mirror of the whole fleet's history and keeps working exactly as it does today (the UI only ever reads local SQLite — nothing about phase 1's "no filesystem/network access from the frontend" rule changes).

## 2. Goals / non-goals

**Goals**
- Each paired device periodically pushes its own new `transcript_lines`/`file_snapshots` rows to a backend, and pulls down every other device's new rows under the same account, merging them into its own local `store.db`.
- Works fully offline between syncs — the local mirror is what the existing UI reads, so nothing degrades when the backend is unreachable.
- A backend built in Rust (axum + Postgres), sharing wire-format types with the desktop app via a new crate in this repo's Cargo workspace, so the two never drift on payload shapes.
- Lightweight pairing (a short-lived code shown on one device, entered on another) rather than full account/password auth — matches this being a personal tool for a handful of devices, while still modeling `users`/`devices` from the start so real multi-user growth later is additive, not a rewrite.

**Non-goals (this phase)**
- No end-to-end payload encryption beyond TLS-in-transit + bearer-token auth — sync-everything was a deliberate choice, resting on the backend being the user's own, properly secured infrastructure.
- No real-time/websocket push — periodic polling only, matching every other background job in this app (`poll_loop.rs`).
- No multi-region or high-availability backend deployment — personal scale, a single instance is fine.
- No automated account/device recovery if an API key is lost — re-pairing a device is the recovery path.
- No UI work surfacing "which device this came from" — this phase gets the data into the local mirror; a device filter/badge in the report UI is a separate future feature.
- No selective sync (excluding specific `kind`s) — everything archived locally syncs, per the explicit choice to keep this simple and rely on the backend's own security.

## 3. Architecture

```
Device A (Tauri app)                    Device B (Tauri app)
  local store.db                          local store.db
  transcript_lines ─┐                     ┌─ transcript_lines
  file_snapshots  ──┤                     ├── file_snapshots
                    │   periodic sync     │
                    ▼   (push + pull)     ▼
              ┌─────────────────────────────────┐
              │   sync backend (axum, Rust)     │
              │   Postgres: archive_rows        │
              │   tagged user_id + device_id +  │
              │   server-assigned seq number    │
              └─────────────────────────────────┘
```

The sync model is an **append-only replicated log, not a CRDT**. Every archived row already belongs to exactly one device (nothing is ever edited by a different device — both tables are insert-only and never pruned), so there is no write-write conflict to resolve: each device only ever pushes rows tagged with its own `device_id`, and only ever pulls rows tagged with *other* devices' `device_id`s.

**Repo layout:** a new Cargo workspace at the repo root, with three members:
- `src-tauri/` (existing desktop app, becomes a workspace member)
- `sync-backend/` (new axum service)
- `archive-sync-types/` (new shared crate: the wire-format DTOs both sides serialize/deserialize)

## 4. Data model

**Local (client) — schema v14 migration**, additive to phase 1's tables:
```sql
ALTER TABLE transcript_lines ADD COLUMN device_id TEXT NOT NULL DEFAULT '';
ALTER TABLE file_snapshots  ADD COLUMN device_id TEXT NOT NULL DEFAULT '';
-- Backfill existing rows: UPDATE ... SET device_id = '<this install's device_id>' WHERE device_id = ''
-- Replace the old UNIQUE constraints with device_id-scoped ones:
--   UNIQUE(device_id, jsonl_path, line_no)
--   UNIQUE(device_id, source_path, content_hash)
```
`device_id` is a UUID generated once per install and stored as a `settings` row (same key-value pattern as `migration_completed`/`warmup_consent_granted` — no new table). Sync bookkeeping (backend URL, API key, per-table push/pull watermarks) also lives as `settings` rows for the same reason — a handful of scalars doesn't warrant a dedicated table.

This is also the resolution to phase 1's parked finding about `file_snapshots.source_path` being absolute and machine-specific: no relative-path normalization is needed. `(device_id, source_path, content_hash)` is unambiguous — `device_id` already distinguishes "my settings.json on machine A" from "my settings.json on machine B," which is what actually made the absolute path ambiguous in the first place.

Rows **pulled from other devices** land in these same two local tables, tagged with the *remote* device's `device_id` — every existing query against `transcript_lines`/`file_snapshots` keeps working unchanged; the tables just become fuller.

**Backend (Postgres):**
```sql
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE devices (
    id UUID PRIMARY KEY,               -- same value as the client's local device_id
    user_id UUID NOT NULL REFERENCES users(id),
    name TEXT NOT NULL,
    api_key_hash TEXT NOT NULL,         -- SHA-256 of the raw key (high-entropy random token, not a user secret — a fast hash is appropriate, matching e.g. GitHub PAT storage)
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ
);

CREATE TABLE pairing_codes (
    code TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    expires_at TIMESTAMPTZ NOT NULL,    -- 10-minute TTL
    used_at TIMESTAMPTZ                 -- single-use: set on first successful join
);

CREATE TABLE archive_transcript_lines (
    seq BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    device_id UUID NOT NULL REFERENCES devices(id),
    project_slug TEXT NOT NULL,
    session_id TEXT NOT NULL,
    jsonl_path TEXT NOT NULL,
    line_no BIGINT NOT NULL,
    raw_line TEXT NOT NULL,
    ingested_at BIGINT NOT NULL,
    UNIQUE (device_id, jsonl_path, line_no)
);

CREATE TABLE archive_file_snapshots (
    seq BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    device_id UUID NOT NULL REFERENCES devices(id),
    source_path TEXT NOT NULL,
    kind TEXT NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    captured_at BIGINT NOT NULL,
    UNIQUE (device_id, source_path, content_hash)
);
```
`seq` is a single global auto-increment per table — pull queries are always `WHERE user_id = $1 AND device_id != $2 AND seq > $3`, so no per-device cursor bookkeeping is needed on the backend.

## 5. Sync protocol & API contract

**Bootstrapping a user + first device:**
```
POST /v1/accounts  { device_name }
  → { user_id, device_id, api_key }
```

**Pairing a second device:**
```
POST /v1/devices/pair-code            (authenticated as an existing device)
  → { pairing_code }                  (10-minute TTL, single-use)

POST /v1/devices/join  { pairing_code, device_name }   (unauthenticated — this is how a new device obtains its key)
  → { user_id, device_id, api_key }
```

**Ongoing sync** (authenticated via `Authorization: Bearer <api_key>`):
```
POST /v1/archive/push
  { transcript_lines: [StoredTranscriptLine...], file_snapshots: [StoredFileSnapshot...] }
  → { accepted: <count> }

GET /v1/archive/pull?since_transcript_seq=N&since_snapshot_seq=M&limit=L
  → { transcript_lines: [...], file_snapshots: [...],
      transcript_seq_high_water, snapshot_seq_high_water }
```
`pull` excludes the caller's own `device_id` server-side. Both endpoints are idempotent: the backend enforces the same `(device_id, key-columns)` uniqueness as the client (`ON CONFLICT DO NOTHING`), so a retried push after a dropped connection is a safe no-op rather than a duplicate.

All request/response bodies are defined once as serde types in `archive-sync-types`, imported by both `src-tauri` and `sync-backend`.

## 6. Client sync engine & error handling

A new `sync` module in `src-tauri/src/sync/`, structured like the existing usage poller (`poll_loop.rs`):

- **Disabled by default.** Does nothing until the user pairs a device via Settings (enter or generate a pairing code).
- **Each cycle:** push new local rows since the last-pushed watermark in batches of 500 rows (looping until caught up), then pull remote rows since the last-pulled watermark in pages of 500 (looping on the cursor until a response comes back under the page limit), inserting pulled rows via the existing `Db::insert_transcript_lines`/`Db::insert_file_snapshot` methods — unchanged, since they're already idempotent and now correctly partitioned by `device_id`.
- **Manual "Sync now"** command, so pairing a new device doesn't require waiting for the next timer tick.
- **401 (revoked/invalid key):** stop syncing; surface a "needs re-authentication" status in Settings — never retry-loop against a permanently broken credential.
- **Network unreachable:** log at info level (expected/transient for a personal backend that isn't always up); retry next cycle; never surfaced as an error state.
- **Partial batch failure:** all-or-nothing per batch — the watermark only advances past a batch that fully succeeded, so a retry naturally resends it rather than skipping rows.

## 7. Testing

- Client-side: watermark advancement, batching, and idempotent pull-insert logic tested against a mocked HTTP layer (`mockito`, already a dev-dependency in this repo) — no real network.
- Backend-side: push/pull/pagination/idempotency tested against a real test Postgres instance.
- Any manual end-to-end (two-device) verification in the eventual implementation plan runs against an isolated test backend and test data only — **never the real production database**, directly incorporating the lesson from the phase-1 manual-verification incident.

## 8. Relationship to phase 1

This phase is purely additive to phase 1's schema (one migration adding `device_id` to both tables) and purely additive to phase 1's architecture (a new `sync` module alongside `archive_watcher`, no changes to how local archiving itself works). Phase 1's local-only archive continues to function identically whether or not sync is ever enabled.
