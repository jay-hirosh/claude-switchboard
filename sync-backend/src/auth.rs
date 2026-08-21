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
