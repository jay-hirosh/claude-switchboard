use sha2::{Digest, Sha256};

/// SHA-256 hex digest of a raw API key. High-entropy random tokens (not
/// user-chosen secrets) don't need a slow hash — this matches how e.g.
/// GitHub stores personal access tokens.
pub fn hash_api_key(raw: &str) -> String {
    format!("{:x}", Sha256::digest(raw.as_bytes()))
}

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
}
