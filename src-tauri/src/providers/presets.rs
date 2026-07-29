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
            ("ANTHROPIC_DEFAULT_FABLE_MODEL", "glm-5.2"),
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
            ("ANTHROPIC_DEFAULT_FABLE_MODEL", "k3"),
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
            ("ANTHROPIC_DEFAULT_FABLE_MODEL", "deepseek-chat"),
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
            ("ANTHROPIC_DEFAULT_FABLE_MODEL", "MiniMax-M2"),
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
            // Pinned even though OpenRouter proxies Anthropic: an unset alias
            // falls back to a bare `claude-…` id, and OpenRouter needs the
            // `anthropic/` vendor prefix.
            ("ANTHROPIC_DEFAULT_OPUS_MODEL", "anthropic/claude-opus-4.5"),
            ("ANTHROPIC_DEFAULT_FABLE_MODEL", "anthropic/claude-sonnet-4.5"),
            ("ANTHROPIC_DEFAULT_SONNET_MODEL", "anthropic/claude-sonnet-4.5"),
            ("ANTHROPIC_DEFAULT_HAIKU_MODEL", "anthropic/claude-haiku-4.5"),
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
            env: p
                .env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
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
            env: self
                .env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
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
            assert!(
                keys.contains(&"ANTHROPIC_MODEL"),
                "{} must declare ANTHROPIC_MODEL",
                p.id
            );
        }
    }

    // Every `/model <alias>` that is left unset falls back to a first-party
    // Anthropic id, which a third-party endpoint cannot serve — so a preset
    // that pins only ANTHROPIC_MODEL leaves `/model opus` broken. The quick
    // model is separate again: Claude Code reads ANTHROPIC_SMALL_FAST_MODEL
    // first and only then the haiku alias.
    #[test]
    fn every_preset_pins_every_model_alias() {
        for p in all() {
            let keys: Vec<&str> = p.env.iter().map(|(k, _)| *k).collect();
            for required in [
                "ANTHROPIC_DEFAULT_OPUS_MODEL",
                "ANTHROPIC_DEFAULT_SONNET_MODEL",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL",
                "ANTHROPIC_DEFAULT_FABLE_MODEL",
                "ANTHROPIC_SMALL_FAST_MODEL",
            ] {
                assert!(
                    keys.contains(&required),
                    "{} must declare {required} — an unset alias resolves to a \
                     first-party Anthropic id this endpoint cannot serve",
                    p.id
                );
            }
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
            assert!(
                p.base_url.starts_with("https://"),
                "{} base_url must be https",
                p.id
            );
            assert!(
                !p.base_url.ends_with('/'),
                "{} base_url must not end in a slash",
                p.id
            );
        }
    }

    #[test]
    fn to_provider_carries_env_and_marks_preset_id() {
        let p = by_id("glm")
            .unwrap()
            .to_provider("uuid-1".into(), "tok".into(), 3);
        assert_eq!(p.preset_id.as_deref(), Some("glm"));
        assert_eq!(p.kind, ProviderKind::ThirdParty);
        assert_eq!(p.auth_token.as_deref(), Some("tok"));
        assert_eq!(p.sort_index, 3);
        assert_eq!(
            p.resolved_env().get("ANTHROPIC_MODEL").map(String::as_str),
            Some("glm-5.2")
        );
    }

    #[test]
    fn by_id_returns_none_for_unknown() {
        assert!(by_id("does-not-exist").is_none());
    }
}
