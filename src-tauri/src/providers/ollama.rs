//! Model discovery for a local Ollama server, backing the suggestion
//! dropdown in the provider form's Model field.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct TagsResponse {
    models: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    name: String,
}

/// Model names from an Ollama `/api/tags` response body, sorted for stable
/// display order.
fn parse_tags(body: &str) -> Result<Vec<String>, String> {
    let parsed: TagsResponse = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let mut names: Vec<String> = parsed.models.into_iter().map(|m| m.name).collect();
    names.sort();
    Ok(names)
}

/// Fetches installed model names from a local Ollama server. `base_url` is
/// the provider's own base URL (e.g. `http://localhost:11434`) — the same
/// value used for `ANTHROPIC_BASE_URL` — not a separate setting.
pub async fn fetch_models(client: &reqwest::Client, base_url: &str) -> Result<Vec<String>, String> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Ollama returned HTTP {}", resp.status()));
    }
    let body = resp.text().await.map_err(|e| e.to_string())?;
    parse_tags(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_model_names_sorted() {
        let body = r#"{"models":[{"name":"llama3.2:latest"},{"name":"gpt-oss:20b"}]}"#;
        assert_eq!(
            parse_tags(body).unwrap(),
            vec!["gpt-oss:20b".to_string(), "llama3.2:latest".to_string()]
        );
    }

    #[test]
    fn empty_model_list_is_not_an_error() {
        assert_eq!(parse_tags(r#"{"models":[]}"#).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn malformed_json_is_an_error() {
        assert!(parse_tags("not json").is_err());
    }

    #[test]
    fn missing_models_key_is_an_error() {
        assert!(parse_tags(r#"{"unrelated":true}"#).is_err());
    }
}
