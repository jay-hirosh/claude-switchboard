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
        assert_eq!(
            env.get("ANTHROPIC_BASE_URL").map(String::as_str),
            Some("https://api.z.ai/api/anthropic")
        );
        assert_eq!(env.get("ANTHROPIC_AUTH_TOKEN").map(String::as_str), Some("tok"));
        assert_eq!(env.get("ANTHROPIC_MODEL").map(String::as_str), Some("glm-5.2"));
    }

    #[test]
    fn explicit_env_overrides_base_url_column() {
        let mut p = third_party();
        p.env
            .insert("ANTHROPIC_BASE_URL".into(), "https://override.example".into());
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
