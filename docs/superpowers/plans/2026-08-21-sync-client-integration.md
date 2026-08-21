# Sync Client Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the existing claude-switchboard desktop app up to the already-built `sync-backend` service — a local schema migration adding `device_id`, a Rust `SyncClient` + periodic sync engine, and a minimal Settings UI for pairing devices and triggering/observing sync.

**Architecture:** A new `sync` module in `src-tauri/src/` (client, engine) mirrors the existing `usage_api`/`poll_loop` pattern already in this codebase: a typed HTTP client wrapping the shared `Arc<reqwest::Client>`, a periodic background task, and Tauri commands exposing pairing/status/manual-sync to a small new Settings panel section. The local schema gains a `device_id` column on both archive tables; `Db::insert_transcript_lines`/`insert_file_snapshot` stamp it automatically (no call-site changes needed for phase 1's existing three call sites) via a new `Db::device_id()` get-or-create accessor.

**Tech Stack:** Rust (reqwest, uuid — new dependency), the already-built `archive-sync-types` shared crate, React/TypeScript (existing Settings panel conventions).

**Spec:** `docs/superpowers/specs/2026-08-21-cross-machine-archive-sync-design.md` — read it alongside this plan. Also read `docs/superpowers/plans/2026-08-21-sync-backend.md` for the actual, already-built API contract this plan's client code talks to (endpoints, request/response shapes, auth) — that plan is the ground truth for what the server does, not just the spec's prose description.

## Global Constraints

- Sync is opt-in and disabled by default — none of this code runs until the user pairs a device via Settings.
- `Db::insert_transcript_lines`/`insert_file_snapshot`'s existing public signatures do not change — device_id is stamped internally via `Db::device_id()`, so phase 1's three existing call sites (`jsonl_parser/walker.rs` x2, `archive_watcher.rs` x1) need zero changes.
- `transcript_lines` keeps its existing `UNIQUE(jsonl_path, line_no)` constraint unchanged — `jsonl_path` already embeds Claude Code's own globally-unique session UUID, so it's already correct across devices; only a plain `ADD COLUMN` is needed (no SQLite table rebuild). `file_snapshots`' `source_path` is a literal absolute path that CAN collide across two different machines (e.g. both have `/Users/x/.claude/settings.json` as unrelated files) — its `UNIQUE` constraint must become `(device_id, source_path, content_hash)`, which SQLite can only do via a table rebuild (create new, copy, drop, rename).
- The API key is stored as a `settings` row (same pattern as `migration_completed`/`repriced_pricing_version`), not a separate credentials file — matches this codebase's already-established rationale (§2.5.1 of the original design doc: disk encryption + file permissions are the security boundary, not app-level secret storage, given the small blast radius of a single revocable key).
- Any manual/integration verification in this plan's tests runs against the real `sync-backend` service pointed at an isolated test Postgres — never a real production backend. (Lesson from an earlier, unrelated feature's incident.)
- Every new Tauri command is registered in **both** `collect_commands!` blocks in `lib.rs` (release and debug) — every existing command already follows this; missing one means the command silently doesn't exist in one build variant.

---

### Task 1: Schema migration (v14: `device_id`) + `Db::device_id()`

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `uuid` dependency)
- Create: `src-tauri/src/store/migrations/0014_sync_device_id.sql`
- Modify: `src-tauri/src/store/schema.sql` (mirror the same schema for fresh installs)
- Modify: `src-tauri/src/store/mod.rs` (wire migration, bump version stamps, backfill step)
- Modify: `src-tauri/src/store/queries.rs` (add `device_id()`, stamp it in the two insert methods)

**Interfaces:**
- Produces: `pub fn device_id(&self) -> Result<String>` (get-or-create, idempotent) — consumed by Task 2/3's sync client/engine to identify this install, and internally by `insert_transcript_lines`/`insert_file_snapshot`.

- [ ] **Step 1: Add the `uuid` dependency**

In `src-tauri/Cargo.toml`, add to `[dependencies]` (alongside the existing `sha2`/`rand` lines):
```toml
uuid = { version = "1", features = ["v4"] }
```

- [ ] **Step 2: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src-tauri/src/store/queries.rs`:
```rust
    #[test]
    fn device_id_is_generated_once_and_stable_across_calls() {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        let id1 = db.device_id().unwrap();
        let id2 = db.device_id().unwrap();
        assert_eq!(id1, id2, "device_id must be stable across calls, not regenerated");
        assert!(uuid::Uuid::parse_str(&id1).is_ok(), "device_id must be a valid UUID string");
    }

    #[test]
    fn insert_transcript_lines_and_file_snapshot_stamp_this_devices_id() {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        let my_id = db.device_id().unwrap();

        db.insert_transcript_lines(&[StoredTranscriptLine {
            project_slug: "p".into(),
            session_id: "s".into(),
            jsonl_path: "p/s.jsonl".into(),
            line_no: 0,
            raw_line: "{}".into(),
        }])
        .unwrap();
        let device_id: String = db
            .conn()
            .query_row("SELECT device_id FROM transcript_lines WHERE jsonl_path = 'p/s.jsonl'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(device_id, my_id);

        db.insert_file_snapshot(&StoredFileSnapshot {
            source_path: "/x".into(),
            kind: "misc".into(),
            content: "x".into(),
            content_hash: "h".into(),
        })
        .unwrap();
        let device_id: String = db
            .conn()
            .query_row("SELECT device_id FROM file_snapshots WHERE source_path = '/x'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(device_id, my_id);
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib device_id_is_generated_once insert_transcript_lines_and_file_snapshot_stamp`
Expected: FAIL — `device_id` method and `device_id` column don't exist yet.

- [ ] **Step 4: Create the migration**

Create `src-tauri/src/store/migrations/0014_sync_device_id.sql`:
```sql
-- v13 -> v14: add device_id so locally-recorded rows carry provenance, and
-- so pulled-in rows from other devices (phase 2 sync) can be distinguished
-- from this install's own. transcript_lines' existing UNIQUE(jsonl_path,
-- line_no) already disambiguates correctly across devices (jsonl_path
-- embeds Claude Code's own globally-unique session UUID), so it's a plain
-- ADD COLUMN. file_snapshots' source_path is a literal absolute path that
-- CAN collide across two different machines (e.g. both have
-- "/Users/x/.claude/settings.json" as different, unrelated files) — its
-- UNIQUE constraint must include device_id, which SQLite can only do via a
-- table rebuild (no ALTER TABLE ... ADD CONSTRAINT in SQLite).
ALTER TABLE transcript_lines ADD COLUMN device_id TEXT NOT NULL DEFAULT '';

CREATE TABLE file_snapshots_v14 (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id    TEXT NOT NULL DEFAULT '',
    source_path  TEXT NOT NULL,
    kind         TEXT NOT NULL,
    content      TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    captured_at  INTEGER NOT NULL,
    UNIQUE (device_id, source_path, content_hash)
);
INSERT INTO file_snapshots_v14 (id, source_path, kind, content, content_hash, captured_at)
    SELECT id, source_path, kind, content, content_hash, captured_at FROM file_snapshots;
DROP TABLE file_snapshots;
ALTER TABLE file_snapshots_v14 RENAME TO file_snapshots;
CREATE INDEX IF NOT EXISTS idx_file_snapshots_path ON file_snapshots(source_path);
```

- [ ] **Step 5: Mirror the same shape into `schema.sql` for fresh installs**

In `src-tauri/src/store/schema.sql`, change the existing `transcript_lines` table definition to include the new column (add `device_id TEXT NOT NULL DEFAULT '',` as a field, placed after `id`), and change the existing `file_snapshots` table definition to include `device_id TEXT NOT NULL DEFAULT '',` as a field and change its `UNIQUE (source_path, content_hash)` to `UNIQUE (device_id, source_path, content_hash)`.

- [ ] **Step 6: Wire the migration into `migrate()`, bump version stamps, backfill device_id**

In `src-tauri/src/store/mod.rs`'s `migrate()`, after the existing `if current < 13 { ... }` block, add:
```rust
        if current < 14 {
            tracing::info!("migrating v13 -> v14 (device_id for sync)");
            conn.execute_batch(include_str!("migrations/0014_sync_device_id.sql"))
                .context("apply migration 0014")?;
        }
```

Change the final stamp in `migrate()` from `[13_i64]` to `[14_i64]`. Change `create_fresh_db`'s stamp from `[13_i64]` to `[14_i64]` and update its doc comment.

Update the two existing version-stamp tests (`fresh_database_is_stamped_at_version_11` and any other asserting `version == 13`) to assert `14` instead — grep the test module for `13` in a `schema_version`/`MAX(version)` context to find them precisely.

Add a new test mirroring the existing per-migration pattern:
```rust
    #[test]
    fn migration_0014_adds_device_id_and_rebuilds_file_snapshots() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("v13.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("schema.sql")).unwrap();
        // Simulate a pre-v14 shape: drop device_id from both tables by
        // rebuilding them without it, matching what a real v13 DB looked like.
        conn.execute_batch(
            "DROP TABLE transcript_lines;
             CREATE TABLE transcript_lines (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 project_slug TEXT NOT NULL, session_id TEXT NOT NULL,
                 jsonl_path TEXT NOT NULL, line_no INTEGER NOT NULL,
                 raw_line TEXT NOT NULL, ingested_at INTEGER NOT NULL,
                 UNIQUE (jsonl_path, line_no)
             );
             DROP TABLE file_snapshots;
             CREATE TABLE file_snapshots (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 source_path TEXT NOT NULL, kind TEXT NOT NULL,
                 content TEXT NOT NULL, content_hash TEXT NOT NULL,
                 captured_at INTEGER NOT NULL,
                 UNIQUE (source_path, content_hash)
             );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_snapshots (source_path, kind, content, content_hash, captured_at)
             VALUES ('/x', 'misc', 'hi', 'h1', 0)",
            [],
        )
        .unwrap();

        conn.execute_batch(include_str!("migrations/0014_sync_device_id.sql")).unwrap();

        let device_id: String = conn
            .query_row("SELECT device_id FROM file_snapshots WHERE source_path = '/x'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(device_id, "", "pre-existing rows default to empty device_id until backfilled");

        // Prove the new UNIQUE constraint actually includes device_id: two
        // rows with the same (source_path, content_hash) but different
        // device_id must both be insertable.
        conn.execute(
            "INSERT INTO file_snapshots (device_id, source_path, kind, content, content_hash, captured_at)
             VALUES ('device-a', '/y', 'misc', 'hi', 'h2', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO file_snapshots (device_id, source_path, kind, content, content_hash, captured_at)
             VALUES ('device-b', '/y', 'misc', 'hi', 'h2', 0)",
            [],
        )
        .expect("same (source_path, content_hash) from a DIFFERENT device_id must not collide");
    }
```

- [ ] **Step 7: Add `Db::device_id()` and stamp it in the insert methods**

In `src-tauri/src/store/queries.rs`, add near `repriced_version`/`set_repriced_version`:
```rust
    /// This install's stable sync identity — a UUID generated once on
    /// first call and persisted as a settings row, never regenerated
    /// afterward. Independent of whether sync is ever enabled; phase 1's
    /// local archive already needs a stable per-install identity so that
    /// rows synced in later from other devices can be told apart from
    /// this device's own.
    pub fn device_id(&self) -> Result<String> {
        let existing: Option<String> = self
            .conn()
            .query_row(
                "SELECT value FROM settings WHERE key = 'sync_device_id'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.conn().execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES ('sync_device_id', ?1)",
            params![id],
        )?;
        // Re-read rather than trust the just-generated value: a concurrent
        // caller could have won the INSERT OR IGNORE race, in which case
        // the stable, already-persisted id is whichever came first.
        let id: String = self.conn().query_row(
            "SELECT value FROM settings WHERE key = 'sync_device_id'",
            [],
            |r| r.get(0),
        )?;
        Ok(id)
    }
```

Change `insert_transcript_lines`'s INSERT statement from:
```rust
                "INSERT OR REPLACE INTO transcript_lines
                 (project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
```
to:
```rust
                "INSERT OR REPLACE INTO transcript_lines
                 (device_id, project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
```
and bind `device_id` as the first parameter (fetch it once via `let device_id = self.device_id()?;` before the loop, reusing the same value for every row in the batch rather than calling `device_id()` per-row):
```rust
        let device_id = self.device_id()?;
        let now = Utc::now().timestamp();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let inserted = {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO transcript_lines
                 (device_id, project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let mut n = 0;
            for l in lines {
                n += stmt.execute(params![
                    device_id,
                    l.project_slug,
                    l.session_id,
                    l.jsonl_path,
                    l.line_no,
                    l.raw_line,
                    now,
                ])?;
            }
            n
        };
        tx.commit()?;
        Ok(inserted)
```
(Note: calling `self.device_id()?` before `self.conn()` avoids a self-deadlock, since both would otherwise try to lock the same `Mutex<Connection>` — `device_id()` must fully complete and drop its own lock guard before `insert_transcript_lines` takes its own.)

Similarly change `insert_file_snapshot`'s INSERT to include `device_id` as a bound column:
```rust
    pub fn insert_file_snapshot(&self, snap: &StoredFileSnapshot) -> Result<bool> {
        let device_id = self.device_id()?;
        let now = Utc::now().timestamp();
        let changed = self.conn().execute(
            "INSERT OR IGNORE INTO file_snapshots
             (device_id, source_path, kind, content, content_hash, captured_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![device_id, snap.source_path, snap.kind, snap.content, snap.content_hash, now],
        )?;
        Ok(changed > 0)
    }
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib device_id_is_generated_once insert_transcript_lines_and_file_snapshot_stamp migration_0014`
Expected: PASS

- [ ] **Step 9: Run the full test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS (all existing tests + new ones; the one known pre-existing unrelated failure in `commands::tests::warmup_suggestion_takes_only_the_earliest_event_per_day` is expected and out of scope)

- [ ] **Step 10: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/store/schema.sql src-tauri/src/store/migrations/0014_sync_device_id.sql src-tauri/src/store/mod.rs src-tauri/src/store/queries.rs
git commit -m "feat: add device_id to archive tables and Db::device_id()"
```

---

### Task 2: `SyncClient` — Rust HTTP client for the sync-backend API

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `archive-sync-types` path dependency)
- Create: `src-tauri/src/sync/mod.rs`
- Create: `src-tauri/src/sync/client.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod sync;` declaration)

**Interfaces:**
- Consumes: `archive_sync_types::{CreateAccountRequest, CreateAccountResponse, PairCodeResponse, JoinRequest, JoinResponse, PushRequest, PushResponse, PullResponse}` (already built, `docs/superpowers/plans/2026-08-21-sync-backend.md` Task 1).
- Produces: `pub struct SyncClient`, `pub enum SyncOutcome<T> { Ok(T), Unauthorized, Transient(String) }`, methods `create_account`, `pair_code`, `join`, `push`, `pull` — consumed by Task 3's sync engine.

- [ ] **Step 1: Add the `archive-sync-types` dependency**

In `src-tauri/Cargo.toml`, add to `[dependencies]`:
```toml
archive-sync-types = { path = "../archive-sync-types" }
```

- [ ] **Step 2: Write the failing test**

Create `src-tauri/src/sync/client.rs` with the test module first (the implementation stubs come in Step 4):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn test_client(base_url: &str) -> SyncClient {
        let inner = reqwest::Client::builder().build().unwrap();
        SyncClient::new(Arc::new(inner), base_url.to_string())
    }

    #[tokio::test]
    async fn create_account_returns_ok_on_200() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/accounts")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"user_id":"u1","device_id":"d1","api_key":"k1"}"#)
            .create_async()
            .await;
        let client = test_client(&server.url());
        let result = client
            .create_account("d1".to_string(), "Test Device".to_string())
            .await;
        mock.assert_async().await;
        match result {
            SyncOutcome::Ok(resp) => {
                assert_eq!(resp.user_id, "u1");
                assert_eq!(resp.api_key, "k1");
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn push_returns_unauthorized_on_401() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("POST", "/v1/archive/push")
            .with_status(401)
            .create_async()
            .await;
        let client = test_client(&server.url());
        let result = client
            .push("bad-key", archive_sync_types::PushRequest { transcript_lines: vec![], file_snapshots: vec![] })
            .await;
        assert!(matches!(result, SyncOutcome::Unauthorized));
    }

    #[tokio::test]
    async fn pull_returns_transient_on_500() {
        let mut server = mockito::Server::new_async().await;
        server
            .mock("GET", mockito::Matcher::Regex(r"^/v1/archive/pull".to_string()))
            .with_status(500)
            .create_async()
            .await;
        let client = test_client(&server.url());
        let result = client.pull("some-key", 0, 0, 500).await;
        assert!(matches!(result, SyncOutcome::Transient(_)));
    }
}
```

Add `#[derive(Debug)]` requirement note: `SyncOutcome<T>` needs `Debug` for the `panic!("... {other:?}")` in the test above — make sure to derive it (see Step 4).

- [ ] **Step 3: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib sync::client`
Expected: FAIL to compile — `SyncClient`/`SyncOutcome` don't exist yet.

- [ ] **Step 4: Implement `SyncClient`**

At the top of `src-tauri/src/sync/client.rs` (above the test module), add:
```rust
use archive_sync_types::{
    CreateAccountRequest, CreateAccountResponse, JoinRequest, JoinResponse, PairCodeResponse,
    PullResponse, PushRequest, PushResponse,
};
use reqwest::{Client, StatusCode};
use std::sync::Arc;

/// Mirrors `usage_api::FetchOutcome`'s shape: a typed result that
/// distinguishes "the call worked," "the API key is no longer valid"
/// (stop retrying, surface a re-auth prompt), and "something transient
/// went wrong" (network blip, 5xx — safe to retry next cycle).
#[derive(Debug)]
pub enum SyncOutcome<T> {
    Ok(T),
    Unauthorized,
    Transient(String),
}

pub struct SyncClient {
    inner: Arc<Client>,
    base_url: String,
}

impl SyncClient {
    pub fn new(inner: Arc<Client>, base_url: String) -> Self {
        Self { inner, base_url }
    }

    pub async fn create_account(&self, device_id: String, device_name: String) -> SyncOutcome<CreateAccountResponse> {
        let url = format!("{}/v1/accounts", self.base_url);
        let req = CreateAccountRequest { device_id, device_name };
        match self.inner.post(&url).json(&req).send().await {
            Ok(resp) if resp.status().is_success() => match resp.json::<CreateAccountResponse>().await {
                Ok(body) => SyncOutcome::Ok(body),
                Err(e) => SyncOutcome::Transient(e.to_string()),
            },
            Ok(resp) => SyncOutcome::Transient(format!("unexpected status {}", resp.status())),
            Err(e) => SyncOutcome::Transient(e.to_string()),
        }
    }

    pub async fn pair_code(&self, api_key: &str) -> SyncOutcome<PairCodeResponse> {
        let url = format!("{}/v1/devices/pair-code", self.base_url);
        match self.inner.post(&url).bearer_auth(api_key).send().await {
            Ok(resp) if resp.status() == StatusCode::UNAUTHORIZED => SyncOutcome::Unauthorized,
            Ok(resp) if resp.status().is_success() => match resp.json::<PairCodeResponse>().await {
                Ok(body) => SyncOutcome::Ok(body),
                Err(e) => SyncOutcome::Transient(e.to_string()),
            },
            Ok(resp) => SyncOutcome::Transient(format!("unexpected status {}", resp.status())),
            Err(e) => SyncOutcome::Transient(e.to_string()),
        }
    }

    pub async fn join(&self, pairing_code: String, device_id: String, device_name: String) -> SyncOutcome<JoinResponse> {
        let url = format!("{}/v1/devices/join", self.base_url);
        let req = JoinRequest { pairing_code, device_id, device_name };
        match self.inner.post(&url).json(&req).send().await {
            Ok(resp) if resp.status() == StatusCode::UNAUTHORIZED => SyncOutcome::Unauthorized,
            Ok(resp) if resp.status().is_success() => match resp.json::<JoinResponse>().await {
                Ok(body) => SyncOutcome::Ok(body),
                Err(e) => SyncOutcome::Transient(e.to_string()),
            },
            Ok(resp) => SyncOutcome::Transient(format!("unexpected status {}", resp.status())),
            Err(e) => SyncOutcome::Transient(e.to_string()),
        }
    }

    pub async fn push(&self, api_key: &str, req: PushRequest) -> SyncOutcome<PushResponse> {
        let url = format!("{}/v1/archive/push", self.base_url);
        match self.inner.post(&url).bearer_auth(api_key).json(&req).send().await {
            Ok(resp) if resp.status() == StatusCode::UNAUTHORIZED => SyncOutcome::Unauthorized,
            Ok(resp) if resp.status().is_success() => match resp.json::<PushResponse>().await {
                Ok(body) => SyncOutcome::Ok(body),
                Err(e) => SyncOutcome::Transient(e.to_string()),
            },
            Ok(resp) => SyncOutcome::Transient(format!("unexpected status {}", resp.status())),
            Err(e) => SyncOutcome::Transient(e.to_string()),
        }
    }

    pub async fn pull(&self, api_key: &str, since_transcript_seq: i64, since_snapshot_seq: i64, limit: i64) -> SyncOutcome<PullResponse> {
        let url = format!(
            "{}/v1/archive/pull?since_transcript_seq={since_transcript_seq}&since_snapshot_seq={since_snapshot_seq}&limit={limit}",
            self.base_url
        );
        match self.inner.get(&url).bearer_auth(api_key).send().await {
            Ok(resp) if resp.status() == StatusCode::UNAUTHORIZED => SyncOutcome::Unauthorized,
            Ok(resp) if resp.status().is_success() => match resp.json::<PullResponse>().await {
                Ok(body) => SyncOutcome::Ok(body),
                Err(e) => SyncOutcome::Transient(e.to_string()),
            },
            Ok(resp) => SyncOutcome::Transient(format!("unexpected status {}", resp.status())),
            Err(e) => SyncOutcome::Transient(e.to_string()),
        }
    }
}
```

Create `src-tauri/src/sync/mod.rs`:
```rust
pub mod client;
pub use client::{SyncClient, SyncOutcome};
```

Add `mockito` — already a dev-dependency in `src-tauri/Cargo.toml` (used elsewhere, e.g. `usage_api_client.rs` tests) — no change needed there.

Add `mod sync;` to `src-tauri/src/lib.rs`'s module declarations, alphabetically after `pub mod store;` and before `mod tray;`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib sync::client`
Expected: PASS

- [ ] **Step 6: Run the full test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/sync src-tauri/src/lib.rs
git commit -m "feat: add SyncClient for the sync-backend API"
```

---

### Task 3: Sync engine — periodic push/pull loop + watermarks

**Files:**
- Create: `src-tauri/src/sync/engine.rs`
- Modify: `src-tauri/src/sync/mod.rs` (export the new module)
- Modify: `src-tauri/src/store/queries.rs` (watermark get/set helpers)

**Interfaces:**
- Consumes: `SyncClient` (Task 2), `Db::device_id()` (Task 1).
- Produces: `pub async fn run_sync_cycle(db: &Db, client: &SyncClient) -> SyncCycleResult` — consumed by Task 4's periodic spawn + manual "sync now" command.

- [ ] **Step 1: Add watermark storage to `queries.rs`**

Add near `device_id()`:
```rust
    /// Highest local `transcript_lines.id`/`file_snapshots.id` already
    /// pushed to the backend, per table. `None` means nothing pushed yet
    /// (start from the beginning of this device's own rows).
    pub fn sync_push_watermark(&self, table: &str) -> Result<Option<i64>> {
        let key = format!("sync_push_watermark_{table}");
        let v: Option<String> = self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?;
        Ok(v.and_then(|s| s.parse().ok()))
    }

    pub fn set_sync_push_watermark(&self, table: &str, id: i64) -> Result<()> {
        let key = format!("sync_push_watermark_{table}");
        self.conn().execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, id.to_string()],
        )?;
        Ok(())
    }

    /// Highest remote `seq` already pulled from the backend, per table.
    pub fn sync_pull_watermark(&self, table: &str) -> Result<i64> {
        let key = format!("sync_pull_watermark_{table}");
        let v: Option<String> = self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?;
        Ok(v.and_then(|s| s.parse().ok()).unwrap_or(0))
    }

    pub fn set_sync_pull_watermark(&self, table: &str, seq: i64) -> Result<()> {
        let key = format!("sync_pull_watermark_{table}");
        self.conn().execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, seq.to_string()],
        )?;
        Ok(())
    }

    /// This device's own rows created after `since_id` (exclusive),
    /// oldest first, capped at `limit` — used by the sync engine to find
    /// what to push next. Only returns rows tagged with THIS device's own
    /// `device_id` (never rows already pulled in from another device —
    /// re-pushing those would be pointless and would misattribute them).
    pub fn local_transcript_lines_since(&self, since_id: i64, limit: i64) -> Result<Vec<(i64, StoredTranscriptLine)>> {
        let my_id = self.device_id()?;
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, project_slug, session_id, jsonl_path, line_no, raw_line
             FROM transcript_lines WHERE device_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![my_id, since_id, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                StoredTranscriptLine {
                    project_slug: r.get(1)?,
                    session_id: r.get(2)?,
                    jsonl_path: r.get(3)?,
                    line_no: r.get(4)?,
                    raw_line: r.get(5)?,
                },
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Same idea as `local_transcript_lines_since`, for file_snapshots.
    pub fn local_file_snapshots_since(&self, since_id: i64, limit: i64) -> Result<Vec<(i64, StoredFileSnapshot)>> {
        let my_id = self.device_id()?;
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, source_path, kind, content, content_hash
             FROM file_snapshots WHERE device_id = ?1 AND id > ?2 ORDER BY id LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![my_id, since_id, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                StoredFileSnapshot {
                    source_path: r.get(1)?,
                    kind: r.get(2)?,
                    content: r.get(3)?,
                    content_hash: r.get(4)?,
                },
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
```

Write a focused test proving `local_transcript_lines_since` excludes rows tagged with a DIFFERENT device_id (i.e. rows already pulled in from another device don't get re-pushed):
```rust
    #[test]
    fn local_transcript_lines_since_excludes_other_devices_rows() {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        let my_id = db.device_id().unwrap();

        db.insert_transcript_lines(&[StoredTranscriptLine {
            project_slug: "p".into(), session_id: "mine".into(),
            jsonl_path: "p/mine.jsonl".into(), line_no: 0, raw_line: "{}".into(),
        }]).unwrap();

        // Simulate a row pulled in from another device: same table, but a
        // different device_id, inserted directly (bypassing insert_transcript_lines,
        // which always stamps THIS device's own id).
        db.conn().execute(
            "INSERT INTO transcript_lines (device_id, project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)
             VALUES ('other-device', 'p', 'theirs', 'p/theirs.jsonl', 0, '{}', 0)",
            [],
        ).unwrap();

        let rows = db.local_transcript_lines_since(0, 500).unwrap();
        assert_eq!(rows.len(), 1, "only this device's own row should be returned for pushing");
        assert_eq!(rows[0].1.session_id, "mine");
        let _ = my_id;
    }
```

Run: `cd src-tauri && cargo test --lib local_transcript_lines_since_excludes`
Expected: FAIL first (methods don't exist), then implement as above, then PASS.

- [ ] **Step 2: Write the sync engine**

Create `src-tauri/src/sync/engine.rs`:
```rust
use crate::store::{Db, StoredFileSnapshot, StoredTranscriptLine};
use crate::sync::{SyncClient, SyncOutcome};
use archive_sync_types::{PushRequest, SyncFileSnapshot, SyncTranscriptLine};

const PUSH_BATCH_SIZE: i64 = 500;
const PULL_PAGE_SIZE: i64 = 500;

#[derive(Debug, PartialEq)]
pub enum SyncCycleResult {
    Ok { pushed: usize, pulled: usize },
    Unauthorized,
    Transient(String),
}

/// Turns a cycle result into the summary shape both `sync_now` and the
/// periodic background task display — one definition so the two call
/// sites can never drift on how a result maps to a status string.
pub fn summarize_cycle_result(result: SyncCycleResult, at: chrono::DateTime<chrono::Utc>) -> crate::app_state::SyncCycleSummary {
    let (outcome, pushed, pulled) = match result {
        SyncCycleResult::Ok { pushed, pulled } => ("ok", pushed as u32, pulled as u32),
        SyncCycleResult::Unauthorized => ("unauthorized", 0, 0),
        SyncCycleResult::Transient(_) => ("transient", 0, 0),
    };
    crate::app_state::SyncCycleSummary {
        last_run_at: at.to_rfc3339(),
        outcome: outcome.to_string(),
        pushed,
        pulled,
    }
}

/// Runs one full push-then-pull cycle: pushes every not-yet-pushed local
/// row (in batches, advancing the push watermark only after each batch
/// fully succeeds — a partial batch failure must not skip rows), then
/// pulls every not-yet-seen remote row (paginating on the cursor until a
/// page comes back under the limit) and inserts it via the existing,
/// idempotent `Db::insert_transcript_lines`/`insert_file_snapshot`.
pub async fn run_sync_cycle(db: &Db, client: &SyncClient, api_key: &str) -> SyncCycleResult {
    let mut pushed = 0usize;
    loop {
        let since = db.sync_push_watermark("transcript_lines").unwrap_or(None).unwrap_or(0);
        let lines = match db.local_transcript_lines_since(since, PUSH_BATCH_SIZE) {
            Ok(l) => l,
            Err(e) => return SyncCycleResult::Transient(e.to_string()),
        };
        let since_snap = db.sync_push_watermark("file_snapshots").unwrap_or(None).unwrap_or(0);
        let snaps = match db.local_file_snapshots_since(since_snap, PUSH_BATCH_SIZE) {
            Ok(s) => s,
            Err(e) => return SyncCycleResult::Transient(e.to_string()),
        };
        if lines.is_empty() && snaps.is_empty() {
            break;
        }

        let max_line_id = lines.last().map(|(id, _)| *id);
        let max_snap_id = snaps.last().map(|(id, _)| *id);

        let req = PushRequest {
            transcript_lines: lines
                .into_iter()
                .map(|(_, l)| SyncTranscriptLine {
                    project_slug: l.project_slug,
                    session_id: l.session_id,
                    jsonl_path: l.jsonl_path,
                    line_no: l.line_no,
                    raw_line: l.raw_line,
                    ingested_at: 0,
                })
                .collect(),
            file_snapshots: snaps
                .into_iter()
                .map(|(_, s)| SyncFileSnapshot {
                    source_path: s.source_path,
                    kind: s.kind,
                    content: s.content,
                    content_hash: s.content_hash,
                    captured_at: 0,
                })
                .collect(),
        };

        match client.push(api_key, req).await {
            SyncOutcome::Ok(resp) => {
                pushed += resp.transcript_lines_accepted + resp.file_snapshots_accepted;
                // Watermark advances by "rows attempted," not "rows
                // accepted" — a row already accepted in a prior, partially
                // successful run must not be resent forever just because
                // the server correctly no-op'd it as a duplicate.
                if let Some(id) = max_line_id {
                    let _ = db.set_sync_push_watermark("transcript_lines", id);
                }
                if let Some(id) = max_snap_id {
                    let _ = db.set_sync_push_watermark("file_snapshots", id);
                }
            }
            SyncOutcome::Unauthorized => return SyncCycleResult::Unauthorized,
            SyncOutcome::Transient(e) => return SyncCycleResult::Transient(e),
        }
    }

    let mut pulled = 0usize;
    loop {
        let since_t = db.sync_pull_watermark("transcript_lines").unwrap_or(0);
        let since_s = db.sync_pull_watermark("file_snapshots").unwrap_or(0);
        match client.pull(api_key, since_t, since_s, PULL_PAGE_SIZE).await {
            SyncOutcome::Ok(resp) => {
                let got_lines = resp.transcript_lines.len();
                let got_snaps = resp.file_snapshots.len();
                pulled += got_lines + got_snaps;

                let to_insert: Vec<StoredTranscriptLine> = resp
                    .transcript_lines
                    .into_iter()
                    .map(|l| StoredTranscriptLine {
                        project_slug: l.project_slug,
                        session_id: l.session_id,
                        jsonl_path: l.jsonl_path,
                        line_no: l.line_no,
                        raw_line: l.raw_line,
                    })
                    .collect();
                if let Err(e) = db.insert_transcript_lines(&to_insert) {
                    return SyncCycleResult::Transient(e.to_string());
                }
                for snap in resp.file_snapshots {
                    if let Err(e) = db.insert_file_snapshot(&StoredFileSnapshot {
                        source_path: snap.source_path,
                        kind: snap.kind,
                        content: snap.content,
                        content_hash: snap.content_hash,
                    }) {
                        return SyncCycleResult::Transient(e.to_string());
                    }
                }

                let _ = db.set_sync_pull_watermark("transcript_lines", resp.transcript_seq_high_water);
                let _ = db.set_sync_pull_watermark("file_snapshots", resp.snapshot_seq_high_water);

                if got_lines < PULL_PAGE_SIZE as usize && got_snaps < PULL_PAGE_SIZE as usize {
                    break;
                }
            }
            SyncOutcome::Unauthorized => return SyncCycleResult::Unauthorized,
            SyncOutcome::Transient(e) => return SyncCycleResult::Transient(e),
        }
    }

    SyncCycleResult::Ok { pushed, pulled }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Db, StoredAccount};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn test_db() -> (tempfile::TempDir, Db) {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        db.upsert_account(&StoredAccount { id: "a".into(), email: "e".into(), display_name: None }).unwrap();
        (dir, db)
    }

    fn test_client(base_url: &str) -> SyncClient {
        SyncClient::new(Arc::new(reqwest::Client::new()), base_url.to_string())
    }

    #[tokio::test]
    async fn cycle_pushes_new_local_rows_and_advances_watermark() {
        let (_dir, db) = test_db();
        db.insert_transcript_lines(&[StoredTranscriptLine {
            project_slug: "p".into(), session_id: "s".into(),
            jsonl_path: "p/s.jsonl".into(), line_no: 0, raw_line: "{}".into(),
        }]).unwrap();

        let mut server = mockito::Server::new_async().await;
        let push_mock = server
            .mock("POST", "/v1/archive/push")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"transcript_lines_accepted":1,"file_snapshots_accepted":0}"#)
            .create_async()
            .await;
        let pull_mock = server
            .mock("GET", mockito::Matcher::Regex(r"^/v1/archive/pull".to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"transcript_lines":[],"file_snapshots":[],"transcript_seq_high_water":0,"snapshot_seq_high_water":0}"#)
            .create_async()
            .await;

        let client = test_client(&server.url());
        let result = run_sync_cycle(&db, &client, "test-key").await;

        push_mock.assert_async().await;
        pull_mock.assert_async().await;
        assert_eq!(result, SyncCycleResult::Ok { pushed: 1, pulled: 0 });
        assert_eq!(db.sync_push_watermark("transcript_lines").unwrap(), Some(1));
    }

    #[tokio::test]
    async fn cycle_returns_unauthorized_and_does_not_advance_watermark_on_401() {
        let (_dir, db) = test_db();
        db.insert_transcript_lines(&[StoredTranscriptLine {
            project_slug: "p".into(), session_id: "s".into(),
            jsonl_path: "p/s.jsonl".into(), line_no: 0, raw_line: "{}".into(),
        }]).unwrap();

        let mut server = mockito::Server::new_async().await;
        server.mock("POST", "/v1/archive/push").with_status(401).create_async().await;

        let client = test_client(&server.url());
        let result = run_sync_cycle(&db, &client, "bad-key").await;

        assert_eq!(result, SyncCycleResult::Unauthorized);
        assert_eq!(db.sync_push_watermark("transcript_lines").unwrap(), None, "watermark must not advance on auth failure");
    }
}
```

Add `pub mod engine;` to `src-tauri/src/sync/mod.rs`.

- [ ] **Step 3: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib sync::`
Expected: PASS

- [ ] **Step 4: Run the full test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/sync src-tauri/src/store/queries.rs
git commit -m "feat: add sync engine (push/pull cycle with watermarks)"
```

---

### Task 4: Tauri commands + minimal Settings UI

**Files:**
- Modify: `src-tauri/src/app_state.rs` (add `SyncCycleSummary` type + in-memory `sync_status` field — no `SyncClient` field; see Step 1 for why)
- Modify: `src-tauri/src/commands.rs` (new commands)
- Modify: `src-tauri/src/lib.rs` (register commands in both `collect_commands!` blocks, spawn the periodic loop)
- Create: `src/settings/SyncSettings.tsx`
- Modify: `src/settings/SettingsPanel.tsx` (wire in the new section)
- Modify: `src/lib/ipc.ts` (command wrappers)
- Create: `src/settings/__tests__/SyncSettings.test.tsx`

**Interfaces:**
- Consumes: `run_sync_cycle`, `SyncClient`, `summarize_cycle_result` (Task 3), `Db::device_id()` (Task 1).

- [ ] **Step 1: Add the backend-URL/API-key settings helpers, and sync state to `AppState`**

The backend URL is user-configured (the user's own backend, per the spec — there's no fixed default), so `SyncClient` can't be built once at app startup and stored in `AppState` — it's constructed fresh, cheaply, at each call site from the CURRENT `db.sync_backend_url()` value (a thin wrapper around the shared `Arc<reqwest::Client>` that's already in `AppState.http_client` — no new HTTP connection setup cost per construction).

In `src-tauri/src/store/queries.rs`, add near `device_id()`:
```rust
    pub fn sync_backend_url(&self) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = 'sync_backend_url'", [], |r| r.get(0))
            .optional()?)
    }

    pub fn set_sync_backend_url(&self, url: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('sync_backend_url', ?1)",
            params![url],
        )?;
        Ok(())
    }

    pub fn sync_api_key(&self) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = 'sync_api_key'", [], |r| r.get(0))
            .optional()?)
    }

    pub fn set_sync_api_key(&self, key: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('sync_api_key', ?1)",
            params![key],
        )?;
        Ok(())
    }
```
Add a focused test proving round-trip + `None` default:
```rust
    #[test]
    fn sync_backend_url_and_api_key_round_trip() {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        assert_eq!(db.sync_backend_url().unwrap(), None);
        assert_eq!(db.sync_api_key().unwrap(), None);
        db.set_sync_backend_url("https://example.com").unwrap();
        db.set_sync_api_key("k1").unwrap();
        assert_eq!(db.sync_backend_url().unwrap(), Some("https://example.com".to_string()));
        assert_eq!(db.sync_api_key().unwrap(), Some("k1".to_string()));
    }
```

In `src-tauri/src/app_state.rs`, add near the other fields on `AppState`:
```rust
    /// In-memory only, like `sessions_cache` — starts empty on every
    /// launch. Read by `get_sync_status`; written after each sync cycle
    /// (manual or periodic).
    pub sync_status: RwLock<Option<SyncCycleSummary>>,
```
Add above `AppState`:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SyncCycleSummary {
    pub last_run_at: String,
    pub outcome: String, // "ok" | "unauthorized" | "transient"
    pub pushed: u32,
    pub pulled: u32,
}
```
In `src-tauri/src/lib.rs`'s `AppState` construction, add `sync_status: RwLock::new(None),` alongside the existing fields — no new constructor argument needed, since there's no `SyncClient` to build here.

- [ ] **Step 2: Add the Tauri commands**

In `src-tauri/src/commands.rs`, add:
```rust
#[command]
#[specta::specta]
pub async fn get_sync_status(state: State<'_, Arc<AppState>>) -> Result<Option<crate::app_state::SyncCycleSummary>, String> {
    Ok(state.sync_status.read().clone())
}

#[command]
#[specta::specta]
pub async fn set_sync_backend_url(state: State<'_, Arc<AppState>>, url: String) -> Result<(), String> {
    state.db.set_sync_backend_url(&url).map_err(err_to_string)
}

#[command]
#[specta::specta]
pub async fn get_sync_backend_url(state: State<'_, Arc<AppState>>) -> Result<Option<String>, String> {
    state.db.sync_backend_url().map_err(err_to_string)
}

#[command]
#[specta::specta]
pub async fn bootstrap_sync_account(
    state: State<'_, Arc<AppState>>,
    device_name: String,
) -> Result<(), String> {
    let base_url = state.db.sync_backend_url().map_err(err_to_string)?.ok_or("set a backend URL first")?;
    let client = crate::sync::SyncClient::new(state.http_client.clone(), base_url);
    let device_id = state.db.device_id().map_err(err_to_string)?;
    match client.create_account(device_id, device_name).await {
        crate::sync::SyncOutcome::Ok(resp) => {
            state.db.set_sync_api_key(&resp.api_key).map_err(err_to_string)?;
            Ok(())
        }
        crate::sync::SyncOutcome::Unauthorized => Err("unexpected unauthorized on account creation".into()),
        crate::sync::SyncOutcome::Transient(e) => Err(e),
    }
}

#[command]
#[specta::specta]
pub async fn generate_pairing_code(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    let base_url = state.db.sync_backend_url().map_err(err_to_string)?.ok_or("set a backend URL first")?;
    let api_key = state.db.sync_api_key().map_err(err_to_string)?.ok_or("sync not configured on this device")?;
    let client = crate::sync::SyncClient::new(state.http_client.clone(), base_url);
    match client.pair_code(&api_key).await {
        crate::sync::SyncOutcome::Ok(resp) => Ok(resp.pairing_code),
        crate::sync::SyncOutcome::Unauthorized => Err("this device's sync key is no longer valid".into()),
        crate::sync::SyncOutcome::Transient(e) => Err(e),
    }
}

#[command]
#[specta::specta]
pub async fn join_sync_account(
    state: State<'_, Arc<AppState>>,
    pairing_code: String,
    device_name: String,
) -> Result<(), String> {
    let base_url = state.db.sync_backend_url().map_err(err_to_string)?.ok_or("set a backend URL first")?;
    let client = crate::sync::SyncClient::new(state.http_client.clone(), base_url);
    let device_id = state.db.device_id().map_err(err_to_string)?;
    match client.join(pairing_code, device_id, device_name).await {
        crate::sync::SyncOutcome::Ok(resp) => {
            state.db.set_sync_api_key(&resp.api_key).map_err(err_to_string)?;
            Ok(())
        }
        crate::sync::SyncOutcome::Unauthorized => Err("invalid or expired pairing code".into()),
        crate::sync::SyncOutcome::Transient(e) => Err(e),
    }
}

#[command]
#[specta::specta]
pub async fn sync_now(state: State<'_, Arc<AppState>>) -> Result<crate::app_state::SyncCycleSummary, String> {
    let base_url = state.db.sync_backend_url().map_err(err_to_string)?.ok_or("set a backend URL first")?;
    let api_key = state.db.sync_api_key().map_err(err_to_string)?.ok_or("sync not configured on this device")?;
    let client = crate::sync::SyncClient::new(state.http_client.clone(), base_url);
    let result = crate::sync::engine::run_sync_cycle(&state.db, &client, &api_key).await;
    let summary = crate::sync::engine::summarize_cycle_result(result, chrono::Utc::now());
    *state.sync_status.write() = Some(summary.clone());
    Ok(summary)
}
```

- [ ] **Step 3: Register commands + spawn the periodic loop**

In `src-tauri/src/lib.rs`, add `commands::get_sync_status, commands::set_sync_backend_url, commands::get_sync_backend_url, commands::bootstrap_sync_account, commands::generate_pairing_code, commands::join_sync_account, commands::sync_now,` to **both** `collect_commands!` blocks.

In the setup closure, after the existing archive-watcher startup block, add a periodic task mirroring `poll_loop`'s spawn pattern: every 5 minutes, if both `db.sync_backend_url()` and `db.sync_api_key()` return `Some`, construct a `SyncClient` (same as the commands above: `SyncClient::new(state.http_client.clone(), base_url)`) and call `sync::engine::run_sync_cycle`, storing the result via `sync::engine::summarize_cycle_result` into `state.sync_status` — the exact same two-line pattern `sync_now` uses, so the periodic task and the manual command can never disagree on how a result maps to a status.

- [ ] **Step 4: Frontend — `ipc.ts` wrappers**

In `src/lib/ipc.ts`, add:
```ts
getSyncStatus: () => commands.getSyncStatus().then(unwrap),
getSyncBackendUrl: () => commands.getSyncBackendUrl().then(unwrap),
setSyncBackendUrl: (url: string) => commands.setSyncBackendUrl(url).then(unwrap),
bootstrapSyncAccount: (deviceName: string) => commands.bootstrapSyncAccount(deviceName).then(unwrap),
generatePairingCode: () => commands.generatePairingCode().then(unwrap),
joinSyncAccount: (pairingCode: string, deviceName: string) => commands.joinSyncAccount(pairingCode, deviceName).then(unwrap),
syncNow: () => commands.syncNow().then(unwrap),
```

- [ ] **Step 5: Frontend — `SyncSettings.tsx`**

Create `src/settings/SyncSettings.tsx`, following `WarmupSettings.tsx`'s presentational-component convention exactly (state/handlers passed as props, no direct IPC calls inside):
```tsx
interface Props {
  backendUrl: string | null;
  status: { lastRunAt: string; outcome: string; pushed: number; pulled: number } | null;
  pairingCode: string | null;
  onSaveBackendUrl: (url: string) => void;
  onBootstrap: () => void;
  onGenerateCode: () => void;
  onJoin: (code: string, deviceName: string) => void;
  onSyncNow: () => void;
}

export function SyncSettings({ backendUrl, status, pairingCode, onSaveBackendUrl, onBootstrap, onGenerateCode, onJoin, onSyncNow }: Props) {
  const [urlInput, setUrlInput] = useState(backendUrl ?? '');
  const [joinCode, setJoinCode] = useState('');
  const [deviceName, setDeviceName] = useState('');
  const configured = Boolean(backendUrl);

  return (
    <div className="space-y-3 text-[12px]">
      <div className="flex items-center gap-2">
        <input
          type="text"
          value={urlInput}
          onChange={(e) => setUrlInput(e.target.value)}
          placeholder="https://your-sync-backend.example.com"
          className="px-2 py-0.5 rounded bg-[var(--color-bg-surface)] text-[11px] flex-1"
        />
        <button
          type="button"
          onClick={() => onSaveBackendUrl(urlInput)}
          className="px-2 py-0.5 rounded bg-[var(--color-bg-surface-hover)] hover:bg-[var(--color-border-hover)] text-[color:var(--color-text)] text-[11px]"
        >
          Save
        </button>
      </div>
      <div className="flex items-center justify-between">
        <span className="text-neutral-300">
          {status ? `Last sync: ${status.outcome} (${status.pushed} pushed, ${status.pulled} pulled)` : 'Not configured'}
        </span>
        <button
          type="button"
          onClick={onSyncNow}
          disabled={!configured}
          className="px-2 py-0.5 rounded bg-[var(--color-teal-dim)] hover:bg-teal-500/25 text-[color:var(--color-teal)] text-[11px] disabled:opacity-40"
        >
          Sync now
        </button>
      </div>
      <div className="flex items-center gap-2">
        <button
          disabled={!configured}
          type="button"
          onClick={onBootstrap}
          className="px-2 py-0.5 rounded bg-[var(--color-bg-surface-hover)] hover:bg-[var(--color-border-hover)] text-[color:var(--color-text)] text-[11px] disabled:opacity-40"
        >
          Enable on this device
        </button>
        <button
          disabled={!configured}
          type="button"
          onClick={onGenerateCode}
          className="px-2 py-0.5 rounded bg-[var(--color-bg-surface-hover)] hover:bg-[var(--color-border-hover)] text-[color:var(--color-text)] text-[11px] disabled:opacity-40"
        >
          Generate pairing code
        </button>
      </div>
      {pairingCode && (
        <div className="text-[color:var(--color-teal)] font-mono text-[13px]">{pairingCode}</div>
      )}
      <div className="flex items-center gap-2">
        <input
          type="text"
          value={joinCode}
          onChange={(e) => setJoinCode(e.target.value)}
          placeholder="Pairing code"
          className="px-2 py-0.5 rounded bg-[var(--color-bg-surface)] text-[11px] w-24"
        />
        <input
          type="text"
          value={deviceName}
          onChange={(e) => setDeviceName(e.target.value)}
          placeholder="This device's name"
          className="px-2 py-0.5 rounded bg-[var(--color-bg-surface)] text-[11px] flex-1"
        />
        <button
          disabled={!configured}
          type="button"
          onClick={() => onJoin(joinCode, deviceName)}
          className="px-2 py-0.5 rounded bg-[var(--color-teal-dim)] hover:bg-teal-500/25 text-[color:var(--color-teal)] text-[11px] disabled:opacity-40"
        >
          Join
        </button>
      </div>
    </div>
  );
}
```
(`useState` needs `import { useState } from 'react';` at the top of the file.)

- [ ] **Step 6: Wire `SyncSettings` into `SettingsPanel.tsx`**

Following the exact `<section>`/`<Card>`/`<WarmupSettings ... />` pattern already used for the Warm-up section (see `SettingsPanel.tsx` around its `<WarmupSettings>` usage), add state (`backendUrl`, `syncStatus`, `pairingCode`) — loaded once on mount via `ipc.getSyncBackendUrl()`/`ipc.getSyncStatus()`, the same way the panel already loads its other initial state — and handlers (`handleSaveBackendUrl`, `handleBootstrap`, `handleGenerateCode`, `handleJoin`, `handleSyncNow`) that call the new `ipc.ts` wrappers and update that state from the result, then add a new section:
```tsx
      {/* Sync */}
      <section className="flex flex-col gap-[var(--space-sm)]">
        <h2 className="text-[length:var(--text-label)] font-[var(--weight-semibold)] text-[color:var(--color-text-muted)] uppercase tracking-[0.04em] px-[var(--space-2xs)]">
          Sync
        </h2>
        <Card className="p-[var(--space-md)]">
          <SyncSettings
            backendUrl={backendUrl}
            status={syncStatus}
            pairingCode={pairingCode}
            onSaveBackendUrl={handleSaveBackendUrl}
            onBootstrap={handleBootstrap}
            onGenerateCode={handleGenerateCode}
            onJoin={handleJoin}
            onSyncNow={handleSyncNow}
          />
        </Card>
      </section>
```
Import it at the top: `import { SyncSettings } from './SyncSettings';`.

- [ ] **Step 7: Frontend test**

Create `src/settings/__tests__/SyncSettings.test.tsx`, mirroring the existing `WarmupSettings.test.tsx`'s structure — render the component with mock props (`backendUrl: null` in one test, `backendUrl: 'https://example.com'` in another), assert the status text renders, assert "Sync now"/"Enable on this device"/"Generate pairing code"/"Join" are disabled when `backendUrl` is `null` and enabled when it's set, type a URL into the backend-URL input and click "Save," asserting `onSaveBackendUrl` was called with the typed value, click "Sync now" (with a backend URL set) and assert `onSyncNow` was called, type into the join-code/device-name inputs and click "Join," asserting `onJoin` was called with the typed values.

- [ ] **Step 8: Run tests**

Run: `cd src-tauri && cargo test` — expect PASS (Rust side).
Run: `npm test` (or the project's frontend test command — check `package.json`'s `scripts.test`) — expect PASS including the new `SyncSettings.test.tsx`.
Run: `npx tsc --noEmit` (or the project's typecheck command) — expect clean, since `ipc.ts`/`bindings.ts` types must line up with the new Rust commands (regenerate specta bindings first if the project has a command for that — check for a `bindings.ts` generation script/command in `package.json` or `src-tauri`'s dev workflow before assuming this step, since specta auto-generates `src/lib/generated/bindings.ts` from the Rust command signatures).

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/app_state.rs src-tauri/src/commands.rs src-tauri/src/lib.rs src-tauri/src/store/queries.rs src/settings/SyncSettings.tsx src/settings/SettingsPanel.tsx src/settings/__tests__/SyncSettings.test.tsx src/lib/ipc.ts src/lib/generated/bindings.ts
git commit -m "feat: add sync Tauri commands and Settings UI"
```

---

### Task 5: End-to-end verification against the real sync-backend

**Files:**
- Create: `src-tauri/tests/sync_e2e.rs`

**Interfaces:**
- Consumes: everything from Tasks 1-4.

- [ ] **Step 1: Write the test**

Create `src-tauri/tests/sync_e2e.rs` — a test gated behind an environment variable so it only runs when a real (test) `sync-backend` instance is available, never accidentally against production:
```rust
use claude_switchboard_lib::store::{Db, StoredTranscriptLine};
use claude_switchboard_lib::sync::{engine::run_sync_cycle, SyncClient, SyncOutcome};
use std::sync::Arc;

/// Requires a real sync-backend instance reachable at SYNC_E2E_BASE_URL
/// (point this at a LOCAL TEST instance only — e.g. the same
/// `sync-backend-dev-pg`-backed server used during that plan's own
/// development, run via `cargo run -p sync-backend` with DATABASE_URL set
/// to a test database. NEVER point this at a real production backend).
/// Skips (passes trivially) if the env var isn't set, so this doesn't
/// break `cargo test` for anyone who hasn't started a local server.
#[tokio::test]
async fn desktop_app_can_push_and_pull_against_a_real_backend() {
    let Ok(base_url) = std::env::var("SYNC_E2E_BASE_URL") else {
        eprintln!("SYNC_E2E_BASE_URL not set — skipping (this is expected in normal `cargo test` runs)");
        return;
    };

    let client = SyncClient::new(Arc::new(reqwest::Client::new()), base_url);
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path()).unwrap();
    let device_id = db.device_id().unwrap();

    let api_key = match client.create_account(device_id, "e2e-test-device".to_string()).await {
        SyncOutcome::Ok(resp) => resp.api_key,
        other => panic!("account creation failed: {other:?}"),
    };
    db.set_sync_api_key(&api_key).unwrap();

    db.insert_transcript_lines(&[StoredTranscriptLine {
        project_slug: "e2e".into(),
        session_id: "session-1".into(),
        jsonl_path: "e2e/session-1.jsonl".into(),
        line_no: 0,
        raw_line: "{\"hello\":\"world\"}".into(),
    }])
    .unwrap();

    let result = run_sync_cycle(&db, &client, &api_key).await;
    match result {
        claude_switchboard_lib::sync::engine::SyncCycleResult::Ok { pushed, .. } => {
            assert_eq!(pushed, 1);
        }
        other => panic!("sync cycle failed: {other:?}"),
    }
}
```

- [ ] **Step 2: Run it (optional, manual)**

This test is skip-by-default (no `SYNC_E2E_BASE_URL` set), so `cargo test` passes normally without it doing anything. To actually exercise it: start a LOCAL TEST instance of `sync-backend` (`cargo run -p sync-backend` with `DATABASE_URL` pointed at a throwaway test database — never production), then run:
```bash
SYNC_E2E_BASE_URL="http://localhost:8787" cargo test --test sync_e2e -- --ignored --nocapture
```
Confirm it passes.

- [ ] **Step 3: Run the full test suite (without the e2e env var set)**

Run: `cd src-tauri && cargo test`
Expected: PASS (the e2e test skips cleanly since `SYNC_E2E_BASE_URL` isn't set in normal CI/dev runs).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/tests/sync_e2e.rs
git commit -m "test: add opt-in end-to-end sync test against a real backend instance"
```
