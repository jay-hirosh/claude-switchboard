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
        kind: if kind == "official" {
            ProviderKind::Official
        } else {
            ProviderKind::ThirdParty
        },
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

    /// The official row is identity, not configuration: it must always resolve
    /// to whatever the accounts subsystem has active. Its `extra_args` are the
    /// user's to set — that is the whole point of editing it — but its kind and
    /// credentials are pinned here rather than trusted from the caller, so no
    /// UI slip can turn "Anthropic (official)" into a third-party provider
    /// pointing at somebody else's endpoint while keeping the official name.
    fn pin_official(p: &Provider) -> Provider {
        let mut out = p.clone();
        if out.id == OFFICIAL_PROVIDER_ID {
            out.kind = ProviderKind::Official;
            out.base_url = None;
            out.auth_token = None;
        }
        out
    }

    pub fn upsert_provider(&self, p: &Provider) -> Result<()> {
        let p = &Self::pin_official(p);
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
            params![
                p.id,
                p.name,
                kind,
                p.base_url,
                p.auth_token,
                env_json,
                extra_args_json,
                p.preset_id,
                p.sort_index
            ],
        )?;
        Ok(())
    }

    pub fn delete_provider(&self, id: &str) -> Result<()> {
        if id == OFFICIAL_PROVIDER_ID {
            bail!("the official Anthropic provider cannot be deleted");
        }
        self.conn()
            .execute("DELETE FROM providers WHERE id = ?1", params![id])?;
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
        self.conn()
            .execute("DELETE FROM provider_default WHERE id = 1", [])?;
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

    /// Editing the official provider is how a user adds launch flags to it, so
    /// the write has to go through — the flags are the point.
    #[test]
    fn official_provider_accepts_extra_args() {
        let (_d, db) = fresh();
        let mut official = db.get_provider(OFFICIAL_PROVIDER_ID).unwrap().unwrap();
        official.extra_args = vec!["--dangerously-skip-permissions".to_string()];
        db.upsert_provider(&official).unwrap();

        let back = db.get_provider(OFFICIAL_PROVIDER_ID).unwrap().unwrap();
        assert_eq!(back.extra_args, vec!["--dangerously-skip-permissions"]);
        assert_eq!(back.kind, ProviderKind::Official);
    }

    /// A UI slip must not be able to turn the official row into a third-party
    /// endpoint that still carries the official name — the session would
    /// silently run against somebody else's API.
    #[test]
    fn official_provider_cannot_be_turned_into_a_third_party_one() {
        let (_d, db) = fresh();
        let hijacked = Provider {
            id: OFFICIAL_PROVIDER_ID.into(),
            name: "Anthropic (official)".into(),
            kind: ProviderKind::ThirdParty,
            base_url: Some("https://evil.example".into()),
            auth_token: Some("tok".into()),
            env: BTreeMap::new(),
            extra_args: vec!["--continue".to_string()],
            preset_id: None,
            sort_index: 0,
        };
        db.upsert_provider(&hijacked).unwrap();

        let back = db.get_provider(OFFICIAL_PROVIDER_ID).unwrap().unwrap();
        assert_eq!(back.kind, ProviderKind::Official);
        assert_eq!(back.base_url, None);
        assert_eq!(back.auth_token, None);
        assert!(back.resolved_env().is_empty(), "must still inherit the account");
        // The one field that is legitimately the user's still lands.
        assert_eq!(back.extra_args, vec!["--continue"]);
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
            (
                "ANTHROPIC_MODEL".to_string(),
                Some("claude-opus-5".to_string()),
            ),
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
