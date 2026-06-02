//! Shared helpers for import adapters.
//!
//! Many AI tools export their session history as JSONL, a JSON array, or a
//! single JSON envelope object that wraps an `events`/`messages` list. Their
//! field names also evolve quickly. Rather than re-implement the same tolerant
//! parsing loop in every adapter, the import adapters share this module:
//!
//! - [`read_json_values`] accepts JSONL, a top-level array, an envelope object,
//!   or a single object, and fails with a line-specific error on malformed
//!   JSONL (never silently dropping data).
//! - [`parse_stream`] drives the common loop: track the session id, classify
//!   each value into a Tellur [`EventType`], and build a [`TraceEvent`] with a
//!   sanitized, prompt-hashed payload.
//! - [`base_payload`] redacts secret-looking strings, hashes prompt-like text,
//!   and lifts the stable `model`/`command`/`file_path` concepts out of the many
//!   field spellings tools use.
//!
//! Each adapter then only owns the part that is genuinely tool-specific: how its
//! event names map to Tellur event types.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use tellur_core::schema::types::*;

/// Envelope keys whose array value holds the event list when a tool exports a
/// single wrapping object instead of a bare array or JSONL stream.
const ENVELOPE_LIST_KEYS: &[&str] = &[
    "events", "messages", "items", "records", "entries", "history", "data",
];

/// Field paths checked, in order, for a stable event id.
pub const EVENT_ID_PATHS: &[&[&str]] = &[&["id"], &["event_id"], &["eventId"], &["uuid"]];

/// Field paths checked, in order, for an event timestamp.
pub const TIMESTAMP_PATHS: &[&[&str]] = &[
    &["timestamp"],
    &["time"],
    &["created_at"],
    &["createdAt"],
    &["ts"],
    &["date"],
];

const COMMAND_PATHS: &[&[&str]] = &[
    &["command"],
    &["cmd"],
    &["tool_input", "command"],
    &["toolInput", "command"],
    &["data", "command"],
    &["params", "command"],
    &["args", "command"],
    &["input", "command"],
];

const FILE_PATH_PATHS: &[&[&str]] = &[
    &["file_path"],
    &["filePath"],
    &["filepath"],
    &["path"],
    &["file"],
    &["tool_input", "file_path"],
    &["tool_input", "path"],
    &["toolInput", "filePath"],
    &["data", "filepath"],
    &["data", "file_path"],
    &["data", "filePath"],
    &["params", "path"],
    &["args", "path"],
    &["input", "path"],
];

const MODEL_PATHS: &[&[&str]] = &[
    &["model"],
    &["model_id"],
    &["modelId"],
    &["model_name"],
    &["modelName"],
    &["data", "model"],
    &["data", "modelProvider"],
    &["data", "modelTitle"],
    &["params", "model"],
];

/// Read an import source as a list of JSON event values.
///
/// Accepts (in this order of preference):
/// - a top-level JSON array of events,
/// - a single JSON object that wraps the events under a known envelope key
///   (`events`, `messages`, ...),
/// - a single JSON object treated as one event,
/// - a JSONL stream (one JSON value per non-empty line).
///
/// Malformed JSONL fails with a line-specific error instead of dropping data.
pub fn read_json_values(path: &Path, adapter_label: &str) -> Result<Vec<Value>> {
    let content = std::fs::read_to_string(path)?;
    read_json_values_from_str(&content, adapter_label)
}

fn read_json_values_from_str(content: &str, adapter_label: &str) -> Result<Vec<Value>> {
    match serde_json::from_str::<Value>(content) {
        Ok(Value::Array(items)) => Ok(items),
        Ok(Value::Object(map)) => {
            for key in ENVELOPE_LIST_KEYS {
                if let Some(Value::Array(items)) = map.get(*key) {
                    return Ok(items.clone());
                }
            }
            Ok(vec![Value::Object(map)])
        }
        // A scalar document (or empty input) is not an event stream.
        Ok(_) => {
            if content.trim().is_empty() {
                Ok(Vec::new())
            } else {
                anyhow::bail!(
                    "{adapter_label} import expects a JSON array, object, or JSONL stream"
                )
            }
        }
        // Not a single JSON document: treat as JSONL.
        Err(doc_err) => {
            let mut items = Vec::new();
            for (idx, line) in content.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                items.push(serde_json::from_str(line).with_context(|| {
                    format!(
                        "invalid {adapter_label} JSON/JSONL at line {} (document parse failed: {doc_err})",
                        idx + 1,
                    )
                })?);
            }
            Ok(items)
        }
    }
}

/// Drive the shared import loop for a JSONL/array/envelope source.
///
/// `classify` is the only tool-specific piece: it maps a raw event value to a
/// Tellur [`EventType`]. The session id is taken from the first matching path in
/// `session_id_paths` and carried forward until a later event overrides it.
pub fn parse_stream(
    path: &Path,
    adapter_label: &str,
    tool: &str,
    fallback_session_id: &str,
    session_id_paths: &[&[&str]],
    classify: impl Fn(&Value) -> EventType,
) -> Result<Vec<TraceEvent>> {
    let values = read_json_values(path, adapter_label)?;
    let mut events = Vec::with_capacity(values.len());
    let mut session_id = fallback_session_id.to_string();

    for raw in &values {
        if let Some(id) = first_string(raw, session_id_paths) {
            session_id = id.to_string();
        }
        let event_type = classify(raw);
        events.push(build_event(raw, &session_id, event_type, tool));
    }

    Ok(events)
}

/// Build a single [`TraceEvent`] from a raw value, preserving source id and
/// timestamp where present and recording a sanitized payload.
pub fn build_event(raw: &Value, session_id: &str, event_type: EventType, tool: &str) -> TraceEvent {
    TraceEvent {
        schema: "tellur.event.v1".to_string(),
        id: first_string(raw, EVENT_ID_PATHS)
            .map(ToString::to_string)
            .unwrap_or_else(tellur_core::schema::ids::generate_event_id),
        session_id: session_id.to_string(),
        timestamp: timestamp(raw, TIMESTAMP_PATHS),
        event_type,
        actor: EventActor::Agent,
        payload: base_payload(raw, tool),
        redaction: None,
        prev_hash: None,
        event_hash: None,
    }
}

/// Build the normalized, redacted payload for an imported event.
///
/// The full raw value is kept (sanitized) for audit context, prompt-like text is
/// hashed rather than stored, and the stable `model`/`command`/`file_path`
/// concepts are lifted out of their many field spellings.
pub fn base_payload(raw: &Value, tool: &str) -> Value {
    let mut out = serde_json::json!({
        "tool": tool,
        "raw_payload": crate::sanitize::sanitized_value(raw),
    });
    if let Some(hash) = prompt_hash(raw) {
        out["prompt_hash"] = Value::String(hash);
    }
    if let Some(model) = first_string(raw, MODEL_PATHS) {
        out["model"] = Value::String(model.to_string());
    }
    if let Some(command) = first_string(raw, COMMAND_PATHS) {
        out["command"] = crate::sanitize::sanitized_value(&Value::String(command.to_string()));
    }
    if let Some(file_path) = first_string(raw, FILE_PATH_PATHS) {
        out["file_path"] = crate::sanitize::sanitized_value(&Value::String(file_path.to_string()));
    }
    out
}

/// Hash prompt-like text from the top level or a nested `data`/`payload` object.
fn prompt_hash(raw: &Value) -> Option<String> {
    crate::sanitize::first_prompt_hash(raw)
        .or_else(|| raw.get("data").and_then(crate::sanitize::first_prompt_hash))
        .or_else(|| {
            raw.get("payload")
                .and_then(crate::sanitize::first_prompt_hash)
        })
}

/// Resolve an event timestamp as RFC 3339, accepting either string timestamps or
/// numeric epoch values (seconds or milliseconds, as some editors emit).
fn timestamp(raw: &Value, paths: &[&[&str]]) -> String {
    if let Some(s) = first_string(raw, paths) {
        return s.to_string();
    }
    for path in paths {
        if let Some(n) = json_path(raw, path).and_then(Value::as_i64) {
            return epoch_to_rfc3339(n);
        }
    }
    chrono::Utc::now().to_rfc3339()
}

fn epoch_to_rfc3339(n: i64) -> String {
    // Values past the year ~2001 in milliseconds exceed this threshold; smaller
    // values are treated as whole seconds.
    let (secs, nanos) = if n.abs() >= 1_000_000_000_000 {
        (n / 1000, ((n % 1000).unsigned_abs() as u32) * 1_000_000)
    } else {
        (n, 0)
    };
    chrono::DateTime::from_timestamp(secs, nanos)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339())
}

/// First string value found by walking each candidate path in order.
pub fn first_string<'a>(value: &'a Value, paths: &[&[&str]]) -> Option<&'a str> {
    paths
        .iter()
        .filter_map(|path| json_path(value, path))
        .find_map(|value| value.as_str())
}

/// Walk a `["a", "b"]` key path into a nested object.
pub fn json_path<'a>(mut value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    for key in path {
        value = value.get(*key)?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_json_array() {
        let values = read_json_values_from_str(r#"[{"a":1},{"a":2}]"#, "Test").unwrap();
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn reads_envelope_object() {
        let values =
            read_json_values_from_str(r#"{"events":[{"a":1},{"a":2},{"a":3}]}"#, "Test").unwrap();
        assert_eq!(values.len(), 3);
    }

    #[test]
    fn reads_single_object_as_one_event() {
        let values = read_json_values_from_str(r#"{"type":"prompt"}"#, "Test").unwrap();
        assert_eq!(values.len(), 1);
    }

    #[test]
    fn reads_jsonl() {
        let values = read_json_values_from_str("{\"a\":1}\n\n{\"a\":2}\n", "Test").unwrap();
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn rejects_malformed_jsonl_with_line_number() {
        let err = read_json_values_from_str("{\"a\":1}\nnot-json\n", "Test").unwrap_err();
        assert!(err.to_string().contains("line 2"));
    }

    #[test]
    fn base_payload_hashes_prompt_and_redacts_secrets() {
        let raw = serde_json::json!({
            "prompt": "use token=abcdefghijklmnopqrstuvwxyz0123",
            "model": "some-model",
            "file_path": "src/main.rs"
        });
        let payload = base_payload(&raw, "test-tool");
        assert!(payload.get("prompt_hash").is_some());
        assert_eq!(payload["model"], "some-model");
        assert_eq!(payload["file_path"], "src/main.rs");
        let serialized = serde_json::to_string(&payload).unwrap();
        assert!(!serialized.contains("abcdefghijklmnopqrstuvwxyz0123"));
    }

    #[test]
    fn base_payload_finds_nested_fields() {
        let raw = serde_json::json!({
            "name": "editInteraction",
            "data": {"filepath": "app/lib.ts", "modelTitle": "claude"}
        });
        let payload = base_payload(&raw, "continue");
        assert_eq!(payload["file_path"], "app/lib.ts");
        assert_eq!(payload["model"], "claude");
    }

    #[test]
    fn timestamp_accepts_epoch_millis() {
        let raw = serde_json::json!({"ts": 1_700_000_000_000_i64});
        let ts = timestamp(&raw, TIMESTAMP_PATHS);
        assert!(ts.starts_with("2023-11-"));
    }
}
