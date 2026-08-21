# Sync Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the sync backend service (accounts, device pairing, archive push/pull) that devices will sync against — the server half of phase 2. The client-side integration (schema migration, sync engine, Settings UI) is a separate follow-on plan, built once this one is done.

**Architecture:** A new Cargo workspace at the repo root containing the existing `src-tauri` crate, a new `archive-sync-types` crate (shared serde DTOs), and a new `sync-backend` crate (axum + sqlx/Postgres). The sync model is an append-only replicated log: every archived row belongs to exactly one device, so push/pull never has a write-write conflict to resolve — `device_id` (client-generated, sent by the client at registration time) is the partition key.

**Tech Stack:** Rust, axum 0.8, sqlx 0.8 (Postgres, runtime-tokio), tokio, serde. All API/DB patterns below (axum extractors, sqlx runtime queries, `sqlx::migrate!`, `#[sqlx::test]`, `tower::ServiceExt::oneshot`) were verified to compile and run correctly against a real Postgres 16 instance before this plan was written — they are not speculative.

**Spec:** `docs/superpowers/specs/2026-08-21-cross-machine-archive-sync-design.md` — read it alongside this plan.

## Global Constraints

- The backend never generates a device's `device_id` — the client generates it locally (independent of sync, since phase 1's local archive works with or without sync ever being enabled) and sends it at registration/join time. The backend only validates and stores it.
- All wire-format types (request/response bodies) live in `archive-sync-types` and use plain `String` for UUIDs — not the `uuid` crate — so that crate stays a lean dependency shared with `src-tauri`, which doesn't currently depend on `uuid`. The `sync-backend` crate parses those strings into `uuid::Uuid` internally for its own Postgres column typing.
- `push`/`pull` payloads never include `device_id` on individual rows — the server stamps `device_id`/`user_id` from the authenticated caller (the Bearer token), never trusting a client-supplied value for its own identity.
- Every backend table row keyed by `(device_id, ...)` uses `INSERT ... ON CONFLICT DO NOTHING`, mirroring the client's own idempotent insert semantics — a retried push must never create duplicates.
- `api_key_hash` stores a SHA-256 hex digest of the raw key, never the raw key itself. The raw key is returned to the caller exactly once, at registration/join time.
- Tests require a real local Postgres instance reachable via `DATABASE_URL` (see Task 2) — this is a documented, one-time local dev-environment requirement, not something each task re-explains.

---

### Task 1: Cargo workspace + `archive-sync-types` crate

**Files:**
- Create: `Cargo.toml` (new, at repo root)
- Modify: `src-tauri/Cargo.toml:82-87` (remove the `[profile.release]` block — profiles are workspace-level only)
- Delete: `src-tauri/Cargo.lock` (regenerated at the workspace root)
- Create: `archive-sync-types/Cargo.toml`
- Create: `archive-sync-types/src/lib.rs`

**Interfaces:**
- Produces: `SyncTranscriptLine`, `SyncFileSnapshot`, `PushRequest`, `PushResponse`, `PullQuery`, `PullResponse`, `CreateAccountRequest`, `CreateAccountResponse`, `PairCodeResponse`, `JoinRequest`, `JoinResponse` — all `#[derive(Serialize, Deserialize)]`, all consumed by `sync-backend` (Tasks 3-4) and later by the client-side plan.

- [ ] **Step 1: Create the workspace root `Cargo.toml`**

```toml
[workspace]
members = ["src-tauri", "sync-backend", "archive-sync-types"]
resolver = "2"

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
strip = true
panic = "abort"
```

- [ ] **Step 2: Remove the profile block from `src-tauri/Cargo.toml`**

Delete these lines (currently `src-tauri/Cargo.toml:82-87`):
```toml
[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
strip = true
panic = "abort"
```

- [ ] **Step 3: Regenerate the lockfile at the workspace root**

```bash
rm src-tauri/Cargo.lock
```

(`sync-backend` and `archive-sync-types` don't exist as directories yet, so the workspace won't fully resolve until Step 4 creates at least stub crates — proceed to Step 4 before building.)

- [ ] **Step 4: Create a placeholder `sync-backend` crate (fleshed out in Task 2)**

Create `sync-backend/Cargo.toml`:
```toml
[package]
name = "sync-backend"
version = "0.1.0"
edition = "2021"

[dependencies]
```

Create `sync-backend/src/main.rs`:
```rust
fn main() {}
```

- [ ] **Step 5: Create `archive-sync-types`**

Create `archive-sync-types/Cargo.toml`:
```toml
[package]
name = "archive-sync-types"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
```

Create `archive-sync-types/src/lib.rs`:
```rust
use serde::{Deserialize, Serialize};

/// Wire format for one archived transcript line. Deliberately has no
/// `device_id` field — the server stamps that from the authenticated
/// caller, never trusting a client-supplied value for its own identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncTranscriptLine {
    pub project_slug: String,
    pub session_id: String,
    pub jsonl_path: String,
    pub line_no: i64,
    pub raw_line: String,
    pub ingested_at: i64,
}

/// Wire format for one archived file snapshot. Same no-device_id rule as
/// SyncTranscriptLine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncFileSnapshot {
    pub source_path: String,
    pub kind: String,
    pub content: String,
    pub content_hash: String,
    pub captured_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushRequest {
    pub transcript_lines: Vec<SyncTranscriptLine>,
    pub file_snapshots: Vec<SyncFileSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushResponse {
    pub transcript_lines_accepted: usize,
    pub file_snapshots_accepted: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PullQuery {
    pub since_transcript_seq: i64,
    pub since_snapshot_seq: i64,
    pub limit: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PullResponse {
    pub transcript_lines: Vec<SyncTranscriptLine>,
    pub file_snapshots: Vec<SyncFileSnapshot>,
    pub transcript_seq_high_water: i64,
    pub snapshot_seq_high_water: i64,
}

/// `device_id` is client-generated (a UUID formatted as a string) —
/// phase 1's local archive already assigns one per install, independent
/// of whether sync is ever enabled. The server validates and stores it,
/// never mints its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateAccountRequest {
    pub device_id: String,
    pub device_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateAccountResponse {
    pub user_id: String,
    pub device_id: String,
    pub api_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairCodeResponse {
    pub pairing_code: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinRequest {
    pub pairing_code: String,
    pub device_id: String,
    pub device_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JoinResponse {
    pub user_id: String,
    pub device_id: String,
    pub api_key: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dto_round_trips_through_json() {
        let line = SyncTranscriptLine {
            project_slug: "p".into(),
            session_id: "s".into(),
            jsonl_path: "p/s.jsonl".into(),
            line_no: 0,
            raw_line: "{}".into(),
            ingested_at: 0,
        };
        let json = serde_json::to_string(&line).unwrap();
        let back: SyncTranscriptLine = serde_json::from_str(&json).unwrap();
        assert_eq!(line, back);

        let snap = SyncFileSnapshot {
            source_path: "/x".into(),
            kind: "settings".into(),
            content: "{}".into(),
            content_hash: "h".into(),
            captured_at: 0,
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: SyncFileSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(snap, back);

        let push = PushRequest { transcript_lines: vec![line], file_snapshots: vec![snap] };
        let json = serde_json::to_string(&push).unwrap();
        let back: PushRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(push, back);

        let create_req = CreateAccountRequest {
            device_id: "d1".into(),
            device_name: "MacBook".into(),
        };
        let json = serde_json::to_string(&create_req).unwrap();
        let back: CreateAccountRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(create_req, back);

        let join_req = JoinRequest {
            pairing_code: "ABCD1234".into(),
            device_id: "d2".into(),
            device_name: "Desktop".into(),
        };
        let json = serde_json::to_string(&join_req).unwrap();
        let back: JoinRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(join_req, back);
    }
}
```

Add `serde_json` as a dev-dependency in `archive-sync-types/Cargo.toml` (needed by the test above):
```toml
[dev-dependencies]
serde_json = "1"
```

- [ ] **Step 6: Verify the workspace builds and the existing app is unaffected**

Run: `cargo build --workspace` from the repo root.
Expected: builds all three members successfully. This is the highest-regression-risk step in this task — `src-tauri` must build exactly as it did before the workspace conversion.

Run: `cargo test -p archive-sync-types`
Expected: `every_dto_round_trips_through_json` passes.

Run: `cargo test -p claude-switchboard` (the existing app's full suite)
Expected: PASS — same results as before this task (458 passed, 1 pre-existing unrelated failure in `commands::tests::warmup_suggestion_takes_only_the_earliest_event_per_day`, 2 ignored — this specific failure predates this work and is not something to fix here).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src-tauri/Cargo.toml archive-sync-types sync-backend
git rm --cached src-tauri/Cargo.lock 2>/dev/null; git add Cargo.lock
git commit -m "feat: convert to a Cargo workspace, add archive-sync-types crate"
```

---

### Task 2: `sync-backend` skeleton — Postgres connection, migrations, health check

**Files:**
- Modify: `sync-backend/Cargo.toml` (replace the placeholder from Task 1)
- Create: `sync-backend/migrations/0001_users_devices.sql`
- Create: `sync-backend/src/lib.rs`
- Modify: `sync-backend/src/main.rs` (replace the placeholder from Task 1)

**Interfaces:**
- Produces: `pub struct AppState { pub pool: PgPool }`, `pub fn app(pool: PgPool) -> Router` (routes added incrementally in later tasks; this task wires the `Router` construction itself plus a `/health` route) — consumed by Tasks 3-5.

- [ ] **Step 1: Fill in `sync-backend/Cargo.toml`**

```toml
[package]
name = "sync-backend"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = "0.8"
tokio = { version = "1", features = ["full"] }
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4", "serde"] }
sha2 = "0.10"
rand = "0.9"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
archive-sync-types = { path = "../archive-sync-types" }

[dev-dependencies]
tower = { version = "0.5", features = ["util"] }
```

- [ ] **Step 2: Create the first migration**

Create `sync-backend/migrations/0001_users_devices.sql`:
```sql
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE devices (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    name TEXT NOT NULL,
    api_key_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ
);
CREATE UNIQUE INDEX idx_devices_api_key_hash ON devices(api_key_hash);

CREATE TABLE pairing_codes (
    code TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    expires_at TIMESTAMPTZ NOT NULL,
    used_at TIMESTAMPTZ
);
```

`gen_random_uuid()` is a Postgres-16 built-in (verified directly — no `pgcrypto` extension needed).

- [ ] **Step 3: Write `sync-backend/src/lib.rs`**

```rust
use axum::{routing::get, Json, Router};
use serde::Serialize;
use sqlx::PgPool;
use std::sync::Arc;

pub mod auth;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

pub fn app(pool: PgPool) -> Router {
    Router::new()
        .route("/health", get(health))
        .with_state(Arc::new(AppState { pool }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[sqlx::test]
    async fn health_returns_ok(pool: PgPool) {
        let response = app(pool)
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }
}
```

- [ ] **Step 4: Create the `auth` module (stub for now, real `AuthedDevice` extractor lands in Task 3)**

Create `sync-backend/src/auth.rs`:
```rust
use sha2::{Digest, Sha256};

/// SHA-256 hex digest of a raw API key. High-entropy random tokens (not
/// user-chosen secrets) don't need a slow hash — this matches how e.g.
/// GitHub stores personal access tokens.
pub fn hash_api_key(raw: &str) -> String {
    format!("{:x}", Sha256::digest(raw.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic_and_hex() {
        let h1 = hash_api_key("abc123");
        let h2 = hash_api_key("abc123");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn different_keys_hash_differently() {
        assert_ne!(hash_api_key("a"), hash_api_key("b"));
    }
}
```

- [ ] **Step 5: Write `sync-backend/src/main.rs`**

```rust
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set (e.g. postgres://user:pass@localhost/sync)");
    let bind_addr =
        std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8787".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .expect("connect to Postgres");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    let app = sync_backend::app(pool);

    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .expect("bind listener");
    tracing::info!("sync-backend listening on {bind_addr}");
    axum::serve(listener, app).await.expect("serve");
}
```

Note the crate name in `sync_backend::app(...)` — Cargo automatically converts the hyphenated package name `sync-backend` to the underscored identifier `sync_backend` for `use`/path purposes; this is standard and not a typo.

- [ ] **Step 6: Set up local Postgres for testing, then verify**

Document (in this step, not a separate file) how to get a local Postgres running for development/testing:
```bash
docker run -d --name sync-backend-dev-pg -e POSTGRES_PASSWORD=devpass -e POSTGRES_DB=sync_backend_dev -p 5433:5432 postgres:16-alpine
export DATABASE_URL="postgres://postgres:devpass@localhost:5433/sync_backend_dev"
```
(Port 5433 is used here to avoid colliding with any other locally-running Postgres on the default 5432 — check `docker ps` first and pick a free port if 5433 is also taken.)

Run: `cargo build -p sync-backend`
Expected: builds successfully.

Run (with `DATABASE_URL` exported as above): `cargo test -p sync-backend`
Expected: both `health_returns_ok` and the two `hash_api_key` tests pass. `#[sqlx::test]` automatically creates a fresh, isolated test database per test run and applies `./migrations` — no manual test-DB setup needed beyond having `DATABASE_URL` point at a reachable Postgres server.

Run: `cargo build --workspace` once more, to confirm the whole workspace (including `src-tauri`) still builds together.

- [ ] **Step 7: Commit**

```bash
git add sync-backend
git commit -m "feat: sync-backend skeleton (Postgres connection, migrations, health check)"
```

---

### Task 3: Accounts, device pairing, and the `AuthedDevice` extractor

**Files:**
- Modify: `sync-backend/src/auth.rs` (add the real `AuthedDevice` extractor)
- Create: `sync-backend/src/accounts.rs`
- Modify: `sync-backend/src/lib.rs` (register the new routes, add the `accounts` module)

**Interfaces:**
- Consumes: `archive-sync-types::{CreateAccountRequest, CreateAccountResponse, PairCodeResponse, JoinRequest, JoinResponse}` (Task 1), `AppState` (Task 2).
- Produces: `pub struct AuthedDevice { pub user_id: Uuid, pub device_id: Uuid }` (implements `FromRequestParts<Arc<AppState>>`) — consumed by Task 4's push/pull handlers.

- [ ] **Step 1: Add the `AuthedDevice` extractor to `sync-backend/src/auth.rs`**

`auth.rs` already has (from Task 2) `hash_api_key` plus a `#[cfg(test)] mod tests { ... }` block containing `hash_is_deterministic_and_hex` and `different_keys_hash_differently`. Leave those untouched.

Insert the following new code **above** the existing `#[cfg(test)] mod tests { ... }` block (i.e., between `hash_api_key`'s closing brace and the `#[cfg(test)]` line):
```rust
use axum::extract::FromRequestParts;
use axum::http::{request::Parts, StatusCode};
use std::sync::Arc;
use uuid::Uuid;

use crate::AppState;

/// A request authenticated by a valid device API key. Extracting this
/// type is how every protected route requires auth — a route that takes
/// `AuthedDevice` as a handler argument cannot be reached without one.
pub struct AuthedDevice {
    pub user_id: Uuid,
    pub device_id: Uuid,
}

impl FromRequestParts<Arc<AppState>> for AuthedDevice {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let raw_key = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or((StatusCode::UNAUTHORIZED, "missing bearer token".to_string()))?;

        let hash = hash_api_key(raw_key);

        let row: Option<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT id, user_id FROM devices WHERE api_key_hash = $1",
        )
        .bind(&hash)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let (device_id, user_id) = row.ok_or((
            StatusCode::UNAUTHORIZED,
            "invalid api key".to_string(),
        ))?;

        // Best-effort — a failure here must never fail the actual request.
        let _ = sqlx::query("UPDATE devices SET last_seen_at = now() WHERE id = $1")
            .bind(device_id)
            .execute(&state.pool)
            .await;

        Ok(AuthedDevice { user_id, device_id })
    }
}

/// Generates a raw, high-entropy API key (32 random bytes, hex-encoded).
pub fn generate_api_key() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::rng().random();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

const PAIRING_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no O/0/I/1 — easy to mistype

/// Generates an 8-character pairing code from a restricted, unambiguous
/// alphabet — meant to be read off one screen and typed on another.
pub fn generate_pairing_code() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..8)
        .map(|_| PAIRING_CODE_ALPHABET[rng.random_range(0..PAIRING_CODE_ALPHABET.len())] as char)
        .collect()
}
```

Then add these two test functions **inside** the existing `mod tests { ... }` block, alongside `hash_is_deterministic_and_hex` and `different_keys_hash_differently` (do not create a second `mod tests` — Rust will not allow two modules with the same name in one file):
```rust
    #[test]
    fn api_key_is_64_hex_chars() {
        let key = generate_api_key();
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn pairing_code_is_8_chars_from_restricted_alphabet() {
        let code = generate_pairing_code();
        assert_eq!(code.len(), 8);
        assert!(code.bytes().all(|b| PAIRING_CODE_ALPHABET.contains(&b)));
    }
```

- [ ] **Step 2: Write `sync-backend/src/accounts.rs`**

```rust
use archive_sync_types::{
    CreateAccountRequest, CreateAccountResponse, JoinRequest, JoinResponse, PairCodeResponse,
};
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use crate::auth::{generate_api_key, generate_pairing_code, hash_api_key, AuthedDevice};
use crate::AppState;

pub async fn create_account(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateAccountRequest>,
) -> Result<Json<CreateAccountResponse>, (StatusCode, String)> {
    let device_id = Uuid::parse_str(&req.device_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "device_id must be a valid UUID".to_string()))?;

    let (user_id,): (Uuid,) = sqlx::query_as("INSERT INTO users DEFAULT VALUES RETURNING id")
        .fetch_one(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let api_key = generate_api_key();
    let hash = hash_api_key(&api_key);

    sqlx::query(
        "INSERT INTO devices (id, user_id, name, api_key_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(device_id)
    .bind(user_id)
    .bind(&req.device_name)
    .bind(&hash)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    Ok(Json(CreateAccountResponse {
        user_id: user_id.to_string(),
        device_id: device_id.to_string(),
        api_key,
    }))
}

pub async fn pair_code(
    State(state): State<Arc<AppState>>,
    device: AuthedDevice,
) -> Result<Json<PairCodeResponse>, (StatusCode, String)> {
    for _ in 0..5 {
        let code = generate_pairing_code();
        let inserted = sqlx::query(
            "INSERT INTO pairing_codes (code, user_id, expires_at)
             VALUES ($1, $2, now() + interval '10 minutes')
             ON CONFLICT (code) DO NOTHING",
        )
        .bind(&code)
        .bind(device.user_id)
        .execute(&state.pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if inserted.rows_affected() == 1 {
            return Ok(Json(PairCodeResponse { pairing_code: code }));
        }
        // Extremely unlikely collision (32^8 possibilities) — retry with a fresh code.
    }
    Err((StatusCode::INTERNAL_SERVER_ERROR, "could not generate a unique pairing code".to_string()))
}

pub async fn join(
    State(state): State<Arc<AppState>>,
    Json(req): Json<JoinRequest>,
) -> Result<Json<JoinResponse>, (StatusCode, String)> {
    let device_id = Uuid::parse_str(&req.device_id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "device_id must be a valid UUID".to_string()))?;

    let row: Option<(Uuid,)> = sqlx::query_as(
        "UPDATE pairing_codes SET used_at = now()
         WHERE code = $1 AND used_at IS NULL AND expires_at > now()
         RETURNING user_id",
    )
    .bind(&req.pairing_code)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (user_id,) = row.ok_or((
        StatusCode::UNAUTHORIZED,
        "invalid, used, or expired pairing code".to_string(),
    ))?;

    let api_key = generate_api_key();
    let hash = hash_api_key(&api_key);

    sqlx::query(
        "INSERT INTO devices (id, user_id, name, api_key_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(device_id)
    .bind(user_id)
    .bind(&req.device_name)
    .bind(&hash)
    .execute(&state.pool)
    .await
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    Ok(Json(JoinResponse {
        user_id: user_id.to_string(),
        device_id: device_id.to_string(),
        api_key,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    async fn post_json(pool: &PgPool, uri: &str, body: serde_json::Value, bearer: Option<&str>) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method("POST").uri(uri).header("content-type", "application/json");
        if let Some(token) = bearer {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let request = builder.body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap();
        let response = app(pool.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = if bytes.is_empty() { serde_json::json!({}) } else { serde_json::from_slice(&bytes).unwrap() };
        (status, json)
    }

    #[sqlx::test]
    async fn create_account_then_pair_and_join_a_second_device(pool: PgPool) {
        let device_a_id = Uuid::new_v4().to_string();
        let (status, body) = post_json(
            &pool,
            "/v1/accounts",
            serde_json::json!({ "device_id": device_a_id, "device_name": "Device A" }),
            None,
        ).await;
        assert_eq!(status, StatusCode::OK);
        let api_key_a = body["api_key"].as_str().unwrap().to_string();
        assert_eq!(body["device_id"], device_a_id);

        // Pair-code requires authentication as an existing device.
        let (status, body) = post_json(&pool, "/v1/devices/pair-code", serde_json::json!({}), Some(&api_key_a)).await;
        assert_eq!(status, StatusCode::OK);
        let pairing_code = body["pairing_code"].as_str().unwrap().to_string();
        assert_eq!(pairing_code.len(), 8);

        let device_b_id = Uuid::new_v4().to_string();
        let (status, body) = post_json(
            &pool,
            "/v1/devices/join",
            serde_json::json!({ "pairing_code": pairing_code, "device_id": device_b_id, "device_name": "Device B" }),
            None,
        ).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["user_id"], body["user_id"]); // sanity
        assert_eq!(body["device_id"], device_b_id);

        // The pairing code is single-use — joining again with it must fail.
        let (status, _) = post_json(
            &pool,
            "/v1/devices/join",
            serde_json::json!({ "pairing_code": pairing_code, "device_id": Uuid::new_v4().to_string(), "device_name": "Device C" }),
            None,
        ).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    async fn pair_code_requires_auth(pool: PgPool) {
        let (status, _) = post_json(&pool, "/v1/devices/pair-code", serde_json::json!({}), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[sqlx::test]
    async fn join_rejects_unknown_pairing_code(pool: PgPool) {
        let (status, _) = post_json(
            &pool,
            "/v1/devices/join",
            serde_json::json!({ "pairing_code": "NOTREAL1", "device_id": Uuid::new_v4().to_string(), "device_name": "X" }),
            None,
        ).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
```

- [ ] **Step 3: Wire the new routes into `sync-backend/src/lib.rs`**

Change:
```rust
pub mod auth;
```
to:
```rust
pub mod accounts;
pub mod auth;
```

Change:
```rust
pub fn app(pool: PgPool) -> Router {
    Router::new()
        .route("/health", get(health))
        .with_state(Arc::new(AppState { pool }))
}
```
to:
```rust
pub fn app(pool: PgPool) -> Router {
    use axum::routing::post;
    Router::new()
        .route("/health", get(health))
        .route("/v1/accounts", post(accounts::create_account))
        .route("/v1/devices/pair-code", post(accounts::pair_code))
        .route("/v1/devices/join", post(accounts::join))
        .with_state(Arc::new(AppState { pool }))
}
```

- [ ] **Step 4: Run tests**

Run (with `DATABASE_URL` exported, per Task 2 Step 6): `cargo test -p sync-backend`
Expected: PASS — all tests in `auth.rs` and `accounts.rs`, plus the existing `health_returns_ok`.

- [ ] **Step 5: Commit**

```bash
git add sync-backend
git commit -m "feat: account creation and device pairing endpoints"
```

---

### Task 4: Archive push/pull endpoints

**Files:**
- Create: `sync-backend/migrations/0002_archive.sql`
- Create: `sync-backend/src/archive.rs`
- Modify: `sync-backend/src/lib.rs` (register the new routes, add the `archive` module)

**Interfaces:**
- Consumes: `archive_sync_types::{PushRequest, PushResponse, PullQuery, PullResponse, SyncTranscriptLine, SyncFileSnapshot}` (Task 1), `AuthedDevice` (Task 3).

- [ ] **Step 1: Create the archive migration**

Create `sync-backend/migrations/0002_archive.sql`:
```sql
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
CREATE INDEX idx_archive_transcript_lines_user_seq ON archive_transcript_lines(user_id, seq);

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
CREATE INDEX idx_archive_file_snapshots_user_seq ON archive_file_snapshots(user_id, seq);
```

- [ ] **Step 2: Write `sync-backend/src/archive.rs`**

```rust
use archive_sync_types::{PullQuery, PullResponse, PushRequest, PushResponse, SyncFileSnapshot, SyncTranscriptLine};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use std::sync::Arc;

use crate::auth::AuthedDevice;
use crate::AppState;

pub async fn push(
    State(state): State<Arc<AppState>>,
    device: AuthedDevice,
    Json(req): Json<PushRequest>,
) -> Result<Json<PushResponse>, (StatusCode, String)> {
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut transcript_lines_accepted = 0usize;
    for line in &req.transcript_lines {
        let result = sqlx::query(
            "INSERT INTO archive_transcript_lines
             (user_id, device_id, project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (device_id, jsonl_path, line_no) DO NOTHING",
        )
        .bind(device.user_id)
        .bind(device.device_id)
        .bind(&line.project_slug)
        .bind(&line.session_id)
        .bind(&line.jsonl_path)
        .bind(line.line_no)
        .bind(&line.raw_line)
        .bind(line.ingested_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if result.rows_affected() == 1 {
            transcript_lines_accepted += 1;
        }
    }

    let mut file_snapshots_accepted = 0usize;
    for snap in &req.file_snapshots {
        let result = sqlx::query(
            "INSERT INTO archive_file_snapshots
             (user_id, device_id, source_path, kind, content, content_hash, captured_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (device_id, source_path, content_hash) DO NOTHING",
        )
        .bind(device.user_id)
        .bind(device.device_id)
        .bind(&snap.source_path)
        .bind(&snap.kind)
        .bind(&snap.content)
        .bind(&snap.content_hash)
        .bind(snap.captured_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if result.rows_affected() == 1 {
            file_snapshots_accepted += 1;
        }
    }

    tx.commit()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(PushResponse { transcript_lines_accepted, file_snapshots_accepted }))
}

pub async fn pull(
    State(state): State<Arc<AppState>>,
    device: AuthedDevice,
    Query(q): Query<PullQuery>,
) -> Result<Json<PullResponse>, (StatusCode, String)> {
    let rows: Vec<(i64, String, String, String, i64, String, i64)> = sqlx::query_as(
        "SELECT seq, project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at
         FROM archive_transcript_lines
         WHERE user_id = $1 AND device_id != $2 AND seq > $3
         ORDER BY seq
         LIMIT $4",
    )
    .bind(device.user_id)
    .bind(device.device_id)
    .bind(q.since_transcript_seq)
    .bind(q.limit)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let transcript_seq_high_water = rows.last().map(|r| r.0).unwrap_or(q.since_transcript_seq);
    let transcript_lines: Vec<SyncTranscriptLine> = rows
        .into_iter()
        .map(|(_, project_slug, session_id, jsonl_path, line_no, raw_line, ingested_at)| SyncTranscriptLine {
            project_slug,
            session_id,
            jsonl_path,
            line_no,
            raw_line,
            ingested_at,
        })
        .collect();

    let rows: Vec<(i64, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT seq, source_path, kind, content, content_hash, captured_at
         FROM archive_file_snapshots
         WHERE user_id = $1 AND device_id != $2 AND seq > $3
         ORDER BY seq
         LIMIT $4",
    )
    .bind(device.user_id)
    .bind(device.device_id)
    .bind(q.since_snapshot_seq)
    .bind(q.limit)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let snapshot_seq_high_water = rows.last().map(|r| r.0).unwrap_or(q.since_snapshot_seq);
    let file_snapshots: Vec<SyncFileSnapshot> = rows
        .into_iter()
        .map(|(_, source_path, kind, content, content_hash, captured_at)| SyncFileSnapshot {
            source_path,
            kind,
            content,
            content_hash,
            captured_at,
        })
        .collect();

    Ok(Json(PullResponse {
        transcript_lines,
        file_snapshots,
        transcript_seq_high_water,
        snapshot_seq_high_water,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use sqlx::PgPool;
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn register_device(pool: &PgPool, name: &str) -> (String, String) {
        let device_id = Uuid::new_v4().to_string();
        let request = Request::builder()
            .method("POST")
            .uri("/v1/accounts")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({ "device_id": device_id, "device_name": name })).unwrap(),
            ))
            .unwrap();
        let response = app(pool.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (device_id, body["api_key"].as_str().unwrap().to_string())
    }

    async fn push_lines(pool: &PgPool, api_key: &str, lines: Vec<SyncTranscriptLine>) -> PushResponse {
        let body = PushRequest { transcript_lines: lines, file_snapshots: vec![] };
        let request = Request::builder()
            .method("POST")
            .uri("/v1/archive/push")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let response = app(pool.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn pull_lines(pool: &PgPool, api_key: &str) -> PullResponse {
        let request = Request::builder()
            .method("GET")
            .uri("/v1/archive/pull?since_transcript_seq=0&since_snapshot_seq=0&limit=500")
            .header("authorization", format!("Bearer {api_key}"))
            .body(Body::empty())
            .unwrap();
        let response = app(pool.clone()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn sample_line(n: i64) -> SyncTranscriptLine {
        SyncTranscriptLine {
            project_slug: "proj".into(),
            session_id: "sess".into(),
            jsonl_path: "proj/sess.jsonl".into(),
            line_no: n,
            raw_line: format!("{{\"n\":{n}}}"),
            ingested_at: 0,
        }
    }

    #[sqlx::test]
    async fn a_device_never_pulls_back_its_own_pushed_rows(pool: PgPool) {
        let (_device_a, key_a) = register_device(&pool, "A").await;
        let accepted = push_lines(&pool, &key_a, vec![sample_line(0), sample_line(1)]).await;
        assert_eq!(accepted.transcript_lines_accepted, 2);

        let pulled = pull_lines(&pool, &key_a).await;
        assert!(pulled.transcript_lines.is_empty(), "a device must not see its own rows in pull");
    }

    #[sqlx::test]
    async fn a_second_device_pulls_the_first_devices_pushed_rows(pool: PgPool) {
        let (_device_a, key_a) = register_device(&pool, "A").await;
        push_lines(&pool, &key_a, vec![sample_line(0), sample_line(1)]).await;

        // Device B joins the SAME account via the pairing flow, matching
        // real usage — a lone second /v1/accounts call would create a
        // second, unrelated user, which pull correctly scopes away from.
        let pair_request = Request::builder()
            .method("POST")
            .uri("/v1/devices/pair-code")
            .header("authorization", format!("Bearer {key_a}"))
            .body(Body::empty())
            .unwrap();
        let pair_response = app(pool.clone()).oneshot(pair_request).await.unwrap();
        let bytes = to_bytes(pair_response.into_body(), usize::MAX).await.unwrap();
        let code = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["pairing_code"]
            .as_str()
            .unwrap()
            .to_string();

        let device_b_id = Uuid::new_v4().to_string();
        let join_request = Request::builder()
            .method("POST")
            .uri("/v1/devices/join")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({ "pairing_code": code, "device_id": device_b_id, "device_name": "B" })).unwrap(),
            ))
            .unwrap();
        let join_response = app(pool.clone()).oneshot(join_request).await.unwrap();
        let bytes = to_bytes(join_response.into_body(), usize::MAX).await.unwrap();
        let key_b = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["api_key"]
            .as_str()
            .unwrap()
            .to_string();

        let pulled = pull_lines(&pool, &key_b).await;
        assert_eq!(pulled.transcript_lines.len(), 2, "device B must see device A's rows");
        assert_eq!(pulled.transcript_seq_high_water, 2);
    }

    #[sqlx::test]
    async fn re_pushing_the_same_batch_is_idempotent(pool: PgPool) {
        let (_device_a, key_a) = register_device(&pool, "A").await;
        let first = push_lines(&pool, &key_a, vec![sample_line(0)]).await;
        assert_eq!(first.transcript_lines_accepted, 1);
        let second = push_lines(&pool, &key_a, vec![sample_line(0)]).await;
        assert_eq!(second.transcript_lines_accepted, 0, "re-pushing an identical row must not duplicate it");
    }

    #[sqlx::test]
    async fn push_and_pull_require_auth(pool: PgPool) {
        let request = Request::builder()
            .method("GET")
            .uri("/v1/archive/pull?since_transcript_seq=0&since_snapshot_seq=0&limit=10")
            .body(Body::empty())
            .unwrap();
        let response = app(pool).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
```

- [ ] **Step 3: Wire the new routes into `sync-backend/src/lib.rs`**

Change:
```rust
pub mod accounts;
pub mod auth;
```
to:
```rust
pub mod accounts;
pub mod archive;
pub mod auth;
```

Change the route builder to:
```rust
pub fn app(pool: PgPool) -> Router {
    use axum::routing::post;
    Router::new()
        .route("/health", get(health))
        .route("/v1/accounts", post(accounts::create_account))
        .route("/v1/devices/pair-code", post(accounts::pair_code))
        .route("/v1/devices/join", post(accounts::join))
        .route("/v1/archive/push", post(archive::push))
        .route("/v1/archive/pull", get(archive::pull))
        .with_state(Arc::new(AppState { pool }))
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p sync-backend`
Expected: PASS — all tests across `auth.rs`, `accounts.rs`, and `archive.rs`.

- [ ] **Step 5: Commit**

```bash
git add sync-backend
git commit -m "feat: archive push/pull endpoints"
```

---

### Task 5: End-to-end integration test

**Files:**
- Create: `sync-backend/tests/end_to_end.rs`

**Interfaces:**
- Consumes: `sync_backend::app` (Tasks 2-4), `archive_sync_types::*` (Task 1).

- [ ] **Step 1: Write the end-to-end test**

Create `sync-backend/tests/end_to_end.rs`:
```rust
use archive_sync_types::{PullResponse, PushRequest, PushResponse, SyncFileSnapshot, SyncTranscriptLine};
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

async fn call(pool: &PgPool, method: &str, uri: &str, api_key: Option<&str>, body: Option<serde_json::Value>) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(key) = api_key {
        builder = builder.header("authorization", format!("Bearer {key}"));
    }
    let request = if let Some(b) = body {
        builder.header("content-type", "application/json").body(Body::from(serde_json::to_vec(&b).unwrap())).unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };
    let response = sync_backend::app(pool.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json = if bytes.is_empty() { serde_json::json!({}) } else { serde_json::from_slice(&bytes).unwrap() };
    (status, json)
}

/// Full lifecycle across two devices: create an account on device A, pair
/// and join device B onto the same account, push a mix of transcript
/// lines and file snapshots from A, pull them from B, verify content and
/// the high-water cursors, then push a second batch from B and confirm A
/// picks up exactly the new rows (not a re-delivery of what it already
/// pushed itself).
#[sqlx::test]
async fn two_device_full_sync_lifecycle(pool: PgPool) {
    let device_a = Uuid::new_v4().to_string();
    let (status, body) = call(&pool, "POST", "/v1/accounts", None, Some(serde_json::json!({ "device_id": device_a, "device_name": "A" }))).await;
    assert_eq!(status, StatusCode::OK);
    let key_a = body["api_key"].as_str().unwrap().to_string();

    let (status, body) = call(&pool, "POST", "/v1/devices/pair-code", Some(&key_a), Some(serde_json::json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    let code = body["pairing_code"].as_str().unwrap().to_string();

    let device_b = Uuid::new_v4().to_string();
    let (status, body) = call(&pool, "POST", "/v1/devices/join", None, Some(serde_json::json!({ "pairing_code": code, "device_id": device_b, "device_name": "B" }))).await;
    assert_eq!(status, StatusCode::OK);
    let key_b = body["api_key"].as_str().unwrap().to_string();

    let push_body = PushRequest {
        transcript_lines: vec![SyncTranscriptLine {
            project_slug: "proj".into(),
            session_id: "sess".into(),
            jsonl_path: "proj/sess.jsonl".into(),
            line_no: 0,
            raw_line: "{}".into(),
            ingested_at: 100,
        }],
        file_snapshots: vec![SyncFileSnapshot {
            source_path: "/home/a/.claude/settings.json".into(),
            kind: "settings".into(),
            content: "{\"x\":1}".into(),
            content_hash: "hash1".into(),
            captured_at: 100,
        }],
    };
    let (status, body) = call(&pool, "POST", "/v1/archive/push", Some(&key_a), Some(serde_json::to_value(&push_body).unwrap())).await;
    assert_eq!(status, StatusCode::OK);
    let pushed: PushResponse = serde_json::from_value(body).unwrap();
    assert_eq!(pushed.transcript_lines_accepted, 1);
    assert_eq!(pushed.file_snapshots_accepted, 1);

    let (status, body) = call(&pool, "GET", "/v1/archive/pull?since_transcript_seq=0&since_snapshot_seq=0&limit=500", Some(&key_b), None).await;
    assert_eq!(status, StatusCode::OK);
    let pulled: PullResponse = serde_json::from_value(body).unwrap();
    assert_eq!(pulled.transcript_lines.len(), 1);
    assert_eq!(pulled.transcript_lines[0].raw_line, "{}");
    assert_eq!(pulled.file_snapshots.len(), 1);
    assert_eq!(pulled.file_snapshots[0].content_hash, "hash1");
    assert_eq!(pulled.transcript_seq_high_water, 1);
    assert_eq!(pulled.snapshot_seq_high_water, 1);

    // Device B pushes its own new row; device A, pulling from its own
    // last-seen cursor (1, since it already has seq 1 — its own row),
    // should see only B's new row, not a re-delivery of its own.
    let push_body_b = PushRequest {
        transcript_lines: vec![SyncTranscriptLine {
            project_slug: "proj".into(),
            session_id: "sess-b".into(),
            jsonl_path: "proj/sess-b.jsonl".into(),
            line_no: 0,
            raw_line: "{\"from\":\"b\"}".into(),
            ingested_at: 200,
        }],
        file_snapshots: vec![],
    };
    let (status, _) = call(&pool, "POST", "/v1/archive/push", Some(&key_b), Some(serde_json::to_value(&push_body_b).unwrap())).await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = call(&pool, "GET", "/v1/archive/pull?since_transcript_seq=1&since_snapshot_seq=1&limit=500", Some(&key_a), None).await;
    assert_eq!(status, StatusCode::OK);
    let pulled: PullResponse = serde_json::from_value(body).unwrap();
    assert_eq!(pulled.transcript_lines.len(), 1);
    assert_eq!(pulled.transcript_lines[0].raw_line, "{\"from\":\"b\"}");
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p sync-backend --test end_to_end`
Expected: PASS.

Run: `cargo test --workspace` once, for a final full-repo confirmation.
Expected: PASS everywhere except the one pre-existing, unrelated `commands::tests::warmup_suggestion_takes_only_the_earliest_event_per_day` failure noted in Task 1.

- [ ] **Step 3: Commit**

```bash
git add sync-backend
git commit -m "test: end-to-end two-device sync lifecycle"
```
