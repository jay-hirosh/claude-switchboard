# Custom Model Providers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users run Claude Code against third-party Anthropic-compatible endpoints by launching provider-scoped terminal sessions from Switchboard, with an opt-in global default.

**Architecture:** Providers live in Switchboard's SQLite DB. Launching writes a mode-`0700` shell/PowerShell script containing per-process `export`s and `exec claude`, then spawns the user's terminal pointed at that script — so nothing global is mutated and several providers can run concurrently. A separate opt-in path merges the same env into `~/.claude/settings.json` for bare `claude` invocations, guarded by an undo manifest, backups, and atomic writes.

**Tech Stack:** Rust (rusqlite, serde_json, anyhow, tempfile, `which`), Tauri 2.x + tauri-specta, React 19 + TypeScript, Tailwind v4 tokens, Vitest + Testing Library.

**Spec reference:** `docs/superpowers/specs/2026-07-29-custom-model-providers-design.md`

## Global Constraints

- **Never write `~/.claude/settings.json` except via `providers::default_env`.** Every write in that module backs up first, writes atomically, and records an undo manifest.
- **Secrets never appear in a process command line.** Env values go into the generated script file only; the terminal receives a path.
- **Both `collect_commands!` lists must be updated.** `src-tauri/src/lib.rs` has a `#[cfg(not(debug_assertions))]` list at ~line 159 and a `#[cfg(debug_assertions)]` list at ~line 196. A command added to only one compiles but is missing in the other build profile.
- **Schema version bumps in two places:** `create_fresh_db` (`store/mod.rs:106-112`) and the trailing stamp in `migrate()` (`store/mod.rs:159-162`). Both currently say `6`; both become `7`.
- **No hard-coded design values.** Every colour, radius, spacing and duration comes from `var(--…)` tokens, per `CLAUDE.md`.
- **Icons come from `src/lib/icons.ts` (Lucide). No emojis.**
- **Cross-platform parity is non-negotiable.** Every launcher behaviour must have a macOS and a Windows path.
- Run Rust tests with `cd src-tauri && cargo test`, frontend tests with `npm test`, type-check with `npm run lint`.

---

## File Structure

**New — Rust**

| File | Responsibility |
|---|---|
| `src-tauri/src/providers/mod.rs` | Module root, re-exports |
| `src-tauri/src/providers/model.rs` | `Provider`, `ProviderKind`, `resolved_env()` |
| `src-tauri/src/providers/store.rs` | `impl Db` CRUD + default-provider state |
| `src-tauri/src/providers/presets.rs` | Curated catalog |
| `src-tauri/src/providers/default_env.rs` | The guarded `settings.json` writer |
| `src-tauri/src/providers/launcher/mod.rs` | `Terminal`, `LaunchSpec`, spawn + sweep |
| `src-tauri/src/providers/launcher/script.rs` | Quoting + script rendering (pure) |
| `src-tauri/src/store/migrations/0007_providers.sql` | Upgrade path |

**New — TypeScript**

| File | Responsibility |
|---|---|
| `src/providers/ProvidersTab.tsx` | Tab container, list, empty state |
| `src/providers/ProviderRow.tsx` | One row: name, endpoint, model, Launch |
| `src/providers/ProviderForm.tsx` | Add/edit + preset picker |
| `src/providers/LaunchDialog.tsx` | Folder picker, terminal choice, Copy command |
| `src/providers/DefaultProviderBanner.tsx` | Visible only while a default is active |
| `src/providers/useProviders.ts` | Data hook |
| `src/providers/__tests__/*.test.tsx` | Component tests |

**Modified:** `src-tauri/src/lib.rs`, `src-tauri/src/commands.rs`, `src-tauri/src/store/mod.rs`, `src-tauri/src/store/schema.sql`, `src-tauri/src/app_state.rs`, `src-tauri/src/tray_icon/mod.rs`, `src-tauri/Cargo.toml`, `src/report/ExpandedReport.tsx`, `src/lib/ipc.ts`, `src/components/modals/SettingsModal.tsx`, `docs/release-checklist.md`

---

## Task 1: Schema, migration, and provider persistence

**Files:**
- Create: `src-tauri/src/providers/mod.rs`, `src-tauri/src/providers/model.rs`, `src-tauri/src/providers/store.rs`
- Create: `src-tauri/src/store/migrations/0007_providers.sql`
- Modify: `src-tauri/src/store/schema.sql`, `src-tauri/src/store/mod.rs`, `src-tauri/src/lib.rs`

**Interfaces:**
- Produces: `providers::model::{Provider, ProviderKind}`, `Provider::resolved_env() -> BTreeMap<String,String>`, `providers::store::DefaultProviderState`, and `impl Db` methods `list_providers`, `get_provider`, `upsert_provider`, `delete_provider`, `seed_official_provider`, `get_default_provider`, `set_default_provider`, `clear_default_provider`.

**Why:** Everything else reads from this. The `Official` row must exist before the UI can render a list, and `resolved_env()` is the single definition of "what env does this provider imply" — used by both the launcher and the settings.json writer.

- [ ] **Step 1: Add the tables to the fresh-DB schema**

Append to `src-tauri/src/store/schema.sql`:

```sql
CREATE TABLE IF NOT EXISTS providers (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    kind         TEXT NOT NULL DEFAULT 'third_party',
    base_url     TEXT,
    auth_token   TEXT,
    env_json     TEXT NOT NULL DEFAULT '{}',
    extra_args   TEXT NOT NULL DEFAULT '[]',
    preset_id    TEXT,
    sort_index   INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS provider_default (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    provider_id   TEXT REFERENCES providers(id) ON DELETE SET NULL,
    managed_env   TEXT NOT NULL DEFAULT '{}',
    applied_at    INTEGER
);
```

- [ ] **Step 2: Create the migration for existing databases**

Create `src-tauri/src/store/migrations/0007_providers.sql` with **exactly the same two `CREATE TABLE IF NOT EXISTS` statements** as Step 1. (They are idempotent, so running both on a fresh DB is harmless.)

- [ ] **Step 3: Wire the migration and bump the schema version**

In `src-tauri/src/store/mod.rs`, inside `migrate()`, after the `if current < 6 { … }` block (around line 157) insert:

```rust
        if current < 7 {
            tracing::info!("migrating v6 -> v7 (providers + provider_default tables)");
            conn.execute_batch(include_str!("migrations/0007_providers.sql"))
                .context("apply migration 0007")?;
        }
```

Then change **both** version stamps from `6` to `7`:
- `create_fresh_db` (~line 108): `[6_i64]` → `[7_i64]`
- end of `migrate()` (~line 160): `[6_i64]` → `[7_i64]`

- [ ] **Step 4: Register the module**

In `src-tauri/src/lib.rs`, alongside the other `pub mod` declarations, add:

```rust
pub mod providers;
```

Create `src-tauri/src/providers/mod.rs`:

```rust
//! Custom model providers: persistence, presets, per-process launching, and
//! the opt-in `~/.claude/settings.json` default.
//!
//! Claude Code reads `settings.json` `env` exactly once at startup and
//! `Object.assign`s it over the inherited shell environment, so a running
//! session can never adopt a provider change and a global write silently
//! overrides the user's own launch scripts. Launching with per-process env
//! is therefore the default path; `default_env` is opt-in and guarded.

pub mod default_env;
pub mod launcher;
pub mod model;
pub mod presets;
pub mod store;

pub use model::{Provider, ProviderKind};
pub use store::DefaultProviderState;
```

- [ ] **Step 5: Write the failing test for the model**

Create `src-tauri/src/providers/model.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Anthropic via the accounts subsystem. Applies no env overrides.
    Official,
    ThirdParty,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: Option<String>,
    pub auth_token: Option<String>,
    pub env: BTreeMap<String, String>,
    /// Appended to the `claude` invocation. Needed because the generated
    /// script execs the binary directly and so bypasses shell functions —
    /// e.g. a `claude()` wrapper that injects --dangerously-skip-permissions.
    pub extra_args: Vec<String>,
    pub preset_id: Option<String>,
    pub sort_index: i64,
}

pub const OFFICIAL_PROVIDER_ID: &str = "official";

impl Provider {
    /// The complete env this provider implies. `base_url` and `auth_token`
    /// are stored as dedicated columns for UI convenience but are ordinary
    /// env keys downstream; explicit `env` entries win over both.
    ///
    /// `Official` deliberately returns an empty map: the session must
    /// inherit whatever the accounts subsystem has active.
    pub fn resolved_env(&self) -> BTreeMap<String, String> {
        if self.kind == ProviderKind::Official {
            return BTreeMap::new();
        }
        let mut out = BTreeMap::new();
        if let Some(url) = self.base_url.as_deref().filter(|s| !s.is_empty()) {
            out.insert("ANTHROPIC_BASE_URL".to_string(), url.to_string());
        }
        if let Some(tok) = self.auth_token.as_deref().filter(|s| !s.is_empty()) {
            out.insert("ANTHROPIC_AUTH_TOKEN".to_string(), tok.to_string());
        }
        for (k, v) in &self.env {
            out.insert(k.clone(), v.clone());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn third_party() -> Provider {
        Provider {
            id: "p1".into(),
            name: "GLM".into(),
            kind: ProviderKind::ThirdParty,
            base_url: Some("https://api.z.ai/api/anthropic".into()),
            auth_token: Some("tok".into()),
            env: BTreeMap::from([("ANTHROPIC_MODEL".to_string(), "glm-5.2".to_string())]),
            extra_args: vec!["--dangerously-skip-permissions".to_string()],
            preset_id: Some("glm".into()),
            sort_index: 1,
        }
    }

    #[test]
    fn resolved_env_merges_columns_and_env() {
        let env = third_party().resolved_env();
        assert_eq!(env.get("ANTHROPIC_BASE_URL").map(String::as_str), Some("https://api.z.ai/api/anthropic"));
        assert_eq!(env.get("ANTHROPIC_AUTH_TOKEN").map(String::as_str), Some("tok"));
        assert_eq!(env.get("ANTHROPIC_MODEL").map(String::as_str), Some("glm-5.2"));
    }

    #[test]
    fn explicit_env_overrides_base_url_column() {
        let mut p = third_party();
        p.env.insert("ANTHROPIC_BASE_URL".into(), "https://override.example".into());
        assert_eq!(
            p.resolved_env().get("ANTHROPIC_BASE_URL").map(String::as_str),
            Some("https://override.example")
        );
    }

    #[test]
    fn official_resolves_to_empty_env() {
        let p = Provider {
            id: OFFICIAL_PROVIDER_ID.into(),
            name: "Anthropic (official)".into(),
            kind: ProviderKind::Official,
            base_url: None,
            auth_token: None,
            env: BTreeMap::new(),
            extra_args: Vec::new(),
            preset_id: None,
            sort_index: 0,
        };
        assert!(p.resolved_env().is_empty());
    }

    #[test]
    fn empty_string_credentials_are_not_emitted() {
        let mut p = third_party();
        p.auth_token = Some(String::new());
        assert!(!p.resolved_env().contains_key("ANTHROPIC_AUTH_TOKEN"));
    }
}
```

- [ ] **Step 6: Run the model tests**

```bash
cd src-tauri && cargo test --lib providers::model
```

Expected: 4 tests PASS.

- [ ] **Step 7: Write the failing store tests**

Create `src-tauri/src/providers/store.rs`:

```rust
use crate::providers::model::{Provider, ProviderKind, OFFICIAL_PROVIDER_ID};
use crate::store::Db;
use anyhow::{bail, Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What Switchboard wrote into `~/.claude/settings.json`, and what each key
/// held beforehand (`None` = the key was absent). This is both the manifest
/// of what we own and the undo record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct DefaultProviderState {
    pub provider_id: String,
    pub managed_env: BTreeMap<String, Option<String>>,
    pub applied_at: i64,
}

fn row_to_provider(row: &rusqlite::Row<'_>) -> rusqlite::Result<Provider> {
    let kind: String = row.get("kind")?;
    let env_json: String = row.get("env_json")?;
    Ok(Provider {
        id: row.get("id")?,
        name: row.get("name")?,
        kind: if kind == "official" { ProviderKind::Official } else { ProviderKind::ThirdParty },
        base_url: row.get("base_url")?,
        auth_token: row.get("auth_token")?,
        env: serde_json::from_str(&env_json).unwrap_or_default(),
        extra_args: serde_json::from_str::<Vec<String>>(&row.get::<_, String>("extra_args")?)
            .unwrap_or_default(),
        preset_id: row.get("preset_id")?,
        sort_index: row.get("sort_index")?,
    })
}

impl Db {
    pub fn list_providers(&self) -> Result<Vec<Provider>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, kind, base_url, auth_token, env_json, extra_args, preset_id, sort_index
             FROM providers ORDER BY sort_index ASC, name ASC",
        )?;
        let rows = stmt.query_map([], row_to_provider)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_provider(&self, id: &str) -> Result<Option<Provider>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, kind, base_url, auth_token, env_json, extra_args, preset_id, sort_index
             FROM providers WHERE id = ?1",
        )?;
        Ok(stmt.query_row(params![id], row_to_provider).optional()?)
    }

    pub fn upsert_provider(&self, p: &Provider) -> Result<()> {
        let kind = match p.kind {
            ProviderKind::Official => "official",
            ProviderKind::ThirdParty => "third_party",
        };
        let env_json = serde_json::to_string(&p.env).context("serialize provider env")?;
        let extra_args_json =
            serde_json::to_string(&p.extra_args).context("serialize provider extra_args")?;
        self.conn().execute(
            "INSERT INTO providers
               (id, name, kind, base_url, auth_token, env_json, extra_args, preset_id, sort_index)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               kind = excluded.kind,
               base_url = excluded.base_url,
               auth_token = excluded.auth_token,
               env_json = excluded.env_json,
               extra_args = excluded.extra_args,
               preset_id = excluded.preset_id,
               sort_index = excluded.sort_index",
            params![p.id, p.name, kind, p.base_url, p.auth_token, env_json, extra_args_json, p.preset_id, p.sort_index],
        )?;
        Ok(())
    }

    pub fn delete_provider(&self, id: &str) -> Result<()> {
        if id == OFFICIAL_PROVIDER_ID {
            bail!("the official Anthropic provider cannot be deleted");
        }
        self.conn().execute("DELETE FROM providers WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Idempotent. Runs on every startup so the row survives a manual delete
    /// from sqlite and appears in databases created before this feature.
    pub fn seed_official_provider(&self) -> Result<()> {
        self.conn().execute(
            "INSERT OR IGNORE INTO providers
               (id, name, kind, base_url, auth_token, env_json, extra_args, preset_id, sort_index)
             VALUES (?1, 'Anthropic (official)', 'official', NULL, NULL, '{}', '[]', NULL, 0)",
            params![OFFICIAL_PROVIDER_ID],
        )?;
        Ok(())
    }

    pub fn get_default_provider(&self) -> Result<Option<DefaultProviderState>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT provider_id, managed_env, applied_at FROM provider_default WHERE id = 1",
        )?;
        let row = stmt
            .query_row([], |r| {
                let provider_id: Option<String> = r.get(0)?;
                let managed_env: String = r.get(1)?;
                let applied_at: Option<i64> = r.get(2)?;
                Ok((provider_id, managed_env, applied_at))
            })
            .optional()?;
        let Some((Some(provider_id), managed_env, applied_at)) = row else {
            return Ok(None);
        };
        Ok(Some(DefaultProviderState {
            provider_id,
            managed_env: serde_json::from_str(&managed_env).unwrap_or_default(),
            applied_at: applied_at.unwrap_or(0),
        }))
    }

    pub fn set_default_provider(
        &self,
        provider_id: &str,
        managed_env: &BTreeMap<String, Option<String>>,
        applied_at: i64,
    ) -> Result<()> {
        let json = serde_json::to_string(managed_env).context("serialize managed_env")?;
        self.conn().execute(
            "INSERT INTO provider_default (id, provider_id, managed_env, applied_at)
             VALUES (1, ?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET
               provider_id = excluded.provider_id,
               managed_env = excluded.managed_env,
               applied_at = excluded.applied_at",
            params![provider_id, json, applied_at],
        )?;
        Ok(())
    }

    pub fn clear_default_provider(&self) -> Result<()> {
        self.conn().execute("DELETE FROM provider_default WHERE id = 1", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fresh() -> (tempfile::TempDir, Db) {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        db.seed_official_provider().unwrap();
        (dir, db)
    }

    fn glm() -> Provider {
        Provider {
            id: "p1".into(),
            name: "GLM".into(),
            kind: ProviderKind::ThirdParty,
            base_url: Some("https://api.z.ai/api/anthropic".into()),
            auth_token: Some("tok".into()),
            env: BTreeMap::from([("ANTHROPIC_MODEL".to_string(), "glm-5.2".to_string())]),
            extra_args: vec!["--dangerously-skip-permissions".to_string()],
            preset_id: Some("glm".into()),
            sort_index: 1,
        }
    }

    #[test]
    fn official_row_is_seeded_and_sorts_first() {
        let (_d, db) = fresh();
        db.upsert_provider(&glm()).unwrap();
        let all = db.list_providers().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, OFFICIAL_PROVIDER_ID);
        assert_eq!(all[0].kind, ProviderKind::Official);
    }

    #[test]
    fn seeding_twice_is_idempotent() {
        let (_d, db) = fresh();
        db.seed_official_provider().unwrap();
        assert_eq!(db.list_providers().unwrap().len(), 1);
    }

    #[test]
    fn upsert_roundtrips_and_updates_in_place() {
        let (_d, db) = fresh();
        db.upsert_provider(&glm()).unwrap();
        let mut p = db.get_provider("p1").unwrap().expect("provider");
        assert_eq!(p, glm());
        p.name = "GLM (edited)".into();
        db.upsert_provider(&p).unwrap();
        assert_eq!(db.get_provider("p1").unwrap().unwrap().name, "GLM (edited)");
        assert_eq!(db.list_providers().unwrap().len(), 2);
    }

    #[test]
    fn official_provider_cannot_be_deleted() {
        let (_d, db) = fresh();
        assert!(db.delete_provider(OFFICIAL_PROVIDER_ID).is_err());
        assert!(db.get_provider(OFFICIAL_PROVIDER_ID).unwrap().is_some());
    }

    #[test]
    fn third_party_provider_can_be_deleted() {
        let (_d, db) = fresh();
        db.upsert_provider(&glm()).unwrap();
        db.delete_provider("p1").unwrap();
        assert!(db.get_provider("p1").unwrap().is_none());
    }

    #[test]
    fn default_provider_roundtrips_with_null_prior_values() {
        let (_d, db) = fresh();
        db.upsert_provider(&glm()).unwrap();
        let managed = BTreeMap::from([
            ("ANTHROPIC_BASE_URL".to_string(), None),
            ("ANTHROPIC_MODEL".to_string(), Some("claude-opus-5".to_string())),
        ]);
        db.set_default_provider("p1", &managed, 1_700_000_000).unwrap();
        let state = db.get_default_provider().unwrap().expect("default state");
        assert_eq!(state.provider_id, "p1");
        assert_eq!(state.managed_env, managed);
        assert_eq!(state.applied_at, 1_700_000_000);
    }

    #[test]
    fn no_default_returns_none() {
        let (_d, db) = fresh();
        assert!(db.get_default_provider().unwrap().is_none());
    }

    #[test]
    fn clear_default_removes_state() {
        let (_d, db) = fresh();
        db.upsert_provider(&glm()).unwrap();
        db.set_default_provider("p1", &BTreeMap::new(), 1).unwrap();
        db.clear_default_provider().unwrap();
        assert!(db.get_default_provider().unwrap().is_none());
    }
}
```

- [ ] **Step 8: Run the store tests**

```bash
cd src-tauri && cargo test --lib providers::store
```

Expected: 8 tests PASS. If `Db::open` fails to find the tables, the schema change in Step 1 did not land.

- [ ] **Step 9: Add a migration test proving the upgrade path**

Append to the `tests` module in `src-tauri/src/store/mod.rs`:

```rust
    /// Mirrors `migration_0004_inserts_row_when_upgrading_from_v3`: build a
    /// pre-migration database by hand, run the migration SQL directly, and
    /// assert its effect. `Db::open` cannot be used here because it applies
    /// `schema.sql`, which already contains the tables under test.
    #[test]
    fn migration_0007_creates_provider_tables_on_upgrade() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("v6.db");
        let conn = Connection::open(&db_path).unwrap();

        // Simulate a v6 database: full schema, then drop what v7 introduces.
        conn.execute_batch(include_str!("schema.sql")).unwrap();
        conn.execute_batch("DROP TABLE providers; DROP TABLE provider_default;")
            .unwrap();
        conn.execute("INSERT OR REPLACE INTO schema_version (version) VALUES (6)", [])
            .unwrap();

        let before: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('providers','provider_default')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, 0, "precondition: the v6 database has neither table");

        conn.execute_batch(include_str!("migrations/0007_providers.sql"))
            .unwrap();

        let after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('providers','provider_default')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(after, 2, "migration 0007 must create both provider tables");
    }

    #[test]
    fn fresh_database_is_stamped_at_version_7() {
        let dir = tempdir().unwrap();
        let db = Db::open(dir.path()).unwrap();
        let version: i64 = db
            .conn()
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 7, "create_fresh_db and migrate() must both stamp 7");
    }
```

- [ ] **Step 10: Run the full store suite**

```bash
cd src-tauri && cargo test --lib store
```

Expected: all PASS, including the pre-existing migration tests.

- [ ] **Step 11: Commit**

```bash
git add src-tauri/src/providers src-tauri/src/store src-tauri/src/lib.rs
git commit -m "feat(providers): schema, migration, and provider persistence"
```

---

## Task 2: Preset catalog

**Files:**
- Create: `src-tauri/src/providers/presets.rs`

**Interfaces:**
- Consumes: `providers::model::{Provider, ProviderKind}` (Task 1).
- Produces: `presets::Preset`, `presets::all() -> &'static [Preset]`, `presets::by_id(&str) -> Option<&'static Preset>`, `Preset::to_provider(&self, id: String, auth_token: String) -> Provider`, and `presets::PresetInfo` (owned, `specta::Type`) with `presets::all_info() -> Vec<PresetInfo>`.

**Why:** Presets carry the context-window and timeout values users get wrong. Claude Code assigns any unrecognised model id a 200K window, so a 1M-token endpoint is silently under-used.

- [ ] **Step 1: Write the catalog with its tests**

Create `src-tauri/src/providers/presets.rs`:

```rust
use crate::providers::model::{Provider, ProviderKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    /// Signup / key-management page, surfaced as a link in the form.
    pub website: &'static str,
    pub env: &'static [(&'static str, &'static str)],
}

/// Serializable mirror of `Preset` for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct PresetInfo {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub website: String,
    pub env: BTreeMap<String, String>,
}

static PRESETS: &[Preset] = &[
    Preset {
        id: "glm",
        name: "GLM (z.ai)",
        base_url: "https://api.z.ai/api/anthropic",
        website: "https://z.ai",
        env: &[
            ("ANTHROPIC_MODEL", "glm-5.2"),
            ("ANTHROPIC_SMALL_FAST_MODEL", "glm-5-turbo"),
            ("ANTHROPIC_DEFAULT_OPUS_MODEL", "glm-5.2"),
            ("ANTHROPIC_DEFAULT_SONNET_MODEL", "glm-5.2"),
            ("ANTHROPIC_DEFAULT_HAIKU_MODEL", "glm-5-turbo"),
            ("CLAUDE_CODE_MAX_CONTEXT_TOKENS", "1000000"),
            ("CLAUDE_CODE_AUTO_COMPACT_WINDOW", "1000000"),
            ("API_TIMEOUT_MS", "3000000"),
        ],
    },
    Preset {
        id: "kimi",
        name: "Kimi",
        base_url: "https://api.kimi.com/coding",
        website: "https://platform.moonshot.cn",
        env: &[
            ("ANTHROPIC_MODEL", "k3"),
            ("ANTHROPIC_SMALL_FAST_MODEL", "kimi-for-coding-highspeed"),
            ("ANTHROPIC_DEFAULT_OPUS_MODEL", "k3"),
            ("ANTHROPIC_DEFAULT_SONNET_MODEL", "k3"),
            ("ANTHROPIC_DEFAULT_HAIKU_MODEL", "kimi-for-coding-highspeed"),
            ("CLAUDE_CODE_MAX_CONTEXT_TOKENS", "262144"),
            ("CLAUDE_CODE_AUTO_COMPACT_WINDOW", "262144"),
            ("API_TIMEOUT_MS", "3000000"),
        ],
    },
    Preset {
        id: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/anthropic",
        website: "https://platform.deepseek.com",
        env: &[
            ("ANTHROPIC_MODEL", "deepseek-chat"),
            ("ANTHROPIC_SMALL_FAST_MODEL", "deepseek-chat"),
            ("ANTHROPIC_DEFAULT_OPUS_MODEL", "deepseek-chat"),
            ("ANTHROPIC_DEFAULT_SONNET_MODEL", "deepseek-chat"),
            ("ANTHROPIC_DEFAULT_HAIKU_MODEL", "deepseek-chat"),
            ("CLAUDE_CODE_MAX_CONTEXT_TOKENS", "131072"),
            ("CLAUDE_CODE_AUTO_COMPACT_WINDOW", "131072"),
            ("API_TIMEOUT_MS", "3000000"),
        ],
    },
    Preset {
        id: "minimax",
        name: "MiniMax",
        base_url: "https://api.minimax.io/anthropic",
        website: "https://platform.minimax.io",
        env: &[
            ("ANTHROPIC_MODEL", "MiniMax-M2"),
            ("ANTHROPIC_SMALL_FAST_MODEL", "MiniMax-M2"),
            ("ANTHROPIC_DEFAULT_OPUS_MODEL", "MiniMax-M2"),
            ("ANTHROPIC_DEFAULT_SONNET_MODEL", "MiniMax-M2"),
            ("ANTHROPIC_DEFAULT_HAIKU_MODEL", "MiniMax-M2"),
            ("CLAUDE_CODE_MAX_CONTEXT_TOKENS", "204800"),
            ("CLAUDE_CODE_AUTO_COMPACT_WINDOW", "204800"),
            ("API_TIMEOUT_MS", "3000000"),
        ],
    },
    Preset {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        website: "https://openrouter.ai/keys",
        env: &[
            ("ANTHROPIC_MODEL", "anthropic/claude-sonnet-4.5"),
            ("ANTHROPIC_SMALL_FAST_MODEL", "anthropic/claude-haiku-4.5"),
            ("CLAUDE_CODE_MAX_CONTEXT_TOKENS", "200000"),
            ("CLAUDE_CODE_AUTO_COMPACT_WINDOW", "200000"),
            ("API_TIMEOUT_MS", "3000000"),
        ],
    },
];

pub fn all() -> &'static [Preset] {
    PRESETS
}

pub fn by_id(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

pub fn all_info() -> Vec<PresetInfo> {
    PRESETS
        .iter()
        .map(|p| PresetInfo {
            id: p.id.to_string(),
            name: p.name.to_string(),
            base_url: p.base_url.to_string(),
            website: p.website.to_string(),
            env: p.env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        })
        .collect()
}

impl Preset {
    /// Seed a provider row. The preset does not own the row afterwards —
    /// user edits persist and are never overwritten by a preset change.
    pub fn to_provider(&self, id: String, auth_token: String, sort_index: i64) -> Provider {
        Provider {
            id,
            name: self.name.to_string(),
            kind: ProviderKind::ThirdParty,
            base_url: Some(self.base_url.to_string()),
            auth_token: Some(auth_token),
            env: self.env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            extra_args: Vec::new(),
            preset_id: Some(self.id.to_string()),
            sort_index,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_sets_the_context_window_knobs() {
        for p in all() {
            let keys: Vec<&str> = p.env.iter().map(|(k, _)| *k).collect();
            assert!(
                keys.contains(&"CLAUDE_CODE_MAX_CONTEXT_TOKENS"),
                "{} must declare CLAUDE_CODE_MAX_CONTEXT_TOKENS — Claude Code defaults unknown models to 200K",
                p.id
            );
            assert!(keys.contains(&"ANTHROPIC_MODEL"), "{} must declare ANTHROPIC_MODEL", p.id);
        }
    }

    #[test]
    fn preset_ids_are_unique() {
        let mut ids: Vec<&str> = all().iter().map(|p| p.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate preset id");
    }

    #[test]
    fn base_urls_are_https_and_have_no_trailing_slash() {
        for p in all() {
            assert!(p.base_url.starts_with("https://"), "{} base_url must be https", p.id);
            assert!(!p.base_url.ends_with('/'), "{} base_url must not end in a slash", p.id);
        }
    }

    #[test]
    fn to_provider_carries_env_and_marks_preset_id() {
        let p = by_id("glm").unwrap().to_provider("uuid-1".into(), "tok".into(), 3);
        assert_eq!(p.preset_id.as_deref(), Some("glm"));
        assert_eq!(p.kind, ProviderKind::ThirdParty);
        assert_eq!(p.auth_token.as_deref(), Some("tok"));
        assert_eq!(p.sort_index, 3);
        assert_eq!(p.resolved_env().get("ANTHROPIC_MODEL").map(String::as_str), Some("glm-5.2"));
    }

    #[test]
    fn by_id_returns_none_for_unknown() {
        assert!(by_id("does-not-exist").is_none());
    }
}
```

- [ ] **Step 2: Run the preset tests**

```bash
cd src-tauri && cargo test --lib providers::presets
```

Expected: 5 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/providers/presets.rs
git commit -m "feat(providers): curated preset catalog with context-window defaults"
```

---

## Task 3: Launch-script rendering and quoting

**Files:**
- Create: `src-tauri/src/providers/launcher/script.rs`
- Create: `src-tauri/src/providers/launcher/mod.rs` (module declaration only in this task)

**Interfaces:**
- Consumes: nothing from earlier tasks (pure string functions).
- Produces: `script::ScriptFlavor` (`Sh` | `PowerShell`), `script::quote_sh(&str) -> String`, `script::quote_ps(&str) -> String`, `script::render(flavor, cwd: &str, env: &BTreeMap<String,String>, claude_path: &str, resume: Option<&str>) -> String`.

**Why:** Quoting is where a launcher gets exploited or silently corrupted. An API key containing `'` must not break out of the string, and a path containing a space must not split. These are pure functions, so they get exhaustive tests without spawning anything.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/providers/launcher/script.rs`:

```rust
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptFlavor {
    Sh,
    PowerShell,
}

/// POSIX single-quote quoting. Everything inside `'…'` is literal, so the
/// only escape needed is for `'` itself: close the quote, emit an escaped
/// quote, reopen. Handles `$`, backticks, newlines and `"` for free.
pub fn quote_sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// PowerShell single-quote quoting. Inside `'…'` no expansion occurs and a
/// literal quote is written by doubling it.
pub fn quote_ps(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub fn render(
    flavor: ScriptFlavor,
    cwd: &str,
    env: &BTreeMap<String, String>,
    claude_path: &str,
    extra_args: &[String],
    resume: Option<&str>,
) -> String {
    let q: fn(&str) -> String = match flavor {
        ScriptFlavor::Sh => quote_sh,
        ScriptFlavor::PowerShell => quote_ps,
    };

    let mut s = String::new();
    match flavor {
        ScriptFlavor::Sh => {
            s.push_str("#!/bin/sh\n# Generated by Claude Switchboard. Safe to delete.\n");
            s.push_str(&format!("cd {} || exit 1\n", q(cwd)));
            for (k, v) in env {
                s.push_str(&format!("export {}={}\n", k, q(v)));
            }
            s.push_str(&format!("exec {}", q(claude_path)));
        }
        ScriptFlavor::PowerShell => {
            s.push_str("# Generated by Claude Switchboard. Safe to delete.\n");
            s.push_str(&format!("Set-Location {}\n", q(cwd)));
            for (k, v) in env {
                s.push_str(&format!("$env:{} = {}\n", k, q(v)));
            }
            s.push_str(&format!("& {}", q(claude_path)));
        }
    }

    // Provider-supplied flags, each quoted individually — never joined into a
    // single string, which would make one argument out of several.
    for a in extra_args {
        s.push(' ');
        s.push_str(&q(a));
    }

    // `--fork-session` is not optional (Spec B §7.1): resuming a session that
    // is still open elsewhere would otherwise put two Claude Code processes on
    // the same transcript file, corrupting history that cannot be rebuilt.
    if let Some(id) = resume {
        s.push_str(&format!(" --resume {} --fork-session", q(id)));
    }

    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("ANTHROPIC_BASE_URL".to_string(), "https://api.z.ai/api/anthropic".to_string()),
            ("ANTHROPIC_MODEL".to_string(), "glm-5.2".to_string()),
        ])
    }

    #[test]
    fn sh_quoting_neutralizes_single_quotes() {
        assert_eq!(quote_sh("ab'cd"), r"'ab'\''cd'");
    }

    #[test]
    fn sh_quoting_leaves_shell_metacharacters_inert() {
        let q = quote_sh("$(rm -rf /) `whoami` \"x\" ; echo pwned");
        assert!(q.starts_with('\'') && q.ends_with('\''));
        assert!(!q.contains(r"'\''"), "no quote-breaks expected in this input");
    }

    #[test]
    fn ps_quoting_doubles_single_quotes() {
        assert_eq!(quote_ps("ab'cd"), "'ab''cd'");
    }

    #[test]
    fn sh_script_has_shebang_cd_exports_and_exec() {
        let s = render(ScriptFlavor::Sh, "/tmp/my project", &env(), "/opt/homebrew/bin/claude", &[], None);
        assert!(s.starts_with("#!/bin/sh\n"));
        assert!(s.contains("cd '/tmp/my project' || exit 1"));
        assert!(s.contains("export ANTHROPIC_MODEL='glm-5.2'"));
        assert!(s.trim_end().ends_with("exec '/opt/homebrew/bin/claude'"));
    }

    #[test]
    fn sh_script_appends_resume_flag() {
        let s = render(ScriptFlavor::Sh, "/tmp", &env(), "/usr/bin/claude", &[], Some("57ca2089-1111"));
        assert!(s.trim_end().ends_with("exec '/usr/bin/claude' --resume '57ca2089-1111' --fork-session"));
    }

    #[test]
    fn ps_script_sets_env_and_invokes_claude() {
        let s = render(
            ScriptFlavor::PowerShell,
            r"C:\Users\me\my project",
            &env(),
            r"C:\Program Files\claude.exe",
            &[],
            None,
        );
        assert!(s.contains(r"Set-Location 'C:\Users\me\my project'"));
        assert!(s.contains("$env:ANTHROPIC_MODEL = 'glm-5.2'"));
        assert!(s.trim_end().ends_with(r"& 'C:\Program Files\claude.exe'"));
    }

    #[test]
    fn injection_attempt_in_token_stays_inside_the_string() {
        let mut e = env();
        e.insert("ANTHROPIC_AUTH_TOKEN".to_string(), "x'; rm -rf ~; echo '".to_string());
        let s = render(ScriptFlavor::Sh, "/tmp", &e, "/usr/bin/claude", &[], None);
        // The dangerous text must appear only inside a quoted export line.
        let line = s.lines().find(|l| l.starts_with("export ANTHROPIC_AUTH_TOKEN=")).unwrap();
        assert_eq!(line, r"export ANTHROPIC_AUTH_TOKEN='x'\''; rm -rf ~; echo '\'''");
        assert!(!s.contains("\nrm -rf"), "payload must never start its own line");
    }

    /// Spec §3.1 — a newline in any interpolated value must not be able to
    /// start its own script line. The header comment is a fixed string, so a
    /// provider name cannot reach the script at all; this guards the values
    /// that do.
    #[test]
    fn newlines_in_values_cannot_introduce_a_script_line() {
        let mut e = env();
        e.insert("ANTHROPIC_MODEL".to_string(), "glm\nrm -rf ~/Documents\n#".to_string());
        let s = render(ScriptFlavor::Sh, "/tmp", &e, "/usr/bin/claude", &[], None);
        for line in s.lines() {
            assert!(
                !line.trim_start().starts_with("rm "),
                "payload started its own line: {line:?}"
            );
        }
        // Same for the working directory.
        let s2 = render(ScriptFlavor::Sh, "/tmp\nrm -rf ~\n", &env(), "/usr/bin/claude", &[], None);
        for line in s2.lines() {
            assert!(!line.trim_start().starts_with("rm "), "cwd escaped its quoting");
        }
    }

    #[test]
    fn generated_script_never_interpolates_a_provider_name() {
        let s = render(ScriptFlavor::Sh, "/tmp", &env(), "/usr/bin/claude", &[], None);
        assert!(
            s.contains("# Generated by Claude Switchboard. Safe to delete."),
            "header must be a fixed string, not built from provider data"
        );
    }

    /// The generated script execs the binary directly, bypassing any shell
    /// `claude()` wrapper — so flags the user relies on must be reproduced.
    #[test]
    fn extra_args_are_appended_and_quoted_individually() {
        let args = vec![
            "--dangerously-skip-permissions".to_string(),
            "--append-system-prompt".to_string(),
            "be terse; don't stop".to_string(),
        ];
        let s = render(ScriptFlavor::Sh, "/tmp", &env(), "/usr/bin/claude", &args, None);
        assert!(s.trim_end().ends_with(
            "exec '/usr/bin/claude' '--dangerously-skip-permissions' '--append-system-prompt' 'be terse; don'\\''t stop'"
        ), "got: {}", s.trim_end());
    }

    #[test]
    fn resume_always_forks_the_session() {
        let s = render(ScriptFlavor::Sh, "/tmp", &env(), "/usr/bin/claude", &[], Some("abc-123"));
        assert!(
            s.contains("--fork-session"),
            "resuming without --fork-session lets two processes share one transcript"
        );
        let ps = render(ScriptFlavor::PowerShell, "C:\\w", &env(), "claude.exe", &[], Some("abc-123"));
        assert!(ps.contains("--fork-session"), "PowerShell path must fork too");
    }

    #[test]
    fn no_fork_flag_when_not_resuming() {
        let s = render(ScriptFlavor::Sh, "/tmp", &env(), "/usr/bin/claude", &[], None);
        assert!(!s.contains("--fork-session"));
        assert!(!s.contains("--resume"));
    }

    #[test]
    fn env_order_is_deterministic() {
        let a = render(ScriptFlavor::Sh, "/tmp", &env(), "/usr/bin/claude", &[], None);
        let b = render(ScriptFlavor::Sh, "/tmp", &env(), "/usr/bin/claude", &[], None);
        assert_eq!(a, b);
    }
}
```

- [ ] **Step 2: Create the launcher module root**

Create `src-tauri/src/providers/launcher/mod.rs`:

```rust
//! Launches provider-scoped Claude Code sessions with per-process env.
//!
//! Nothing global is mutated, so several providers can run concurrently and
//! the user's own launch scripts keep working.

pub mod script;
```

- [ ] **Step 3: Run the script tests**

```bash
cd src-tauri && cargo test --lib providers::launcher::script
```

Expected: 8 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/providers/launcher
git commit -m "feat(providers): launch-script rendering with injection-safe quoting"
```

---

## Task 4: Terminal dispatch, script write, and spawn

**Files:**
- Modify: `src-tauri/src/providers/launcher/mod.rs`
- Modify: `src-tauri/Cargo.toml`

**Interfaces:**
- Consumes: `script::{ScriptFlavor, render}` (Task 3), `providers::model::Provider` (Task 1).
- Produces: `launcher::Terminal` (specta enum), `launcher::LaunchSpec`, `launcher::default_terminal() -> Terminal`, `launcher::available_terminals() -> Vec<Terminal>`, `launcher::resolve_claude_binary() -> Result<PathBuf>`, `launcher::script_dir() -> PathBuf`, `launcher::write_script(&LaunchSpec, &Path) -> Result<PathBuf>`, `launcher::build_command(Terminal, &Path, &Path) -> (String, Vec<String>)`, `launcher::launch(&LaunchSpec) -> Result<PathBuf>`, `launcher::sweep_scripts(&Path) -> Result<usize>`.

**Why:** Splitting *command construction* from *spawning* is what makes this testable — `build_command` is pure and gets asserted, `launch` is the thin spawning wrapper covered by manual smoke.

- [ ] **Step 1: Add the `which` dependency**

```bash
cd src-tauri && cargo add which
```

This resolves the `claude` binary through `PATH` on both platforms rather than hard-coding `/opt/homebrew/bin/claude`.

- [ ] **Step 2: Write the failing tests**

Replace the contents of `src-tauri/src/providers/launcher/mod.rs` with:

```rust
//! Launches provider-scoped Claude Code sessions with per-process env.
//!
//! Nothing global is mutated, so several providers can run concurrently and
//! the user's own launch scripts keep working.
//!
//! Secrets are written into a user-only script file rather than passed as
//! process arguments — command lines are world-readable via `ps` on macOS
//! and Task Manager / WMI on Windows.

pub mod script;

use crate::providers::model::Provider;
use anyhow::{anyhow, Context, Result};
use script::ScriptFlavor;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Terminal {
    Ghostty,
    TerminalApp,
    Iterm2,
    Kitty,
    WezTerm,
    WindowsTerminal,
    PowerShell,
}

impl Terminal {
    pub fn label(self) -> &'static str {
        match self {
            Terminal::Ghostty => "Ghostty",
            Terminal::TerminalApp => "Terminal.app",
            Terminal::Iterm2 => "iTerm2",
            Terminal::Kitty => "kitty",
            Terminal::WezTerm => "WezTerm",
            Terminal::WindowsTerminal => "Windows Terminal",
            Terminal::PowerShell => "PowerShell",
        }
    }

    pub fn flavor(self) -> ScriptFlavor {
        match self {
            Terminal::WindowsTerminal | Terminal::PowerShell => ScriptFlavor::PowerShell,
            _ => ScriptFlavor::Sh,
        }
    }
}

pub struct LaunchSpec {
    pub provider: Provider,
    pub cwd: PathBuf,
    pub terminal: Terminal,
    pub resume_session_id: Option<String>,
}

#[cfg(target_os = "macos")]
pub fn default_terminal() -> Terminal {
    Terminal::Ghostty
}

/// `powershell.exe`, not Windows Terminal: `wt.exe` ships with Windows 11 but
/// is a Store install on Windows 10, so defaulting to it drops a large share
/// of Win10 users into the fallback on first use. `powershell.exe` (5.1) is
/// universally present. Never `pwsh` — PowerShell 7 is a separate install.
#[cfg(not(target_os = "macos"))]
pub fn default_terminal() -> Terminal {
    Terminal::PowerShell
}

#[cfg(target_os = "macos")]
const CANDIDATES: &[Terminal] = &[
    Terminal::Ghostty,
    Terminal::TerminalApp,
    Terminal::Iterm2,
    Terminal::Kitty,
    Terminal::WezTerm,
];

#[cfg(not(target_os = "macos"))]
const CANDIDATES: &[Terminal] = &[Terminal::PowerShell, Terminal::WindowsTerminal];

#[cfg(target_os = "macos")]
fn is_installed(t: Terminal) -> bool {
    let app = match t {
        Terminal::Ghostty => "Ghostty.app",
        Terminal::TerminalApp => "Terminal.app",
        Terminal::Iterm2 => "iTerm.app",
        Terminal::Kitty => "kitty.app",
        Terminal::WezTerm => "WezTerm.app",
        _ => return false,
    };
    Path::new("/Applications").join(app).exists()
        || dirs_home().map(|h| h.join("Applications").join(app).exists()).unwrap_or(false)
}

#[cfg(not(target_os = "macos"))]
fn is_installed(t: Terminal) -> bool {
    let exe = match t {
        Terminal::WindowsTerminal => "wt.exe",
        Terminal::PowerShell => "powershell.exe",
        _ => return false,
    };
    which::which(exe).is_ok()
}

#[cfg(target_os = "macos")]
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Terminals actually present on this machine. Probed once at settings time
/// rather than at launch, so an unavailable choice surfaces before the user
/// has committed to a session.
pub fn available_terminals() -> Vec<Terminal> {
    CANDIDATES.iter().copied().filter(|t| is_installed(*t)).collect()
}

pub fn resolve_claude_binary() -> Result<PathBuf> {
    which::which("claude").map_err(|_| {
        anyhow!("could not find the `claude` executable on PATH — install Claude Code, or make sure it is on PATH for GUI apps")
    })
}

/// Generated scripts live under the app data dir, which is user-scoped on
/// both platforms (`~/Library/Application Support/...` and `%LOCALAPPDATA%`).
pub fn script_dir() -> PathBuf {
    crate::store::default_dir().join("launch")
}

pub fn write_script(spec: &LaunchSpec, dir: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).context("create launch script dir")?;
    let claude = resolve_claude_binary()?;
    let body = script::render(
        spec.terminal.flavor(),
        &spec.cwd.to_string_lossy(),
        &spec.provider.resolved_env(),
        &claude.to_string_lossy(),
        &spec.provider.extra_args,
        spec.resume_session_id.as_deref(),
    );
    let ext = match spec.terminal.flavor() {
        ScriptFlavor::Sh => "sh",
        ScriptFlavor::PowerShell => "ps1",
    };
    let path = dir.join(format!("{}.{}", uuid_v4(), ext));
    std::fs::write(&path, body).context("write launch script")?;

    // Owner-only read/write/execute. The exec bit is required because the
    // terminal runs the script directly rather than through an interpreter.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .context("restrict launch script permissions")?;
    }
    Ok(path)
}

/// `(program, args)` for spawning. Pure — asserted in tests without spawning.
pub fn build_command(terminal: Terminal, script: &Path, cwd: &Path) -> (String, Vec<String>) {
    let s = script.to_string_lossy().to_string();
    let d = cwd.to_string_lossy().to_string();
    match terminal {
        Terminal::Ghostty => (
            "open".into(),
            vec!["-na".into(), "Ghostty.app".into(), "--args".into(), "-e".into(), s],
        ),
        Terminal::TerminalApp => ("open".into(), vec!["-a".into(), "Terminal".into(), s]),
        Terminal::Iterm2 => ("open".into(), vec!["-a".into(), "iTerm".into(), s]),
        Terminal::Kitty => (
            "open".into(),
            vec!["-na".into(), "kitty.app".into(), "--args".into(), "-e".into(), s],
        ),
        Terminal::WezTerm => (
            "open".into(),
            vec!["-na".into(), "WezTerm.app".into(), "--args".into(), "start".into(), "--".into(), s],
        ),
        Terminal::WindowsTerminal => (
            "wt.exe".into(),
            vec![
                "-d".into(), d,
                "powershell.exe".into(), "-NoExit".into(), "-ExecutionPolicy".into(),
                "Bypass".into(), "-File".into(), s,
            ],
        ),
        Terminal::PowerShell => (
            "powershell.exe".into(),
            vec!["-NoExit".into(), "-ExecutionPolicy".into(), "Bypass".into(), "-File".into(), s],
        ),
    }
}

pub fn launch(spec: &LaunchSpec) -> Result<PathBuf> {
    let dir = script_dir();
    let script_path = write_script(spec, &dir)?;
    let (program, args) = build_command(spec.terminal, &script_path, &spec.cwd);
    std::process::Command::new(&program)
        .args(&args)
        .spawn()
        .with_context(|| format!("spawn {program} for {}", spec.terminal.label()))?;
    Ok(script_path)
}

/// Deletes scripts older than one hour. Called at app start **and on a
/// recurring timer** — never right after a launch, because the terminal reads
/// the file asynchronously and eager deletion races the spawn.
///
/// A startup-only sweep is insufficient: this is a menu-bar app that stays
/// resident for weeks, so token-bearing files would accumulate for the whole
/// uptime.
pub fn sweep_scripts(dir: &Path) -> Result<usize> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(0);
    };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    let mut removed = 0;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else { continue };
        if modified < cutoff && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

fn uuid_v4() -> String {
    // Matches the existing idiom in auth/oauth_paste_back.rs (rand 0.9).
    use rand::RngCore;
    let mut b = [0u8; 16];
    rand::rng().fill_bytes(&mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::model::ProviderKind;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn provider() -> Provider {
        Provider {
            id: "p1".into(),
            name: "GLM".into(),
            kind: ProviderKind::ThirdParty,
            base_url: Some("https://api.z.ai/api/anthropic".into()),
            auth_token: Some("tok".into()),
            env: BTreeMap::from([("ANTHROPIC_MODEL".to_string(), "glm-5.2".to_string())]),
            extra_args: vec!["--dangerously-skip-permissions".to_string()],
            preset_id: Some("glm".into()),
            sort_index: 1,
        }
    }

    #[test]
    fn ghostty_command_passes_script_after_dash_e() {
        let (prog, args) = build_command(
            Terminal::Ghostty,
            Path::new("/tmp/launch/a.sh"),
            Path::new("/work"),
        );
        assert_eq!(prog, "open");
        assert_eq!(args, vec!["-na", "Ghostty.app", "--args", "-e", "/tmp/launch/a.sh"]);
    }

    #[test]
    fn windows_terminal_command_sets_working_directory() {
        let (prog, args) = build_command(
            Terminal::WindowsTerminal,
            Path::new(r"C:\scripts\a.ps1"),
            Path::new(r"C:\work"),
        );
        assert_eq!(prog, "wt.exe");
        assert_eq!(args[0], "-d");
        assert_eq!(args[1], r"C:\work");
        assert_eq!(args.last().unwrap(), r"C:\scripts\a.ps1");
    }

    #[test]
    fn no_command_carries_a_secret_in_its_arguments() {
        let script = Path::new("/tmp/launch/a.sh");
        for t in [Terminal::Ghostty, Terminal::TerminalApp, Terminal::Iterm2,
                  Terminal::Kitty, Terminal::WezTerm, Terminal::WindowsTerminal,
                  Terminal::PowerShell] {
            let (prog, args) = build_command(t, script, Path::new("/work"));
            let joined = format!("{prog} {}", args.join(" "));
            assert!(!joined.contains("tok"), "{t:?} leaked a token into argv");
            assert!(!joined.contains("ANTHROPIC"), "{t:?} leaked env into argv");
        }
    }

    #[test]
    fn terminal_flavor_matches_platform_family() {
        assert_eq!(Terminal::Ghostty.flavor(), ScriptFlavor::Sh);
        assert_eq!(Terminal::WindowsTerminal.flavor(), ScriptFlavor::PowerShell);
        assert_eq!(Terminal::PowerShell.flavor(), ScriptFlavor::PowerShell);
    }

    #[test]
    fn sweep_removes_only_old_scripts() {
        let dir = tempdir().unwrap();
        let fresh = dir.path().join("fresh.sh");
        std::fs::write(&fresh, "x").unwrap();
        let stale = dir.path().join("stale.sh");
        std::fs::write(&stale, "x").unwrap();
        let two_hours_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        filetime::set_file_mtime(&stale, filetime::FileTime::from_system_time(two_hours_ago)).unwrap();

        let removed = sweep_scripts(dir.path()).unwrap();
        assert_eq!(removed, 1);
        assert!(fresh.exists(), "recent script must survive");
        assert!(!stale.exists(), "stale script must be removed");
    }

    #[test]
    fn sweep_on_missing_dir_is_not_an_error() {
        assert_eq!(sweep_scripts(Path::new("/definitely/not/here")).unwrap(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn written_script_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let spec = LaunchSpec {
            provider: provider(),
            cwd: PathBuf::from("/work"),
            terminal: Terminal::Ghostty,
            resume_session_id: None,
        };
        // Skip when Claude Code is not installed on the test machine.
        let Ok(path) = write_script(&spec, dir.path()) else { return };
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "launch script must be owner-only");
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("export ANTHROPIC_MODEL='glm-5.2'"));
        assert!(body.contains("cd '/work' || exit 1"));
    }
}
```

- [ ] **Step 3: Add the `filetime` dev-dependency used by the sweep test**

```bash
cd src-tauri && cargo add --dev filetime
```

- [ ] **Step 4: Run the launcher tests**

```bash
cd src-tauri && cargo test --lib providers::launcher
```

Expected: all PASS (script tests from Task 3 plus 7 new ones).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/providers/launcher/mod.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(providers): terminal dispatch, script write, and spawn"
```

---

## Task 5: The guarded `settings.json` default writer

**Files:**
- Create: `src-tauri/src/providers/default_env.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks (operates on a path plus an env map).
- Produces: `default_env::apply(&Path, &BTreeMap<String,String>) -> Result<BTreeMap<String,Option<String>>>`, `default_env::clear(&Path, &BTreeMap<String,Option<String>>, &BTreeMap<String,String>) -> Result<Vec<String>>` (returns keys skipped due to user drift), `default_env::foreign_provider_keys(&Path, &BTreeMap<String,Option<String>>) -> Result<Vec<String>>`, `default_env::is_provider_key(&str) -> bool`.

**Why:** This is the only code in Switchboard that writes a file the user also edits by hand — it holds their hooks, `enabledPlugins`, `statusLine` and permissions. Every guarantee here is a test.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/providers/default_env.rs`:

```rust
//! The opt-in global default: merges a provider's env into
//! `~/.claude/settings.json` so bare `claude` invocations use it.
//!
//! Claude Code applies this block with `Object.assign(process.env, …)` at
//! startup, so it OVERRIDES variables exported by the user's own launch
//! scripts. That is why this path is opt-in and loudly warned about, while
//! the launcher (which mutates nothing global) is the default.

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const MAX_BACKUPS: usize = 5;

/// Read + parse. A missing file is an empty object; a malformed file is an
/// error, never something we overwrite.
fn read_settings(path: &Path) -> Result<Map<String, Value>> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let text = std::fs::read_to_string(path).context("read settings.json")?;
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    let value: Value = serde_json::from_str(&text)
        .context("settings.json is not valid JSON — refusing to overwrite it")?;
    match value {
        Value::Object(map) => Ok(map),
        _ => anyhow::bail!("settings.json must contain a JSON object at the top level"),
    }
}

fn backup_path(path: &Path, ts: i64) -> PathBuf {
    let name = format!(
        "{}.switchboard-{ts}",
        path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "settings.json".into())
    );
    path.with_file_name(name)
}

fn backup(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    // Nanoseconds, not seconds: two writes within the same second would
    // otherwise overwrite each other's backup.
    let ts = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_else(|| chrono::Utc::now().timestamp());
    let dest = backup_path(path, ts);
    // fs::copy preserves the source mode on Unix, so a 0600 settings.json
    // yields a 0600 backup. Asserted in tests rather than assumed — the
    // backup will contain a plaintext API key once a default is set.
    std::fs::copy(path, &dest).context("back up settings.json")?;
    prune_backups(path)?;
    Ok(())
}

/// Snapshot used to detect a concurrent writer. Claude Code writes this file
/// itself (`/config`, plugin toggles, statusLine, theme), so an atomic rename
/// is not enough — it prevents a torn file, not a lost update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    mtime_ns: i128,
    len: u64,
}

fn stamp(path: &Path) -> Result<Option<FileStamp>> {
    match std::fs::metadata(path) {
        Ok(m) => {
            let mtime_ns = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i128)
                .unwrap_or(0);
            Ok(Some(FileStamp { mtime_ns, len: m.len() }))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context("stat settings.json"),
    }
}

fn prune_backups(path: &Path) -> Result<()> {
    let Some(dir) = path.parent() else { return Ok(()) };
    let prefix = format!(
        "{}.switchboard-",
        path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    );
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .context("list settings dir")?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with(&prefix))
                .unwrap_or(false)
        })
        .collect();
    found.sort();
    while found.len() > MAX_BACKUPS {
        let oldest = found.remove(0);
        let _ = std::fs::remove_file(oldest);
    }
    Ok(())
}

/// Write via a sibling temp file then rename, so a crash mid-write cannot
/// leave a truncated settings.json.
///
/// `expected` is the stamp captured when the file was read. If the file has
/// changed since, another writer (Claude Code) got there first and we abort
/// rather than clobber their change.
///
/// The temp file is created 0600 *before* any content is written. A plain
/// `fs::write` produces 0644 under a typical umask, and since we rename over
/// the target, that would silently downgrade a 0600 settings.json — in a file
/// that holds a plaintext API key once a default is set.
fn write_atomic(path: &Path, map: &Map<String, Value>, expected: Option<FileStamp>) -> Result<()> {
    if stamp(path)? != expected {
        anyhow::bail!(
            "settings.json changed while Switchboard was editing it — no changes were written. \
             Close any running Claude Code /config screen and try again."
        );
    }

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).context("create settings dir")?;
    let tmp = path.with_extension("switchboard-tmp");
    let text = serde_json::to_string_pretty(&Value::Object(map.clone()))
        .context("serialize settings.json")?;

    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).context("create temp settings.json")?;
        use std::io::Write as _;
        f.write_all(format!("{text}\n").as_bytes()).context("write temp settings.json")?;
        f.sync_all().ok();
    }

    // Carry the original mode forward when the target already existed, so we
    // never loosen (or tighten) what the user chose.
    #[cfg(unix)]
    if let Ok(meta) = std::fs::metadata(path) {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    }

    std::fs::rename(&tmp, path).context("replace settings.json")?;
    Ok(())
}

fn env_object(map: &Map<String, Value>) -> Map<String, Value> {
    map.get("env").and_then(Value::as_object).cloned().unwrap_or_default()
}

/// Merge `env` into `settings.json`. Returns the undo manifest: every key
/// written, mapped to the value it held beforehand (`None` = absent).
pub fn apply(path: &Path, env: &BTreeMap<String, String>) -> Result<BTreeMap<String, Option<String>>> {
    let before = stamp(path)?;
    let mut settings = read_settings(path)?;
    let mut env_map = env_object(&settings);

    let mut manifest = BTreeMap::new();
    for (k, v) in env {
        let prior = env_map.get(k).and_then(Value::as_str).map(str::to_string);
        manifest.insert(k.clone(), prior);
        env_map.insert(k.clone(), Value::String(v.clone()));
    }

    backup(path)?;
    settings.insert("env".to_string(), Value::Object(env_map));
    write_atomic(path, &settings, before)?;
    Ok(manifest)
}

/// Replay the manifest in reverse: keys recorded as `None` are removed,
/// keys with a recorded prior value are restored to it. Keys we never wrote
/// are untouched.
/// Returns the keys that were *skipped* because the user changed them by hand
/// while the default was active. The manifest claims we own those keys, so a
/// naive replay would silently revert the user's edit.
pub fn clear(
    path: &Path,
    manifest: &BTreeMap<String, Option<String>>,
    written: &BTreeMap<String, String>,
) -> Result<Vec<String>> {
    if manifest.is_empty() {
        return Ok(Vec::new());
    }
    let before = stamp(path)?;
    let mut settings = read_settings(path)?;
    let mut env_map = env_object(&settings);
    let mut skipped = Vec::new();

    for (k, prior) in manifest {
        // Drift check: if the current value differs from what we wrote, the
        // user edited it. Leave it alone and report it.
        let current = env_map.get(k).and_then(Value::as_str);
        if let (Some(cur), Some(ours)) = (current, written.get(k)) {
            if cur != ours {
                skipped.push(k.clone());
                continue;
            }
        }
        match prior {
            Some(v) => {
                env_map.insert(k.clone(), Value::String(v.clone()));
            }
            None => {
                env_map.remove(k);
            }
        }
    }

    backup(path)?;
    if env_map.is_empty() {
        settings.remove("env");
    } else {
        settings.insert("env".to_string(), Value::Object(env_map));
    }
    write_atomic(path, &settings, before)?;
    skipped.sort();
    Ok(skipped)
}

/// Every env key this feature can own. A prefix-only filter is wrong:
/// presets also write `API_TIMEOUT_MS`, which matches no prefix.
pub fn is_provider_key(k: &str) -> bool {
    k.starts_with("ANTHROPIC_") || k.starts_with("CLAUDE_CODE_") || k == "API_TIMEOUT_MS"
}

/// Provider keys already present that we do not own. The caller requires
/// explicit confirmation before overwriting these — they indicate hand-editing
/// or another tool such as cc-switch.
pub fn foreign_provider_keys(
    path: &Path,
    manifest: &BTreeMap<String, Option<String>>,
) -> Result<Vec<String>> {
    let settings = read_settings(path)?;
    let env_map = env_object(&settings);
    let mut out: Vec<String> = env_map
        .keys()
        .filter(|k| is_provider_key(k))
        .filter(|k| !manifest.contains_key(*k))
        .cloned()
        .collect();
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn env() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("ANTHROPIC_BASE_URL".to_string(), "https://api.z.ai/api/anthropic".to_string()),
            ("ANTHROPIC_MODEL".to_string(), "glm-5.2".to_string()),
        ])
    }

    fn write(path: &Path, s: &str) {
        std::fs::write(path, s).unwrap();
    }

    #[test]
    fn apply_creates_env_when_settings_missing() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        let manifest = apply(&p, &env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "glm-5.2");
        assert_eq!(manifest.get("ANTHROPIC_MODEL"), Some(&None));
    }

    #[test]
    fn apply_preserves_hooks_plugins_and_statusline() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{
          "hooks": {"PreToolUse": [{"matcher": "Bash"}]},
          "enabledPlugins": {"superpowers@official": true},
          "statusLine": {"type": "command", "command": "bash x.sh"},
          "model": "opus"
        }"#);
        apply(&p, &env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], "Bash");
        assert_eq!(v["enabledPlugins"]["superpowers@official"], true);
        assert_eq!(v["statusLine"]["command"], "bash x.sh");
        assert_eq!(v["model"], "opus");
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "glm-5.2");
    }

    #[test]
    fn apply_preserves_unrelated_env_entries() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"env": {"CLAUDE_CODE_RETRY_WATCHDOG": "1"}}"#);
        apply(&p, &env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["env"]["CLAUDE_CODE_RETRY_WATCHDOG"], "1");
    }

    #[test]
    fn clear_removes_keys_that_did_not_exist_before() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"env": {"CLAUDE_CODE_RETRY_WATCHDOG": "1"}}"#);
        let manifest = apply(&p, &env()).unwrap();
        clear(&p, &manifest, &env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(v["env"].get("ANTHROPIC_MODEL").is_none());
        assert_eq!(v["env"]["CLAUDE_CODE_RETRY_WATCHDOG"], "1");
    }

    #[test]
    fn clear_restores_a_preexisting_value() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"env": {"ANTHROPIC_MODEL": "claude-opus-5"}}"#);
        let manifest = apply(&p, &env()).unwrap();
        assert_eq!(manifest.get("ANTHROPIC_MODEL"), Some(&Some("claude-opus-5".to_string())));
        clear(&p, &manifest, &env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "claude-opus-5");
    }

    #[test]
    fn apply_then_clear_round_trips_to_equivalent_json() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        let original = r#"{"model":"opus","env":{"CLAUDE_CODE_RETRY_WATCHDOG":"1"},"tui":"fullscreen"}"#;
        write(&p, original);
        let before: Value = serde_json::from_str(original).unwrap();
        let manifest = apply(&p, &env()).unwrap();
        clear(&p, &manifest, &env()).unwrap();
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(before, after, "round-trip must restore the original document");
    }

    #[test]
    fn clear_drops_the_env_object_when_it_becomes_empty() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"model":"opus"}"#);
        let manifest = apply(&p, &env()).unwrap();
        clear(&p, &manifest, &env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(v.get("env").is_none(), "empty env object should be removed");
        assert_eq!(v["model"], "opus");
    }

    #[test]
    fn malformed_json_is_refused_not_overwritten() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, "{ this is not json");
        assert!(apply(&p, &env()).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{ this is not json");
    }

    #[test]
    fn non_object_settings_is_refused() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, "[1,2,3]");
        assert!(apply(&p, &env()).is_err());
    }

    #[test]
    fn unmanaged_keys_are_reported() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"env":{"ANTHROPIC_BASE_URL":"https://other","CLAUDE_CODE_MAX_CONTEXT_TOKENS":"9","THEME":"x"}}"#);
        let found = foreign_provider_keys(&p, &BTreeMap::new()).unwrap();
        assert_eq!(found, vec!["ANTHROPIC_BASE_URL", "CLAUDE_CODE_MAX_CONTEXT_TOKENS"]);
    }

    #[test]
    fn keys_we_already_manage_are_not_reported_as_unmanaged() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"env":{"ANTHROPIC_BASE_URL":"https://other"}}"#);
        let manifest = BTreeMap::from([("ANTHROPIC_BASE_URL".to_string(), None)]);
        assert!(foreign_provider_keys(&p, &manifest).unwrap().is_empty());
    }

    #[test]
    fn a_backup_is_written_before_each_change() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"model":"opus"}"#);
        apply(&p, &env()).unwrap();
        let backups: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".switchboard-"))
            .collect();
        assert_eq!(backups.len(), 1, "exactly one backup after one apply");
    }

    #[test]
    fn no_temp_file_is_left_behind() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        apply(&p, &env()).unwrap();
        assert!(!d.path().join("settings.switchboard-tmp").exists());
    }
}
```

- [ ] **Step 2: Run the default_env tests**

```bash
cd src-tauri && cargo test --lib providers::default_env
```

Expected: 13 tests PASS.

- [ ] **Step 3: Add a backup-rotation test**

Append inside the same `tests` module:

```rust
    #[test]
    fn backups_are_capped_at_five() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"model":"opus"}"#);
        // Pre-create seven backups with sortable, distinct timestamps.
        for ts in 1_700_000_000..1_700_000_007i64 {
            std::fs::write(backup_path(&p, ts), "{}").unwrap();
        }
        prune_backups(&p).unwrap();
        let count = std::fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".switchboard-"))
            .count();
        assert_eq!(count, MAX_BACKUPS);
    }
```

- [ ] **Step 4: Run it**

```bash
cd src-tauri && cargo test --lib providers::default_env
```

Expected: 14 tests PASS.

- [ ] **Step 5: Add the review-regression tests**

These cover the defects found in adversarial review (spec §4.1, §4.2, defenses #7 and #8). Append to the same `tests` module:

```rust
    fn kimi_env() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("ANTHROPIC_BASE_URL".to_string(), "https://api.kimi.com/coding".to_string()),
            ("ANTHROPIC_MODEL".to_string(), "k3".to_string()),
            ("CLAUDE_CODE_MAX_CONTEXT_TOKENS".to_string(), "262144".to_string()),
        ])
    }

    /// Spec §4.1 — the C2 regression. A → B → clear must restore the user's
    /// ORIGINAL value, not provider A's.
    #[test]
    fn switching_default_a_to_b_then_clearing_restores_the_users_own_value() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"env":{"ANTHROPIC_MODEL":"claude-opus-5"}}"#);

        // Enable A (GLM).
        let m_a = apply(&p, &env()).unwrap();
        assert_eq!(m_a.get("ANTHROPIC_MODEL"), Some(&Some("claude-opus-5".to_string())));

        // Switch to B (Kimi) the correct way: clear A first, then apply B.
        clear(&p, &m_a, &env()).unwrap();
        let m_b = apply(&p, &kimi_env()).unwrap();
        assert_eq!(
            m_b.get("ANTHROPIC_MODEL"),
            Some(&Some("claude-opus-5".to_string())),
            "B's manifest must record the USER's value, not A's"
        );

        // Clear B.
        clear(&p, &m_b, &kimi_env()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_MODEL"], "claude-opus-5");
        assert!(v["env"].get("ANTHROPIC_BASE_URL").is_none(), "no orphaned key from either provider");
        assert!(v["env"].get("CLAUDE_CODE_MAX_CONTEXT_TOKENS").is_none(), "B's extra key must not orphan");
    }

    /// Spec §4.2 — a key the user edited while the default was active is left
    /// alone on clear, and reported.
    #[test]
    fn clear_does_not_revert_a_key_the_user_edited() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        let manifest = apply(&p, &env()).unwrap();

        // User hand-edits the model while the default is active.
        let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        v["env"]["ANTHROPIC_MODEL"] = Value::String("glm-5.2-air".into());
        std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap()).unwrap();

        let skipped = clear(&p, &manifest, &env()).unwrap();
        assert_eq!(skipped, vec!["ANTHROPIC_MODEL"], "user's edit must be reported as skipped");
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(after["env"]["ANTHROPIC_MODEL"], "glm-5.2-air", "user's edit must survive");
    }

    /// Spec §4 defense #5 — a prefix-only filter passes ANTHROPIC_* and
    /// silently overwrites the rest of what presets write.
    #[test]
    fn foreign_key_detection_covers_every_preset_written_key() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"env":{
            "ANTHROPIC_MODEL":"x",
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS":"1",
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW":"2",
            "CLAUDE_CODE_SUBAGENT_MODEL":"y",
            "API_TIMEOUT_MS":"3",
            "EDITOR":"vim"
        }}"#);
        let found = foreign_provider_keys(&p, &BTreeMap::new()).unwrap();
        assert!(found.contains(&"API_TIMEOUT_MS".to_string()), "API_TIMEOUT_MS matches no prefix");
        assert!(found.contains(&"CLAUDE_CODE_SUBAGENT_MODEL".to_string()));
        assert!(!found.contains(&"EDITOR".to_string()), "unrelated keys are not ours");
        assert_eq!(found.len(), 5);
    }

    /// Spec §4 defense #7 — a concurrent writer must not be clobbered.
    #[test]
    fn write_aborts_when_the_file_changed_under_us() {
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"model":"opus"}"#);
        let stale = stamp(&p).unwrap();

        // Someone else writes — e.g. Claude Code enabling a plugin.
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(&p, r#"{"model":"opus","enabledPlugins":{"x":true}}"#);

        let mut settings = read_settings(&p).unwrap();
        settings.insert("env".into(), Value::Object(Map::new()));
        let err = write_atomic(&p, &settings, stale).unwrap_err();
        assert!(format!("{err}").contains("changed while"), "must abort, got: {err}");

        let after: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(after["enabledPlugins"]["x"], true, "the other writer's change must survive");
    }

    /// Spec §4 defense #8 — never loosen the target's permissions.
    #[cfg(unix)]
    #[test]
    fn apply_and_clear_preserve_a_0600_settings_file() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempdir().unwrap();
        let p = d.path().join("settings.json");
        write(&p, r#"{"model":"opus"}"#);
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();

        let manifest = apply(&p, &env()).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "apply must not downgrade settings.json to 0644");

        // The backup holds the same secrets and must be just as tight.
        let backup = std::fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .find(|e| e.file_name().to_string_lossy().contains(".switchboard-"))
            .expect("a backup exists");
        let bmode = backup.metadata().unwrap().permissions().mode() & 0o777;
        assert_eq!(bmode, 0o600, "backup must inherit the source mode");

        clear(&p, &manifest, &env()).unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "clear must not downgrade either");
    }
```

- [ ] **Step 6: Run the regression tests**

```bash
cd src-tauri && cargo test --lib providers::default_env
```

Expected: 19 tests PASS. `switching_default_a_to_b_then_clearing_restores_the_users_own_value` is the C2 regression and must not be skipped or weakened.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/providers/default_env.rs
git commit -m "feat(providers): guarded settings.json default writer with undo manifest"
```

---

## Task 6: Tauri commands and generated bindings

**Files:**
- Modify: `src-tauri/src/commands.rs`, `src-tauri/src/lib.rs`, `src/lib/ipc.ts`

**Interfaces:**
- Consumes: everything from Tasks 1–5.
- Produces: commands `list_providers`, `upsert_provider`, `delete_provider`, `list_provider_presets`, `launch_provider_session`, `get_provider_launch_command`, `list_available_terminals`, `get_default_provider`, `set_default_provider`, `clear_default_provider`; TS wrappers on `ipc`.
- Produces: `commands::SetDefaultOutcome` (`applied` | `needs_confirmation { unmanaged_keys }`).

**Why:** One task because the Rust command and its TS wrapper are the same deliverable — a reviewer cannot sensibly accept one without the other.

- [ ] **Step 1: Add the commands**

Append to `src-tauri/src/commands.rs`:

```rust
use crate::providers::launcher::{self, LaunchSpec, Terminal};
use crate::providers::model::Provider;
use crate::providers::presets::{self, PresetInfo};
use crate::providers::{default_env, DefaultProviderState};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum SetDefaultOutcome {
    Applied,
    /// `settings.json` already carries provider env we do not own. The UI
    /// must confirm before we overwrite hand-written configuration.
    NeedsConfirmation { unmanaged_keys: Vec<String> },
}

fn claude_settings_path() -> Result<PathBuf, String> {
    crate::auth::paths::claude_config_home()
        .map(|d| d.join("settings.json"))
        .ok_or_else(|| "could not resolve the Claude config directory".to_string())
}

#[command]
#[specta::specta]
pub async fn list_providers(state: State<'_, Arc<AppState>>) -> Result<Vec<Provider>, String> {
    state.db.list_providers().map_err(|e| e.to_string())
}

#[command]
#[specta::specta]
pub async fn upsert_provider(
    provider: Provider,
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    state.db.upsert_provider(&provider).map_err(|e| e.to_string())
}

#[command]
#[specta::specta]
pub async fn delete_provider(id: String, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    // Deleting the active default would leave settings.json holding orphaned
    // env, so undo the default first. `ON DELETE SET NULL` is only a backstop:
    // a nulled provider_id beside a populated managed_env is an orphaned
    // manifest with no UI path to undo the mutation it describes.
    if let Ok(Some(d)) = state.db.get_default_provider() {
        if d.provider_id == id {
            let path = claude_settings_path()?;
            // What we wrote is the provider's own resolved env — §4.1 requires
            // every edit to an active default to go through clear-then-apply,
            // so the stored manifest always corresponds to the current row.
            let written = state
                .db
                .get_provider(&id)
                .map_err(|e| e.to_string())?
                .map(|p| p.resolved_env())
                .unwrap_or_default();
            default_env::clear(&path, &d.managed_env, &written).map_err(|e| e.to_string())?;
            state.db.clear_default_provider().map_err(|e| e.to_string())?;
        }
    }
    state.db.delete_provider(&id).map_err(|e| e.to_string())
}

#[command]
#[specta::specta]
pub async fn list_provider_presets() -> Result<Vec<PresetInfo>, String> {
    Ok(presets::all_info())
}

#[command]
#[specta::specta]
pub async fn list_available_terminals() -> Result<Vec<Terminal>, String> {
    Ok(launcher::available_terminals())
}

#[command]
#[specta::specta]
pub async fn launch_provider_session(
    provider_id: String,
    cwd: String,
    terminal: Terminal,
    resume_session_id: Option<String>,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let provider = state
        .db
        .get_provider(&provider_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no provider with id {provider_id}"))?;
    let spec = LaunchSpec {
        provider,
        cwd: PathBuf::from(cwd),
        terminal,
        resume_session_id,
    };
    launcher::launch(&spec)
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| format!("{e:#}"))
}

/// The shell one-liner equivalent of a launch, for users whose terminal we
/// do not drive. Deliberately returns the script path rather than inlining
/// secrets into a string the user will paste somewhere.
#[command]
#[specta::specta]
pub async fn get_provider_launch_command(
    provider_id: String,
    cwd: String,
    terminal: Terminal,
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let provider = state
        .db
        .get_provider(&provider_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no provider with id {provider_id}"))?;
    let spec = LaunchSpec {
        provider,
        cwd: PathBuf::from(cwd.clone()),
        terminal,
        resume_session_id: None,
    };
    let script = launcher::write_script(&spec, &launcher::script_dir())
        .map_err(|e| format!("{e:#}"))?;
    let (program, args) = launcher::build_command(terminal, &script, &PathBuf::from(cwd));
    Ok(format!("{program} {}", args.join(" ")))
}

#[command]
#[specta::specta]
pub async fn get_default_provider(
    state: State<'_, Arc<AppState>>,
) -> Result<Option<DefaultProviderState>, String> {
    state.db.get_default_provider().map_err(|e| e.to_string())
}

#[command]
#[specta::specta]
pub async fn set_default_provider(
    provider_id: String,
    force: bool,
    state: State<'_, Arc<AppState>>,
) -> Result<SetDefaultOutcome, String> {
    let provider = state
        .db
        .get_provider(&provider_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no provider with id {provider_id}"))?;
    let path = claude_settings_path()?;
    let prev = state.db.get_default_provider().map_err(|e| e.to_string())?;

    // ORDER IS LOAD-BEARING (spec §4.1): the foreign-key check must run
    // BEFORE clear(prev), against the *previous* manifest.
    //
    // Clearing first restores the user's original values into the file, which
    // the check would then report as foreign — so every ordinary A→B switch
    // would prompt, and declining would leave the previous default already
    // undone with no record of it.
    if !force {
        let known = prev.as_ref().map(|p| p.managed_env.clone()).unwrap_or_default();
        let foreign = default_env::foreign_provider_keys(&path, &known).map_err(|e| e.to_string())?;
        if !foreign.is_empty() {
            return Ok(SetDefaultOutcome::NeedsConfirmation { unmanaged_keys: foreign });
        }
    }

    // Only now undo the previous default, so its keys cannot linger and the
    // new manifest is computed against the user's true prior state (§4.1).
    if let Some(prev) = prev {
        let prev_written = state
            .db
            .get_provider(&prev.provider_id)
            .map_err(|e| e.to_string())?
            .map(|p| p.resolved_env())
            .unwrap_or_default();
        default_env::clear(&path, &prev.managed_env, &prev_written).map_err(|e| e.to_string())?;
    }

    let env = provider.resolved_env();
    let manifest = default_env::apply(&path, &env).map_err(|e| e.to_string())?;
    state
        .db
        .set_default_provider(&provider_id, &manifest, Utc::now().timestamp())
        .map_err(|e| e.to_string())?;
    Ok(SetDefaultOutcome::Applied)
}

#[command]
#[specta::specta]
/// Returns the keys left untouched because the user edited them by hand while
/// the default was active (§4.2) — the UI reports these rather than silently
/// reverting someone's edit.
pub async fn clear_default_provider(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<String>, String> {
    let Some(d) = state.db.get_default_provider().map_err(|e| e.to_string())? else {
        return Ok(Vec::new());
    };
    let path = claude_settings_path()?;
    let written = state
        .db
        .get_provider(&d.provider_id)
        .map_err(|e| e.to_string())?
        .map(|p| p.resolved_env())
        .unwrap_or_default();
    let skipped = default_env::clear(&path, &d.managed_env, &written).map_err(|e| e.to_string())?;
    state.db.clear_default_provider().map_err(|e| e.to_string())?;
    Ok(skipped)
}
```

- [ ] **Step 2: Register the commands in BOTH `collect_commands!` lists**

In `src-tauri/src/lib.rs`, add these ten lines to the `#[cfg(not(debug_assertions))]` list (after `commands::os_scheduler_is_registered,`) **and** to the `#[cfg(debug_assertions)]` list at the same position:

```rust
            commands::list_providers,
            commands::upsert_provider,
            commands::delete_provider,
            commands::list_provider_presets,
            commands::list_available_terminals,
            commands::launch_provider_session,
            commands::get_provider_launch_command,
            commands::get_default_provider,
            commands::set_default_provider,
            commands::clear_default_provider,
```

- [ ] **Step 3: Seed the official provider and sweep old scripts at startup**

In `src-tauri/src/lib.rs`, inside the Tauri `setup` closure immediately after the `AppState` is constructed and the `Db` is available, add:

```rust
    if let Err(e) = state.db.seed_official_provider() {
        tracing::warn!("failed to seed official provider row: {e:#}");
    }
    match crate::providers::launcher::sweep_scripts(&crate::providers::launcher::script_dir()) {
        Ok(n) if n > 0 => tracing::info!("swept {n} stale launch script(s)"),
        Ok(_) => {}
        Err(e) => tracing::warn!("launch script sweep failed: {e:#}"),
    }
    // Recurring sweep — a startup-only pass leaves token-bearing scripts on
    // disk for the entire uptime of a resident menu-bar app.
    tokio::spawn(async {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1800));
        tick.tick().await; // consume the immediate first tick
        loop {
            tick.tick().await;
            if let Err(e) = crate::providers::launcher::sweep_scripts(
                &crate::providers::launcher::script_dir(),
            ) {
                tracing::warn!("periodic launch script sweep failed: {e:#}");
            }
        }
    });
```

- [ ] **Step 4: Build and regenerate bindings**

```bash
cd src-tauri && cargo build
```

Expected: clean build. The debug build re-exports `src/lib/generated/bindings.ts` via the specta exporter. Confirm the new commands appear:

```bash
grep -c "listProviders\|setDefaultProvider\|launchProviderSession" src/lib/generated/bindings.ts
```

Expected: `3` or more. If `0`, the `setup` closure did not run the exporter — run `npm run tauri dev` once, stop it, and re-check.

- [ ] **Step 5: Add the TS wrappers**

In `src/lib/ipc.ts`, add inside the `ipc` object (before the closing brace):

```ts
  // Providers pillar
  listProviders: () => commands.listProviders().then(unwrap),
  upsertProvider: (p: import('./generated/bindings').Provider) =>
    commands.upsertProvider(p).then(unwrap),
  deleteProvider: (id: string) => commands.deleteProvider(id).then(unwrap),
  listProviderPresets: () => commands.listProviderPresets().then(unwrap),
  listAvailableTerminals: () => commands.listAvailableTerminals().then(unwrap),
  launchProviderSession: (
    providerId: string,
    cwd: string,
    terminal: import('./generated/bindings').Terminal,
    resumeSessionId: string | null = null,
  ) => commands.launchProviderSession(providerId, cwd, terminal, resumeSessionId).then(unwrap),
  getProviderLaunchCommand: (
    providerId: string,
    cwd: string,
    terminal: import('./generated/bindings').Terminal,
  ) => commands.getProviderLaunchCommand(providerId, cwd, terminal).then(unwrap),
  getDefaultProvider: () => commands.getDefaultProvider().then(unwrap),
  setDefaultProvider: (providerId: string, force: boolean) =>
    commands.setDefaultProvider(providerId, force).then(unwrap),
  /** Resolves to the keys left untouched because the user edited them (spec §4.2). */
  clearDefaultProvider: () => commands.clearDefaultProvider().then(unwrap),
```

- [ ] **Step 6: Type-check and run the whole Rust suite**

```bash
npm run lint && cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings
```

Expected: all clean.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs src/lib/ipc.ts src/lib/generated/bindings.ts
git commit -m "feat(providers): tauri commands and typed bindings"
```

---

## Task 7: Providers tab — list, rows, and launch

**Files:**
- Create: `src/providers/useProviders.ts`, `src/providers/ProviderRow.tsx`, `src/providers/ProvidersTab.tsx`, `src/providers/LaunchDialog.tsx`
- Create: `src/providers/__tests__/ProviderRow.test.tsx`, `src/providers/__tests__/ProvidersTab.test.tsx`
- Modify: `src/report/ExpandedReport.tsx`

**Interfaces:**
- Consumes: `ipc.listProviders`, `ipc.launchProviderSession`, `ipc.getProviderLaunchCommand`, `ipc.listAvailableTerminals` (Task 6).
- Produces: `useProviders()` returning `{ providers, loading, error, reload }`; `<ProviderRow provider onLaunch onEdit onDelete />`; `<ProvidersTab />`.

**Why:** This is the first user-visible deliverable — a reviewer can run the app and launch a session.

- [ ] **Step 1: Write the failing row test**

Create `src/providers/__tests__/ProviderRow.test.tsx`:

```tsx
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import type { Provider } from '../../lib/generated/bindings';
import { ProviderRow } from '../ProviderRow';

function glm(): Provider {
  return {
    id: 'p1',
    name: 'GLM',
    kind: 'third_party',
    base_url: 'https://api.z.ai/api/anthropic',
    auth_token: 'tok',
    env: { ANTHROPIC_MODEL: 'glm-5.2' },
    extra_args: [],
    preset_id: 'glm',
    sort_index: 1,
  };
}

function official(): Provider {
  return {
    id: 'official',
    name: 'Anthropic (official)',
    kind: 'official',
    base_url: null,
    auth_token: null,
    env: {},
    extra_args: [],
    preset_id: null,
    sort_index: 0,
  };
}

describe('ProviderRow', () => {
  it('shows the name, host and model', () => {
    render(<ProviderRow provider={glm()} onLaunch={vi.fn()} onEdit={vi.fn()} onDelete={vi.fn()} />);
    expect(screen.getByText('GLM')).toBeTruthy();
    expect(screen.getByText(/api\.z\.ai/)).toBeTruthy();
    expect(screen.getByText('glm-5.2')).toBeTruthy();
  });

  it('never renders the auth token', () => {
    const { container } = render(
      <ProviderRow provider={glm()} onLaunch={vi.fn()} onEdit={vi.fn()} onDelete={vi.fn()} />,
    );
    expect(container.textContent).not.toContain('tok');
  });

  it('calls onLaunch with the provider id', () => {
    const onLaunch = vi.fn();
    render(<ProviderRow provider={glm()} onLaunch={onLaunch} onEdit={vi.fn()} onDelete={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: /launch/i }));
    expect(onLaunch).toHaveBeenCalledWith('p1');
  });

  it('offers no delete control for the official provider', () => {
    render(<ProviderRow provider={official()} onLaunch={vi.fn()} onEdit={vi.fn()} onDelete={vi.fn()} />);
    expect(screen.queryByRole('button', { name: /delete/i })).toBeNull();
  });

  it('describes the official provider as using the active account', () => {
    render(<ProviderRow provider={official()} onLaunch={vi.fn()} onEdit={vi.fn()} onDelete={vi.fn()} />);
    expect(screen.getByText(/active account/i)).toBeTruthy();
  });
});
```

- [ ] **Step 2: Run it — must fail**

```bash
npm test -- src/providers/__tests__/ProviderRow.test.tsx
```

Expected: FAIL — `Failed to resolve import "../ProviderRow"`.

- [ ] **Step 3: Implement `ProviderRow`**

Create `src/providers/ProviderRow.tsx`:

```tsx
import type { Provider } from '../lib/generated/bindings';
import { Button } from '../components/ui/Button';
import { IconButton } from '../components/ui/IconButton';
import { Play, Pencil, Trash2 } from '../lib/icons';

interface Props {
  provider: Provider;
  onLaunch: (id: string) => void;
  onEdit: (id: string) => void;
  onDelete: (id: string) => void;
}

function hostOf(url: string | null): string {
  if (!url) return '';
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export function ProviderRow({ provider, onLaunch, onEdit, onDelete }: Props) {
  const isOfficial = provider.kind === 'official';
  const model = provider.env['ANTHROPIC_MODEL'] ?? null;

  return (
    <div
      className="
        flex items-center gap-[var(--space-sm)]
        rounded-[var(--radius-sm)] border border-[var(--color-border)]
        bg-[var(--color-bg-card)]
        px-[var(--space-sm)] py-[var(--space-xs)]
      "
    >
      <div className="flex min-w-0 flex-1 flex-col gap-[var(--space-2xs)]">
        <span className="truncate text-[length:var(--text-body)] text-[color:var(--color-text)]">
          {provider.name}
        </span>
        <span className="truncate text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          {isOfficial ? 'Uses your active account' : hostOf(provider.base_url)}
        </span>
      </div>

      {model && (
        <span className="mono shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]">
          {model}
        </span>
      )}

      <Button variant="primary" size="sm" onClick={() => onLaunch(provider.id)} aria-label={`Launch ${provider.name}`}>
        <Play size={13} aria-hidden />
        Launch
      </Button>

      {!isOfficial && (
        <>
          {/* IconButton takes a required `label` prop and applies it as
              aria-label itself — do not pass aria-label directly. */}
          <IconButton label={`Edit ${provider.name}`} onClick={() => onEdit(provider.id)}>
            <Pencil size={14} aria-hidden />
          </IconButton>
          <IconButton label={`Delete ${provider.name}`} onClick={() => onDelete(provider.id)}>
            <Trash2 size={14} aria-hidden />
          </IconButton>
        </>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Export the icons used above**

In `src/lib/icons.ts`, add to the existing re-exports:

```ts
export { Play, Pencil, Trash2, FolderOpen, Terminal as TerminalIcon, Plus } from 'lucide-react';
```

(If any of these are already exported there, do not duplicate the name — leave the existing export in place.)

- [ ] **Step 5: Run the row test — must pass**

```bash
npm test -- src/providers/__tests__/ProviderRow.test.tsx
```

Expected: 5 tests PASS.

- [ ] **Step 6: Implement the data hook**

Create `src/providers/useProviders.ts`:

```ts
import { useCallback, useEffect, useState } from 'react';
import type { Provider } from '../lib/generated/bindings';
import { ipc } from '../lib/ipc';

export function useProviders() {
  const [providers, setProviders] = useState<Provider[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      setProviders(await ipc.listProviders());
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  return { providers, loading, error, reload };
}
```

- [ ] **Step 7: Write the failing tab test**

Create `src/providers/__tests__/ProvidersTab.test.tsx`:

```tsx
import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';

const ipcMock = vi.hoisted(() => ({
  listProviders: vi.fn(),
  listAvailableTerminals: vi.fn().mockResolvedValue(['ghostty']),
  listProviderPresets: vi.fn().mockResolvedValue([]),
  launchProviderSession: vi.fn().mockResolvedValue('/tmp/launch/a.sh'),
  getProviderLaunchCommand: vi.fn().mockResolvedValue('open -na Ghostty.app'),
  getDefaultProvider: vi.fn().mockResolvedValue(null),
  upsertProvider: vi.fn().mockResolvedValue(undefined),
  deleteProvider: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('../../lib/ipc', () => ({ ipc: ipcMock }));

const dialogMock = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => dialogMock);

import { ProvidersTab } from '../ProvidersTab';

const official = {
  id: 'official', name: 'Anthropic (official)', kind: 'official',
  base_url: null, auth_token: null, env: {}, extra_args: [], preset_id: null, sort_index: 0,
};
const glm = {
  id: 'p1', name: 'GLM', kind: 'third_party',
  base_url: 'https://api.z.ai/api/anthropic', auth_token: 'tok',
  env: { ANTHROPIC_MODEL: 'glm-5.2' }, extra_args: [], preset_id: 'glm', sort_index: 1,
};

describe('ProvidersTab', () => {
  beforeEach(() => vi.clearAllMocks());

  it('lists providers returned by the backend', async () => {
    ipcMock.listProviders.mockResolvedValue([official, glm]);
    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText('GLM')).toBeTruthy());
    expect(screen.getByText('Anthropic (official)')).toBeTruthy();
  });

  it('shows an empty state when only the official provider exists', async () => {
    ipcMock.listProviders.mockResolvedValue([official]);
    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText(/add a provider/i)).toBeTruthy());
  });

  it('surfaces a backend error instead of rendering an empty list', async () => {
    ipcMock.listProviders.mockRejectedValue(new Error('db is locked'));
    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText(/db is locked/)).toBeTruthy());
  });

  it('launches with the folder chosen in the picker', async () => {
    ipcMock.listProviders.mockResolvedValue([official, glm]);
    dialogMock.open.mockResolvedValue('/Users/me/work');
    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText('GLM')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /launch glm/i }));
    await waitFor(() =>
      expect(ipcMock.launchProviderSession).toHaveBeenCalledWith('p1', '/Users/me/work', 'ghostty', null),
    );
  });

  it('does not launch when the folder picker is cancelled', async () => {
    ipcMock.listProviders.mockResolvedValue([official, glm]);
    dialogMock.open.mockResolvedValue(null);
    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText('GLM')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /launch glm/i }));
    await waitFor(() => expect(ipcMock.launchProviderSession).not.toHaveBeenCalled());
  });
});
```

- [ ] **Step 8: Run it — must fail**

```bash
npm test -- src/providers/__tests__/ProvidersTab.test.tsx
```

Expected: FAIL — cannot resolve `../ProvidersTab`.

- [ ] **Step 9: Implement `ProvidersTab`**

Create `src/providers/ProvidersTab.tsx`:

```tsx
import { useCallback, useEffect, useState } from 'react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import type { Terminal } from '../lib/generated/bindings';
import { ipc } from '../lib/ipc';
import { useProviders } from './useProviders';
import { ProviderRow } from './ProviderRow';
import { ProviderForm } from './ProviderForm';
import { Button } from '../components/ui/Button';
import { Plus } from '../lib/icons';

export function ProvidersTab() {
  const { providers, loading, error, reload } = useProviders();
  const [terminal, setTerminal] = useState<Terminal | null>(null);
  const [editing, setEditing] = useState<string | 'new' | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    void ipc.listAvailableTerminals().then((ts) => setTerminal(ts[0] ?? null));
  }, []);

  const handleLaunch = useCallback(
    async (id: string) => {
      const dir = await openDialog({ directory: true, multiple: false, title: 'Choose a folder' });
      if (typeof dir !== 'string') return;
      if (!terminal) {
        setNotice('No supported terminal found. Install Ghostty or Windows Terminal.');
        return;
      }
      try {
        await ipc.launchProviderSession(id, dir, terminal, null);
        setNotice(null);
      } catch (e) {
        setNotice(e instanceof Error ? e.message : String(e));
      }
    },
    [terminal],
  );

  const handleDelete = useCallback(
    async (id: string) => {
      await ipc.deleteProvider(id);
      await reload();
    },
    [reload],
  );

  const thirdParty = providers.filter((p) => p.kind !== 'official');

  return (
    <div className="flex flex-col gap-[var(--space-sm)] p-[var(--space-md)]">
      {error && (
        <div
          role="alert"
          className="rounded-[var(--radius-sm)] border border-[var(--color-danger)] bg-[var(--color-danger-dim)] px-[var(--space-sm)] py-[var(--space-2xs)] text-[length:var(--text-micro)]"
        >
          {error}
        </div>
      )}
      {notice && (
        <div
          role="status"
          className="rounded-[var(--radius-sm)] border border-[var(--color-warn)] bg-[var(--color-warn-dim)] px-[var(--space-sm)] py-[var(--space-2xs)] text-[length:var(--text-micro)]"
        >
          {notice}
        </div>
      )}

      {!loading &&
        providers.map((p) => (
          <ProviderRow
            key={p.id}
            provider={p}
            onLaunch={handleLaunch}
            onEdit={setEditing}
            onDelete={handleDelete}
          />
        ))}

      {!loading && !error && thirdParty.length === 0 && (
        <p className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          Add a provider to run Claude Code against a custom endpoint.
        </p>
      )}

      <div>
        <Button variant="ghost" size="sm" onClick={() => setEditing('new')}>
          <Plus size={13} aria-hidden />
          Add provider
        </Button>
      </div>

      {editing && (
        <ProviderForm
          providerId={editing === 'new' ? null : editing}
          onClose={() => setEditing(null)}
          onSaved={async () => {
            setEditing(null);
            await reload();
          }}
        />
      )}
    </div>
  );
}
```

- [ ] **Step 10: Mount the tab**

In `src/report/ExpandedReport.tsx`:

1. Add the import beside the other tab imports:
```tsx
import { ProvidersTab } from '../providers/ProvidersTab';
```
2. Append to `TAB_CONFIG`:
```tsx
  { id: 'providers', label: 'Providers' },
```
3. Append to `TAB_COMPONENTS`:
```tsx
  providers: ProvidersTab,
```

- [ ] **Step 11: Run the tab tests**

Task 7 depends on `ProviderForm`, which Task 8 creates. To keep this task independently runnable, create a minimal placeholder now and replace it in Task 8:

```tsx
// src/providers/ProviderForm.tsx — replaced in Task 8
export function ProviderForm(_: {
  providerId: string | null;
  onClose: () => void;
  onSaved: () => void | Promise<void>;
}) {
  return null;
}
```

```bash
npm test -- src/providers && npm run lint
```

Expected: all PASS, type-check clean.

- [ ] **Step 12: Commit**

```bash
git add src/providers src/report/ExpandedReport.tsx src/lib/icons.ts
git commit -m "feat(providers): providers tab with rows and folder-picker launch"
```

---

## Task 8: Provider form with preset picker

**Files:**
- Modify: `src/providers/ProviderForm.tsx` (replacing the Task 7 placeholder)
- Create: `src/providers/__tests__/ProviderForm.test.tsx`

**Interfaces:**
- Consumes: `ipc.listProviderPresets`, `ipc.listProviders`, `ipc.upsertProvider` (Task 6); `ModalShell` from `src/components/modals/ModalShell.tsx`.
- Produces: `<ProviderForm providerId={string|null} onClose onSaved />`.

**Why:** Choosing a preset must populate the context-window knobs, because that is the value the catalog exists to deliver.

- [ ] **Step 1: Write the failing tests**

Create `src/providers/__tests__/ProviderForm.test.tsx`:

```tsx
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';

const ipcMock = vi.hoisted(() => ({
  listProviderPresets: vi.fn().mockResolvedValue([
    {
      id: 'glm',
      name: 'GLM (z.ai)',
      base_url: 'https://api.z.ai/api/anthropic',
      website: 'https://z.ai',
      env: { ANTHROPIC_MODEL: 'glm-5.2', CLAUDE_CODE_MAX_CONTEXT_TOKENS: '1000000' },
    },
  ]),
  listProviders: vi.fn().mockResolvedValue([]),
  upsertProvider: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('../../lib/ipc', () => ({ ipc: ipcMock }));

import { ProviderForm } from '../ProviderForm';

describe('ProviderForm', () => {
  beforeEach(() => vi.clearAllMocks());

  it('prefills base URL and model when a preset is chosen', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/preset/i)).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/preset/i), { target: { value: 'glm' } });
    await waitFor(() =>
      expect((screen.getByLabelText(/base url/i) as HTMLInputElement).value).toBe(
        'https://api.z.ai/api/anthropic',
      ),
    );
    expect((screen.getByLabelText(/^model/i) as HTMLInputElement).value).toBe('glm-5.2');
  });

  it('carries the preset context-window knobs into the saved provider', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/preset/i)).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/preset/i), { target: { value: 'glm' } });
    fireEvent.change(screen.getByLabelText(/api key/i), { target: { value: 'sk-test' } });
    await waitFor(() =>
      expect((screen.getByLabelText(/base url/i) as HTMLInputElement).value).toBeTruthy(),
    );
    fireEvent.click(screen.getByRole('button', { name: /save/i }));
    await waitFor(() => expect(ipcMock.upsertProvider).toHaveBeenCalled());
    const saved = ipcMock.upsertProvider.mock.calls[0][0];
    expect(saved.env.CLAUDE_CODE_MAX_CONTEXT_TOKENS).toBe('1000000');
    expect(saved.auth_token).toBe('sk-test');
    expect(saved.kind).toBe('third_party');
  });

  it('splits extra CLI arguments into separate argv entries', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/preset/i)).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/preset/i), { target: { value: 'glm' } });
    fireEvent.change(screen.getByLabelText(/api key/i), { target: { value: 'sk-test' } });
    fireEvent.change(screen.getByLabelText(/extra cli arguments/i), {
      target: { value: '--dangerously-skip-permissions  --verbose' },
    });
    await waitFor(() =>
      expect((screen.getByLabelText(/base url/i) as HTMLInputElement).value).toBeTruthy(),
    );
    fireEvent.click(screen.getByRole('button', { name: /save/i }));
    await waitFor(() => expect(ipcMock.upsertProvider).toHaveBeenCalled());
    expect(ipcMock.upsertProvider.mock.calls[0][0].extra_args).toEqual([
      '--dangerously-skip-permissions',
      '--verbose',
    ]);
  });

  it('refuses to save without a name and a base URL', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/preset/i)).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /save/i }));
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
    expect(ipcMock.upsertProvider).not.toHaveBeenCalled();
  });

  it('rejects a base URL that is not http(s)', async () => {
    render(<ProviderForm providerId={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    await waitFor(() => expect(screen.getByLabelText(/name/i)).toBeTruthy());
    fireEvent.change(screen.getByLabelText(/name/i), { target: { value: 'X' } });
    fireEvent.change(screen.getByLabelText(/base url/i), { target: { value: 'ftp://nope' } });
    fireEvent.click(screen.getByRole('button', { name: /save/i }));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toMatch(/https/i));
    expect(ipcMock.upsertProvider).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run — must fail**

```bash
npm test -- src/providers/__tests__/ProviderForm.test.tsx
```

Expected: FAIL — the placeholder renders `null`, so no fields exist.

- [ ] **Step 3: Implement the form**

Replace `src/providers/ProviderForm.tsx` with:

```tsx
import { useEffect, useMemo, useState } from 'react';
import type { PresetInfo, Provider } from '../lib/generated/bindings';
import { ipc } from '../lib/ipc';
import { ModalShell } from '../components/modals/ModalShell';
import { Button } from '../components/ui/Button';

interface Props {
  providerId: string | null;
  onClose: () => void;
  onSaved: () => void | Promise<void>;
}

const inputClass =
  'w-full rounded-[var(--radius-sm)] border border-[var(--color-border)] bg-[var(--color-bg-base)] px-[var(--space-xs)] py-[var(--space-2xs)] text-[length:var(--text-body)] text-[color:var(--color-text)]';

function newId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `p-${Date.now()}`;
}

export function ProviderForm({ providerId, onClose, onSaved }: Props) {
  const [presets, setPresets] = useState<PresetInfo[]>([]);
  const [presetId, setPresetId] = useState('');
  const [name, setName] = useState('');
  const [baseUrl, setBaseUrl] = useState('');
  const [token, setToken] = useState('');
  const [model, setModel] = useState('');
  const [extraArgs, setExtraArgs] = useState('');
  const [env, setEnv] = useState<Record<string, string>>({});
  const [existing, setExisting] = useState<Provider | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    void ipc.listProviderPresets().then(setPresets);
  }, []);

  useEffect(() => {
    if (!providerId) return;
    void ipc.listProviders().then((all) => {
      const p = all.find((x) => x.id === providerId);
      if (!p) return;
      setExisting(p);
      setName(p.name);
      setBaseUrl(p.base_url ?? '');
      setToken(p.auth_token ?? '');
      setModel(p.env['ANTHROPIC_MODEL'] ?? '');
      setExtraArgs(p.extra_args.join(' '));
      setEnv(p.env);
      setPresetId(p.preset_id ?? '');
    });
  }, [providerId]);

  function applyPreset(id: string) {
    setPresetId(id);
    const p = presets.find((x) => x.id === id);
    if (!p) return;
    setName(p.name);
    setBaseUrl(p.base_url);
    setEnv(p.env);
    setModel(p.env['ANTHROPIC_MODEL'] ?? '');
  }

  const title = useMemo(() => (providerId ? 'Edit provider' : 'Add provider'), [providerId]);

  async function save() {
    if (!name.trim() || !baseUrl.trim()) {
      setError('Name and base URL are both required.');
      return;
    }
    if (!/^https?:\/\//i.test(baseUrl.trim())) {
      setError('Base URL must start with https:// (or http:// for a local endpoint).');
      return;
    }
    setSaving(true);
    try {
      const merged = { ...env };
      if (model.trim()) merged['ANTHROPIC_MODEL'] = model.trim();
      const provider: Provider = {
        id: existing?.id ?? providerId ?? newId(),
        name: name.trim(),
        kind: 'third_party',
        base_url: baseUrl.trim(),
        auth_token: token,
        env: merged,
        // Whitespace-split is deliberate: each token becomes its own argv
        // entry, quoted separately by the script renderer.
        extra_args: extraArgs.trim() ? extraArgs.trim().split(/\s+/) : [],
        preset_id: presetId || null,
        sort_index: existing?.sort_index ?? Date.now() % 100000,
      };
      await ipc.upsertProvider(provider);
      await onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    {/* ModalShell's dismiss prop is `onDismiss`, and `id` is required —
        it keys the modal in the app store. */}
    <ModalShell id="provider-form" title={title} onDismiss={onClose}>
      <div className="flex flex-col gap-[var(--space-sm)]">
        {error && (
          <div
            role="alert"
            className="rounded-[var(--radius-sm)] border border-[var(--color-danger)] bg-[var(--color-danger-dim)] px-[var(--space-sm)] py-[var(--space-2xs)] text-[length:var(--text-micro)]"
          >
            {error}
          </div>
        )}

        <label className="flex flex-col gap-[var(--space-2xs)] text-[length:var(--text-micro)]">
          Preset
          <select
            aria-label="Preset"
            className={inputClass}
            value={presetId}
            onChange={(e) => applyPreset(e.target.value)}
          >
            <option value="">Custom</option>
            {presets.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </label>

        <label className="flex flex-col gap-[var(--space-2xs)] text-[length:var(--text-micro)]">
          Name
          <input aria-label="Name" className={inputClass} value={name} onChange={(e) => setName(e.target.value)} />
        </label>

        <label className="flex flex-col gap-[var(--space-2xs)] text-[length:var(--text-micro)]">
          Base URL
          <input
            aria-label="Base URL"
            className={inputClass}
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder="https://api.example.com/anthropic"
          />
        </label>

        <label className="flex flex-col gap-[var(--space-2xs)] text-[length:var(--text-micro)]">
          API key
          <input
            aria-label="API key"
            type="password"
            className={inputClass}
            value={token}
            onChange={(e) => setToken(e.target.value)}
          />
        </label>

        <label className="flex flex-col gap-[var(--space-2xs)] text-[length:var(--text-micro)]">
          Model
          <input aria-label="Model" className={inputClass} value={model} onChange={(e) => setModel(e.target.value)} />
        </label>

        <label className="flex flex-col gap-[var(--space-2xs)] text-[length:var(--text-micro)]">
          Extra CLI arguments
          <input
            aria-label="Extra CLI arguments"
            className={inputClass}
            value={extraArgs}
            onChange={(e) => setExtraArgs(e.target.value)}
            placeholder="--dangerously-skip-permissions"
          />
          <span className="text-[color:var(--color-text-muted)]">
            Passed to <code className="mono">claude</code> on launch. The script runs the binary
            directly, so shell aliases and functions do not apply.
          </span>
        </label>

        <p className="text-[length:var(--text-micro)] text-[color:var(--color-text-muted)]">
          {Object.keys(env).length} environment variable{Object.keys(env).length === 1 ? '' : 's'} will be set for
          sessions launched with this provider.
        </p>

        <div className="flex justify-end gap-[var(--space-xs)]">
          <Button variant="ghost" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" size="sm" onClick={save} disabled={saving}>
            Save
          </Button>
        </div>
      </div>
    </ModalShell>
  );
}
```

- [ ] **Step 4: Run the form tests**

```bash
npm test -- src/providers/__tests__/ProviderForm.test.tsx
```

Expected: 4 tests PASS.

`ModalShell` registers itself in the app store under `id`, so the test may need the store to be initialised. If the render throws on `useAppStore`, mock it alongside `ipc` in the test file:

```tsx
vi.mock('../../lib/store', () => ({
  useAppStore: (sel: (s: unknown) => unknown) => sel({ modals: {}, openModal: vi.fn(), closeModal: vi.fn() }),
}));
```

- [ ] **Step 5: Run everything and type-check**

```bash
npm test && npm run lint
```

Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add src/providers
git commit -m "feat(providers): provider form with preset prefill and validation"
```

---

## Task 9: Global default toggle, warning, and banner

**Files:**
- Create: `src/providers/DefaultProviderBanner.tsx`, `src/providers/__tests__/DefaultProviderBanner.test.tsx`
- Modify: `src/providers/ProvidersTab.tsx`, `src/providers/ProviderRow.tsx`

**Interfaces:**
- Consumes: `ipc.getDefaultProvider`, `ipc.setDefaultProvider`, `ipc.clearDefaultProvider` (Task 6).
- Produces: `<DefaultProviderBanner providerName onClear />`; `ProviderRow` gains `isDefault: boolean` and `onSetDefault: (id: string) => void`.

**Why:** This is the loud path. The warning must name the actual consequence — that it overrides the user's own launch scripts — because that is the failure mode they would otherwise hit.

- [ ] **Step 1: Write the failing banner test**

Create `src/providers/__tests__/DefaultProviderBanner.test.tsx`:

```tsx
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { DefaultProviderBanner } from '../DefaultProviderBanner';

describe('DefaultProviderBanner', () => {
  it('names the provider and warns about shell overrides', () => {
    render(<DefaultProviderBanner providerName="GLM" onClear={vi.fn()} />);
    expect(screen.getByText(/GLM/)).toBeTruthy();
    expect(screen.getByText(/overrides/i)).toBeTruthy();
  });

  it('warns that running sessions are unaffected', () => {
    render(<DefaultProviderBanner providerName="GLM" onClear={vi.fn()} />);
    expect(screen.getByText(/already running/i)).toBeTruthy();
  });

  it('calls onClear when the user turns the default off', () => {
    const onClear = vi.fn();
    render(<DefaultProviderBanner providerName="GLM" onClear={onClear} />);
    fireEvent.click(screen.getByRole('button', { name: /turn off/i }));
    expect(onClear).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run — must fail**

```bash
npm test -- src/providers/__tests__/DefaultProviderBanner.test.tsx
```

Expected: FAIL — module not found.

- [ ] **Step 3: Implement the banner**

Create `src/providers/DefaultProviderBanner.tsx`:

```tsx
interface Props {
  providerName: string;
  onClear: () => void;
}

export function DefaultProviderBanner({ providerName, onClear }: Props) {
  return (
    <div
      role="status"
      className="
        flex items-start gap-[var(--space-sm)]
        rounded-[var(--radius-sm)] border border-[var(--color-warn)]
        bg-[var(--color-warn-dim)]
        px-[var(--space-sm)] py-[var(--space-2xs)]
      "
    >
      <div className="flex-1 text-[length:var(--text-micro)] text-[color:var(--color-text-secondary)]">
        <div className="text-[color:var(--color-text)]">
          <strong>{providerName}</strong> is the default for every Claude Code session.
        </div>
        <div>
          This overrides <code className="mono">ANTHROPIC_*</code> variables exported by your shell, so your own
          launch scripts stop taking effect. Sessions already running keep their current provider until restarted.
        </div>
      </div>
      <button
        type="button"
        onClick={onClear}
        className="shrink-0 text-[length:var(--text-micro)] text-[color:var(--color-accent)] hover:underline"
      >
        Turn off
      </button>
    </div>
  );
}
```

- [ ] **Step 4: Run — must pass**

```bash
npm test -- src/providers/__tests__/DefaultProviderBanner.test.tsx
```

Expected: 3 tests PASS.

- [ ] **Step 5: Add `isDefault` / `onSetDefault` to `ProviderRow`**

In `src/providers/ProviderRow.tsx`, extend the `Props` interface:

```tsx
interface Props {
  provider: Provider;
  isDefault?: boolean;
  onLaunch: (id: string) => void;
  onEdit: (id: string) => void;
  onDelete: (id: string) => void;
  onSetDefault?: (id: string) => void;
}
```

Update the destructure to `({ provider, isDefault = false, onLaunch, onEdit, onDelete, onSetDefault }: Props)` and insert this immediately before the `<Button variant="primary" …>Launch</Button>` element:

```tsx
      {onSetDefault && !isOfficial && (
        <button
          type="button"
          onClick={() => onSetDefault(provider.id)}
          aria-label={`Set ${provider.name} as default`}
          className={[
            'shrink-0 text-[length:var(--text-micro)] hover:underline',
            isDefault
              ? 'text-[color:var(--color-warn)]'
              : 'text-[color:var(--color-text-muted)]',
          ].join(' ')}
        >
          {isDefault ? 'Default' : 'Set default'}
        </button>
      )}
```

- [ ] **Step 6: Wire the default flow into `ProvidersTab`**

In `src/providers/ProvidersTab.tsx`:

1. Add to the imports:
```tsx
import { DefaultProviderBanner } from './DefaultProviderBanner';
import type { DefaultProviderState } from '../lib/generated/bindings';
```
2. Add state and a loader after the existing `useState` declarations:
```tsx
  const [defaultState, setDefaultState] = useState<DefaultProviderState | null>(null);

  const reloadDefault = useCallback(async () => {
    setDefaultState(await ipc.getDefaultProvider());
  }, []);

  useEffect(() => {
    void reloadDefault();
  }, [reloadDefault]);

  const handleSetDefault = useCallback(
    async (id: string) => {
      const outcome = await ipc.setDefaultProvider(id, false);
      if (outcome.status === 'needs_confirmation') {
        const keys = outcome.unmanaged_keys.join(', ');
        const ok = window.confirm(
          `~/.claude/settings.json already sets ${keys}. Switchboard did not write these — another tool or a manual edit did.\n\nOverwrite them?`,
        );
        if (!ok) return;
        await ipc.setDefaultProvider(id, true);
      }
      await reloadDefault();
    },
    [reloadDefault],
  );

  const handleClearDefault = useCallback(async () => {
    // Returns keys left untouched because the user hand-edited them while the
    // default was active (spec §4.2). Silently reverting someone's edit would
    // be worse than leaving it, so say what was skipped.
    const skipped = await ipc.clearDefaultProvider();
    if (skipped.length > 0) {
      setNotice(
        `Default turned off. Left your own edits to ${skipped.join(', ')} in place.`,
      );
    }
    await reloadDefault();
  }, [reloadDefault]);
```
3. Render the banner directly above the provider list:
```tsx
      {defaultState && (
        <DefaultProviderBanner
          providerName={providers.find((p) => p.id === defaultState.provider_id)?.name ?? 'A provider'}
          onClear={handleClearDefault}
        />
      )}
```
4. Pass the two new props into `<ProviderRow …>`:
```tsx
            isDefault={defaultState?.provider_id === p.id}
            onSetDefault={handleSetDefault}
```

- [ ] **Step 7: Add a tab-level test for the confirmation path**

Append to `src/providers/__tests__/ProvidersTab.test.tsx` inside the existing `describe`:

```tsx
  it('asks for confirmation before overwriting unmanaged settings keys', async () => {
    ipcMock.listProviders.mockResolvedValue([official, glm]);
    ipcMock.setDefaultProvider = vi
      .fn()
      .mockResolvedValueOnce({ status: 'needs_confirmation', unmanaged_keys: ['ANTHROPIC_BASE_URL'] })
      .mockResolvedValueOnce({ status: 'applied' });
    ipcMock.clearDefaultProvider = vi.fn().mockResolvedValue([]);
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);

    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText('GLM')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /set glm as default/i }));

    await waitFor(() => expect(confirmSpy).toHaveBeenCalled());
    await waitFor(() => expect(ipcMock.setDefaultProvider).toHaveBeenCalledWith('p1', true));
    confirmSpy.mockRestore();
  });

  it('does not force the write when the user declines confirmation', async () => {
    ipcMock.listProviders.mockResolvedValue([official, glm]);
    ipcMock.setDefaultProvider = vi
      .fn()
      .mockResolvedValue({ status: 'needs_confirmation', unmanaged_keys: ['ANTHROPIC_BASE_URL'] });
    ipcMock.clearDefaultProvider = vi.fn().mockResolvedValue([]);
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(false);

    render(<ProvidersTab />);
    await waitFor(() => expect(screen.getByText('GLM')).toBeTruthy());
    fireEvent.click(screen.getByRole('button', { name: /set glm as default/i }));

    await waitFor(() => expect(confirmSpy).toHaveBeenCalled());
    expect(ipcMock.setDefaultProvider).toHaveBeenCalledTimes(1);
    confirmSpy.mockRestore();
  });
```

Also add `getDefaultProvider`, `setDefaultProvider` and `clearDefaultProvider` to the `ipcMock` object at the top of that file if they are not already present:

```tsx
  setDefaultProvider: vi.fn().mockResolvedValue({ status: 'applied' }),
  clearDefaultProvider: vi.fn().mockResolvedValue([]),   // returns drift-skipped keys
```

- [ ] **Step 8: Run all provider tests**

```bash
npm test -- src/providers && npm run lint
```

Expected: all PASS.

- [ ] **Step 9: Commit**

```bash
git add src/providers
git commit -m "feat(providers): global default toggle with confirmation and banner"
```

---

## Task 10: Tray marker while a default is active

**Files:**
- Modify: `src-tauri/src/tray_icon/mod.rs`, `src-tauri/src/commands.rs`

**Interfaces:**
- Consumes: `Db::get_default_provider` (Task 1).
- Produces: an updated tray tooltip that names the active default provider.

**Why:** A global default is the one state in which the tray's usage bars answer a question the user is no longer asking. Sessions launched from the Providers tab get no marker — they are explicit acts in visible windows and several may run at once.

- [ ] **Step 1: Find the tooltip construction**

```bash
grep -rn "set_tooltip\|tooltip" src-tauri/src/tray_icon/ src-tauri/src/tray.rs | head
```

Note the function that builds the tooltip string — the next step edits it.

- [ ] **Step 2: Append the provider suffix to the tooltip**

In the function identified in Step 1, after the existing tooltip string is assembled and before it is passed to `set_tooltip`, insert:

```rust
    // A global provider default means the usage bars no longer describe where
    // the user's work is going. Name it rather than let the bars mislead.
    let tooltip = match state.db.get_default_provider() {
        Ok(Some(d)) => {
            let name = state
                .db
                .get_provider(&d.provider_id)
                .ok()
                .flatten()
                .map(|p| p.name)
                .unwrap_or_else(|| "a custom provider".to_string());
            format!("{tooltip}\nDefault provider: {name}")
        }
        _ => tooltip,
    };
```

If the local variable is not named `tooltip`, rename the binding in the snippet to match — do not rename the existing variable.

- [ ] **Step 3: Verify it compiles and nothing regressed**

```bash
cd src-tauri && cargo build && cargo test && cargo clippy --all-targets -- -D warnings
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/tray_icon src-tauri/src/commands.rs
git commit -m "feat(tray): name the active default provider in the tooltip"
```

---

## Task 11: Terminal preference in settings

**Files:**
- Modify: `src-tauri/src/app_state.rs`, `src-tauri/src/commands.rs`, `src/components/modals/SettingsModal.tsx`, `src/providers/ProvidersTab.tsx`

**Interfaces:**
- Consumes: `launcher::{Terminal, available_terminals, default_terminal}` (Task 4), `ipc.getSettings` / `ipc.updateSettings` (existing).
- Produces: `Settings.terminal: Option<Terminal>`; `ProvidersTab` launches with the configured terminal instead of the first available one.

**Why:** Without this the launcher always uses whichever terminal happens to sort first, which is wrong for anyone whose default is not Ghostty.

- [ ] **Step 1: Add the field**

In `src-tauri/src/app_state.rs`, add to the `Settings` struct (after `preferred_auth_source`):

```rust
    /// `None` means "use the platform default" (`launcher::default_terminal()`).
    /// `#[serde(default)]` keeps settings written before this field readable.
    #[serde(default)]
    pub terminal: Option<crate::providers::launcher::Terminal>,
```

And in `impl Default for Settings`, add:

```rust
            terminal: None,
```

- [ ] **Step 2: Verify persistence round-trips old settings**

Add to the tests module in `src-tauri/src/app_state.rs` (create one if absent):

```rust
#[cfg(test)]
mod settings_tests {
    use super::Settings;

    #[test]
    fn settings_without_terminal_field_still_deserialize() {
        let json = r#"{
            "polling_interval_secs": 300,
            "stagger_gap_secs": 30,
            "thresholds": [75, 90],
            "theme": "system",
            "launch_at_login": false,
            "crash_reports": false,
            "preferred_auth_source": null
        }"#;
        let s: Settings = serde_json::from_str(json).expect("legacy settings must still parse");
        assert!(s.terminal.is_none());
    }
}
```

- [ ] **Step 3: Run it**

```bash
cd src-tauri && cargo test --lib app_state
```

Expected: PASS. If it fails with "missing field", the `#[serde(default)]` attribute is not on the new field.

- [ ] **Step 4: Use the setting when launching**

In `src/providers/ProvidersTab.tsx`, replace the terminal-loading effect with:

```tsx
  useEffect(() => {
    void (async () => {
      const [settings, available] = await Promise.all([
        ipc.getSettings(),
        ipc.listAvailableTerminals(),
      ]);
      const configured = settings.terminal;
      setTerminal(configured && available.includes(configured) ? configured : (available[0] ?? null));
    })();
  }, []);
```

- [ ] **Step 5: Add the picker to settings**

In `src/components/modals/SettingsModal.tsx`, add a select bound to `settings.terminal`, populated from `ipc.listAvailableTerminals()`, following the file's existing field pattern (label + `Select` from `src/components/ui/Select.tsx`, writing through the same `updateSettings` handler the other fields use). Label it **Terminal**, with a `null` option labelled **System default**.

- [ ] **Step 6: Run everything**

```bash
npm test && npm run lint && cd src-tauri && cargo test
```

Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/app_state.rs src/components/modals/SettingsModal.tsx src/providers/ProvidersTab.tsx
git commit -m "feat(providers): terminal preference setting"
```

---

## Task 12: Release checklist and final verification

**Files:**
- Modify: `docs/release-checklist.md`, `CHANGELOG.md`

- [ ] **Step 1: Add the smoke items**

Append to `docs/release-checklist.md`:

```markdown
## Custom model providers (added 2026-07-29)

Run on **both macOS and Windows**.

- [ ] Providers tab lists `Anthropic (official)` first; it has no delete control
- [ ] Add a provider from the GLM preset — base URL, model and `CLAUDE_CODE_MAX_CONTEXT_TOKENS` prefill
- [ ] Launch it against a project folder; the terminal opens in that folder and `/status` reports the custom endpoint
- [ ] Launch a second provider while the first is still running; both sessions work independently
- [ ] Launch `Anthropic (official)`; the session uses the currently active managed account
- [ ] `ps aux | grep -i ghostty` (macOS) shows **no** API key in the command line
- [ ] Generated scripts under the app data dir are mode `0700` and are swept on next app start
- [ ] Enable "Set default" on a provider; the banner appears and names it
- [ ] With the default on, run `claude` by hand — it uses the default provider
- [ ] `~/.claude/settings.json` still contains your hooks, `enabledPlugins`, `statusLine` and `model`
- [ ] A `settings.json.switchboard-<ts>` backup exists; no more than 5 accumulate
- [ ] Turn the default off; `settings.json` returns to its previous content and a pre-existing hook still fires
- [ ] Set a default while `settings.json` already has a hand-written `ANTHROPIC_BASE_URL` — the confirmation prompt lists it
- [ ] **Switch default A → B → off** (spec §4.1). Hand-set `ANTHROPIC_MODEL` first; after turning off, that value must be back and neither provider's keys may remain
- [ ] **Decline the confirmation on a switch** — the previous default must still be fully in effect afterwards
- [ ] **Hand-edit a managed key while a default is active**, then turn the default off — your edit survives and the UI names it
- [ ] **Concurrency**: with the app open, run `/config` in Claude Code and toggle something, then set a default — either it succeeds or it reports the file changed; the `/config` change is never lost
- [ ] **Permissions**: `stat -f '%Sp' ~/.claude/settings.json` reads `-rw-------` before and after enabling and clearing a default; backups are `-rw-------` too
- [ ] Delete a provider that is currently the default; `settings.json` is cleaned up first
- [ ] Tray tooltip names the default provider while one is set, and omits the line when none is
- [ ] **Windows**: launch works on a machine with neither `pwsh` nor `wt.exe` installed, and with ExecutionPolicy at its `Restricted` default
```

- [ ] **Step 2: Add the changelog entry**

Add to the `## Unreleased` section of `CHANGELOG.md` (create the section if absent):

```markdown
### Added
- **Custom model providers.** Run Claude Code against third-party Anthropic-compatible endpoints (GLM, Kimi, DeepSeek, MiniMax, OpenRouter, or a custom URL) by launching provider-scoped terminal sessions from the new Providers tab. Sessions use per-process environment variables, so several providers can run at once and your own launch scripts keep working.
- **Optional global default.** A provider can additionally be set as the default for bare `claude` invocations. This writes `~/.claude/settings.json` and is guarded by a backup, an undo manifest and a confirmation prompt — it overrides `ANTHROPIC_*` variables exported by your shell.
```

- [ ] **Step 3: Full verification**

```bash
cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings
cd .. && npm test && npm run build
```

Expected: all green. Do not proceed if anything fails.

- [ ] **Step 4: Commit**

```bash
git add docs/release-checklist.md CHANGELOG.md
git commit -m "docs: release checklist and changelog for custom model providers"
```

- [ ] **Step 5: Manual smoke before declaring done**

Run `npm run tauri dev` and work through the entire checklist block added in Step 1 on the current platform. Anything that fails re-opens the relevant task — do not declare the feature complete on a partial pass.

---

## Self-Review Notes

**Spec coverage**

| Spec section | Tasks |
|---|---|
| §2 Data model | 1 |
| §3.1 Script generation + secret hygiene | 3, 4 |
| §3.2 Terminal dispatch (macOS + Windows) | 4, 11 |
| §4 Optional global default | 5, 6, 9 |
| §5 Presets | 2 |
| §6 UI | 7, 8, 9 |
| §6.1 Tray | 10 |
| §7 Errors | 5 (malformed JSON, unmanaged keys), 6 (missing provider, delete-while-default), 7 (no terminal, launch failure) |
| §8 $0 third-party cost | documented only — no code, per the spec's non-goal |
| §11 Testing | inline across all tasks; manual smoke in 12 |

**Deliberate deviations from the spec, and why**

1. **`cmd` is dropped from the Windows terminal list.** An earlier spec revision listed it, but every generated Windows script is PowerShell, so a `cmd` entry would only shell out to PowerShell anyway. Dropping it removes a code path with no user-visible benefit. (Script mode `0700` and `powershell.exe` over `pwsh` were deviations in the first draft of this plan; the spec has since been corrected to match, so they are no longer deviations.)
2. **Providers are a tab in `ExpandedReport`, not a bespoke section.** The spec says "a section in the expanded window"; `ExpandedReport` implements sections as tabs (`TAB_CONFIG` / `TAB_COMPONENTS`), so a tab *is* the existing mechanism. The popover remains untouched as specified.

**Adversarial-review findings incorporated (2026-07-29)**

| Finding | Where it lands |
|---|---|
| C1 — a `0600` script cannot be exec'd (exit 126) | Task 4 writes `0700` and asserts it |
| C2 — `managed_env` destroyed on A → B switch | Task 6 clear-then-apply; Task 5 Step 5 regression test |
| C2b — foreign-key check ran *after* `clear`, so every switch prompted and declining left the default undone | Task 6 `set_default_provider`, ordering comment marks it load-bearing |
| C3 — third-party pricing is partial and wrong in both directions, not `$0` | Spec §8 rewritten; no plan code asserts `$0` |
| H2 — no concurrency guard against Claude Code's own writes | Task 5 `FileStamp` + `write_atomic(…, expected)`; regression test |
| H3 — key filter missed `API_TIMEOUT_MS` | Task 5 `is_provider_key`; regression test |
| H4 — `fs::write` + rename downgraded `0600` → `0644` | Task 5 `write_atomic` opens the temp at `0600` and carries the target's mode forward; regression test |
| H5 — `pwsh` absent, ExecutionPolicy `Restricted`, `wt.exe` absent on Win10 | Task 4 defaults to `powershell.exe` with `-ExecutionPolicy Bypass` |
| M3 — newline injection through interpolated strings | Task 3 tests; header comment is a fixed string |
| M5 — startup-only sweep leaks scripts on a resident app | Task 6 Step 3 adds a 30-minute recurring sweep |
| M6 — macOS `open` exits 0 even when the launch fails | Spec §7; no plan step claims exit status detects it |
| Low — backup collision at second granularity | Task 5 uses `timestamp_nanos_opt()` |

**Rejected, with evidence** — `PRAGMA foreign_keys = ON` *is* present (`store/schema.sql:2`) and `schema.sql` is `execute_batch`'d on every connection open including existing healthy DBs (`store/mod.rs:78`), so the `REFERENCES` clause in Task 1 is enforced rather than decorative; and the claim that `baseURL:vA` is absent from the v2.1.220 bundle was a mis-grep — the text is `baseURL:e=vA("ANTHROPIC_BASE_URL")`, a default parameter value.

**Type consistency check**

`Provider` is defined once in Task 1 (`id, name, kind, base_url, auth_token, env, preset_id, sort_index`) and used with those exact field names in Tasks 2, 4, 6, 7 and 8. `DefaultProviderState` (`provider_id`, `managed_env`, `applied_at`) is defined in Task 1 and consumed in Tasks 6, 9 and 10. `Terminal` is defined in Task 4 and consumed in Tasks 6, 7 and 11. `ScriptFlavor` is defined in Task 3 and consumed in Task 4. `default_env::{apply, clear, foreign_provider_keys, is_provider_key}` are defined in Task 5 with the signatures Task 6 calls; `clear` takes the written-env map as its third argument and returns the drift-skipped keys. `SetDefaultOutcome` is defined in Task 6 and its `status` discriminant is matched in Task 9's `handleSetDefault`.

**Known forward reference**

Task 7 imports `ProviderForm`, which Task 8 implements. Task 7 Step 11 creates a null-rendering placeholder so the task's own tests pass in isolation; Task 8 Step 3 replaces it. This is the only forward dependency in the plan.

**Deferred to Spec B**

The session browser, model-badge → provider mapping, and one-click resume. `launcher::LaunchSpec.resume_session_id` and the `--resume` rendering in `script::render` are already implemented and tested here, so Spec B is largely UI.
