//! ID generation and hashing utilities

use sha2::{Digest, Sha256};

/// Generate a prefixed ID using UUID v7 (time-ordered)
pub fn generate_id(prefix: &str) -> String {
    format!("{}_{}", prefix, uuid::Uuid::now_v7().simple())
}

/// Generate a session ID
pub fn generate_session_id() -> String {
    generate_id("sess")
}

/// Generate an event ID
pub fn generate_event_id() -> String {
    generate_id("evt")
}

/// Generate a range ID
pub fn generate_range_id() -> String {
    generate_id("rng")
}

/// Generate a context set ID
pub fn generate_context_set_id() -> String {
    generate_id("ctx")
}

/// Generate a provenance bundle ID
pub fn generate_bundle_id() -> String {
    generate_id("bundle")
}

/// Compute SHA-256 hash of a string
pub fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)
}

/// Compute the event hash for tamper-evident chain
pub fn hash_event(
    id: &str,
    session_id: &str,
    timestamp: &str,
    event_type: &str,
    actor: &str,
    payload: &serde_json::Value,
    prev_hash: Option<&str>,
) -> String {
    // Canonical JSON for hashing — deterministic ordering
    let canonical = serde_json::json!({
        "id": id,
        "session_id": session_id,
        "timestamp": timestamp,
        "type": event_type,
        "actor": actor,
        "payload": payload,
        "prev_hash": prev_hash,
    });
    hash_content(&canonical.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_session_id() {
        let id = generate_session_id();
        assert!(id.starts_with("sess_"));
        assert!(id.len() > 10);
    }

    #[test]
    fn test_generate_event_id() {
        let id = generate_event_id();
        assert!(id.starts_with("evt_"));
    }

    #[test]
    fn test_ids_are_unique() {
        let id1 = generate_session_id();
        let id2 = generate_session_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_hash_content_deterministic() {
        let hash1 = hash_content("hello world");
        let hash2 = hash_content("hello world");
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn test_hash_content_different_inputs() {
        let hash1 = hash_content("content a");
        let hash2 = hash_content("content b");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_hash_event_consistent() {
        let payload = serde_json::json!({"file": "test.rs"});
        let hash1 = hash_event(
            "evt_123",
            "sess_456",
            "2026-05-31T14:00:00Z",
            "file.write",
            "agent",
            &payload,
            None,
        );
        let hash2 = hash_event(
            "evt_123",
            "sess_456",
            "2026-05-31T14:00:00Z",
            "file.write",
            "agent",
            &payload,
            None,
        );
        assert_eq!(hash1, hash2);
    }
}
