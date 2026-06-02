use tellur_core::redaction::RedactionEngine;
use tellur_core::schema::ids::hash_content;

// Prompt-like fields hashed rather than stored as raw text. Kept in sync with
// `import::PROMPT_LEAF_KEYS` so adapters that use the shared import loop hash the
// same fields here inside `raw_payload`.
const TEXT_CONTENT_KEYS: &[&str] = &[
    "message",
    "prompt",
    "text",
    "content",
    "user_response",
    "user_message",
    "question",
];

pub fn prompt_hash(value: &serde_json::Value) -> Option<String> {
    value.as_str().map(hash_content)
}

pub fn first_prompt_hash(payload: &serde_json::Value) -> Option<String> {
    TEXT_CONTENT_KEYS
        .iter()
        .find_map(|key| payload.get(*key).and_then(prompt_hash))
}

pub fn sanitized_value(value: &serde_json::Value) -> serde_json::Value {
    sanitize_with_key(None, value)
}

fn sanitize_with_key(key: Option<&str>, value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => {
            if key.is_some_and(|k| TEXT_CONTENT_KEYS.contains(&k)) {
                serde_json::json!({
                    "redacted": true,
                    "hash": hash_content(s),
                })
            } else {
                let redacted = RedactionEngine::default_engine()
                    .scan_and_redact(s)
                    .redacted_content
                    .unwrap_or_else(|| "[REDACTED]".to_string());
                serde_json::Value::String(redacted)
            }
        }
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|item| sanitize_with_key(None, item))
                .collect(),
        ),
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), sanitize_with_key(Some(k.as_str()), v)))
                .collect(),
        ),
        other => other.clone(),
    }
}
