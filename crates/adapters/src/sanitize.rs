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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prompt_like_keys_are_hashed_not_stored_raw() {
        let out = sanitized_value(&json!({ "prompt": "secret design notes" }));
        let prompt = &out["prompt"];
        assert_eq!(prompt["redacted"], json!(true));
        assert!(prompt["hash"].as_str().is_some());
        // The raw text must never survive.
        assert!(!out.to_string().contains("secret design notes"));
    }

    #[test]
    fn non_prompt_strings_are_secret_redacted_but_clean_text_passes_through() {
        // A clean (no-secret) non-prompt string is preserved verbatim.
        assert_eq!(
            sanitized_value(&json!({ "label": "const x = 42;" })),
            json!({ "label": "const x = 42;" })
        );
        // A secret in a non-prompt string is redacted in place.
        let out = sanitized_value(&json!({ "cmd": "api_key=sk-abclongkeyvalue12345" }));
        let cmd = out["cmd"].as_str().unwrap();
        assert!(cmd.contains("[REDACTED]"), "secret not redacted: {cmd}");
        assert!(!cmd.contains("sk-abclongkeyvalue12345"));
    }

    #[test]
    fn sanitize_recurses_arrays_and_preserves_scalars() {
        let out = sanitized_value(&json!({
            "items": [{ "message": "hi there" }],
            "count": 3,
            "ok": true,
        }));
        assert_eq!(out["items"][0]["message"]["redacted"], json!(true));
        assert_eq!(out["count"], json!(3));
        assert_eq!(out["ok"], json!(true));
    }

    #[test]
    fn first_prompt_hash_picks_a_text_key_or_none() {
        let h = first_prompt_hash(&json!({ "irrelevant": 1, "text": "ask" }));
        assert_eq!(h, prompt_hash(&json!("ask")));
        assert!(first_prompt_hash(&json!({ "irrelevant": 1 })).is_none());
        // Non-string prompt-like values do not produce a hash.
        assert!(prompt_hash(&json!(42)).is_none());
    }
}
