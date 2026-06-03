//! Webhook ingestion — normalize a tool's native webhook payload into Tellur
//! events.
//!
//! This is the live-capture surface for cloud agents that have no local
//! lifecycle hook or editor extension (Devin in particular). The daemon accepts
//! the tool's native JSON body on `POST /webhook/{source}` and this module maps
//! it onto canonical Tellur events, hashing prompt-like fields and redacting
//! command/text strings before they are stored.
//!
//! Unlike the file-based import adapters in the `tellur-adapters` crate, this
//! runs inside `tellur-core` (the daemon lives here and core cannot depend on
//! adapters). It is intentionally a small, tolerant normalizer: known event
//! kinds map to canonical types, unknown kinds are preserved as
//! `<source>.<kind>` custom events rather than dropped, and only known fields
//! are extracted so arbitrary raw payload text is never persisted verbatim.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::redaction::RedactionEngine;
use crate::schema::ids;
use crate::schema::types::{Actor, AgentInfo, EventActor, ModelInfo, Session};
use crate::storage::{EventWriter, TraceIndex};

/// Paths that may carry a session/run identifier in a webhook payload.
const SESSION_ID_PATHS: &[&[&str]] = &[
    &["session_id"],
    &["sessionId"],
    &["devin_run_id"],
    &["run_id"],
    &["runId"],
    &["session", "id"],
    &["run", "id"],
];

/// Paths that may carry a file path.
const FILE_PATH_PATHS: &[&[&str]] = &[
    &["file_path"],
    &["filePath"],
    &["path"],
    &["file"],
    &["filename"],
    &["fileURI"],
];

/// Paths that may carry a shell command.
const COMMAND_PATHS: &[&[&str]] = &[&["command"], &["cmd"], &["shell"], &["script"], &["text"]];

/// Paths that may carry prompt-like text (hashed, never stored raw).
const PROMPT_PATHS: &[&[&str]] = &[
    &["prompt"],
    &["message"],
    &["text"],
    &["content"],
    &["input"],
    &["user_input"],
];

/// Paths that may carry a model name.
const MODEL_PATHS: &[&[&str]] = &[&["model"], &["model_name"], &["modelName"]];

/// Result of ingesting a webhook payload.
pub struct WebhookIngest {
    pub session_id: String,
    pub event_ids: Vec<String>,
}

/// Ingest a tool's native webhook payload into the local event log, recomputing
/// the hash chain so provenance cannot be forged through the HTTP API.
pub fn ingest_webhook(repo_root: &Path, source: &str, body: &Value) -> Result<WebhookIngest> {
    let source = normalize_source(source);
    let items = collect_items(body);

    let session_id = first_string(body, SESSION_ID_PATHS)
        .map(|s| s.to_string())
        .or_else(|| {
            items
                .iter()
                .find_map(|item| first_string(item, SESSION_ID_PATHS).map(|s| s.to_string()))
        })
        .unwrap_or_else(ids::generate_session_id);

    let model = first_string(body, MODEL_PATHS)
        .or_else(|| {
            items
                .iter()
                .find_map(|item| first_string(item, MODEL_PATHS))
        })
        .map(|s| s.to_string());

    // Record (or refresh) the session row so it appears in the dashboard.
    let index = TraceIndex::open(&repo_root.join(".tellur").join("index").join("tellur.db"))
        .context("failed to open Tellur index for webhook ingestion")?;
    let repo_id = ids::hash_content(&repo_root.to_string_lossy());
    let mut session = Session::new(repo_id, webhook_actor(), agent_info(source));
    session.id = session_id.clone();
    if let Some(model) = model.as_deref() {
        session.model = Some(ModelInfo {
            provider: source.to_string(),
            name: model.to_string(),
            version: None,
        });
    }
    index.index_session(&session)?;

    let redaction = RedactionEngine::default_engine();
    let traces_dir = repo_root.join(".tellur").join("traces");
    let mut writer = EventWriter::new(&traces_dir);
    writer.open()?;

    let mut event_ids = Vec::new();
    for item in &items {
        let (wire_type, payload) = normalize_item(source, item, &redaction);
        let event = writer.write_event(&session_id, &wire_type, "agent", payload, None)?;
        index.index_event(&event)?;
        event_ids.push(event.id);
    }
    writer.close();

    Ok(WebhookIngest {
        session_id,
        event_ids,
    })
}

/// Flatten a webhook body into a list of event-like items. Supports a bare
/// array, an envelope object with an `events`/`messages`/`steps` array, or a
/// single event object.
fn collect_items(body: &Value) -> Vec<Value> {
    if let Some(array) = body.as_array() {
        return array.clone();
    }
    for key in ["events", "messages", "steps", "actions", "items"] {
        if let Some(array) = body.get(key).and_then(|v| v.as_array()) {
            return array.clone();
        }
    }
    vec![body.clone()]
}

/// Map a single item to a canonical wire event type and a sanitized payload.
fn normalize_item(
    source: &str,
    item: &Value,
    redaction: &RedactionEngine,
) -> (String, serde_json::Value) {
    let kind = first_string(
        item,
        &[
            &["type"],
            &["event"],
            &["event_type"],
            &["kind"],
            &["action"],
            &["role"],
        ],
    );

    let wire_type = wire_event_type(source, kind);

    let mut payload = serde_json::json!({
        "tool": source,
        "source": "webhook",
    });
    if let Some(kind) = kind {
        payload["kind"] = Value::String(kind.to_string());
    }
    if let Some(file_path) = first_string(item, FILE_PATH_PATHS) {
        payload["file_path"] = Value::String(file_path.to_string());
    }
    if wire_type == "command.exec"
        && let Some(command) = first_string(item, COMMAND_PATHS)
    {
        payload["command"] = Value::String(redact(redaction, command));
    }
    if wire_type == "user.prompt"
        && let Some(prompt) = first_string(item, PROMPT_PATHS)
    {
        payload["prompt_hash"] = Value::String(ids::hash_content(prompt));
    }
    (wire_type, payload)
}

/// Classify a webhook event kind into a canonical Tellur wire event type.
fn wire_event_type(source: &str, kind: Option<&str>) -> String {
    match kind {
        Some("session_start" | "run_started" | "start" | "session.created" | "session.start") => {
            "session.start".to_string()
        }
        Some(
            "session_end" | "run_finished" | "run_completed" | "end" | "session.finished"
            | "session.end",
        ) => "session.end".to_string(),
        Some("user" | "user_message" | "prompt" | "message" | "request" | "user_input") => {
            "user.prompt".to_string()
        }
        Some(
            "shell" | "shell_command" | "command" | "run_command" | "exec" | "bash" | "terminal",
        ) => "command.exec".to_string(),
        Some(
            "edit" | "edit_file" | "file_write" | "write" | "write_file" | "create_file"
            | "apply_diff" | "patch",
        ) => "file.write".to_string(),
        Some("read" | "read_file" | "view_file" | "open") => "file.read".to_string(),
        Some("pull_request" | "git_commit" | "commit") => format!("{source}.git"),
        Some("assistant" | "agent_message" | "ai_response" | "response") => {
            format!("{source}.response")
        }
        Some(other) => format!("{source}.{}", sanitize_kind(other)),
        None => format!("{source}.event"),
    }
}

fn redact(engine: &RedactionEngine, value: &str) -> String {
    engine
        .scan_and_redact(value)
        .redacted_content
        .unwrap_or_else(|| "[REDACTED]".to_string())
}

fn webhook_actor() -> Actor {
    Actor {
        name: "webhook".to_string(),
        email: None,
        email_hash: None,
        actor_type: EventActor::Agent,
    }
}

fn agent_info(source: &str) -> AgentInfo {
    AgentInfo {
        id: source.to_string(),
        name: agent_display_name(source).to_string(),
        version: None,
    }
}

fn agent_display_name(source: &str) -> &str {
    match source {
        "devin" => "Devin",
        "jetbrains" => "JetBrains AI / Junie",
        "windsurf" => "Windsurf / Cascade",
        "continue" => "Continue",
        "cline" => "Cline / Roo Code",
        other => other,
    }
}

fn normalize_source(source: &str) -> &str {
    match source {
        "devin" => "devin",
        "windsurf" | "cascade" => "windsurf",
        "jetbrains" | "junie" | "jetbrains-ai" => "jetbrains",
        "continue" | "continue-dev" => "continue",
        "cline" | "roo" | "roo-code" => "cline",
        other => other,
    }
}

fn sanitize_kind(kind: &str) -> String {
    kind.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// Return the first string found at any of the candidate paths.
fn first_string<'a>(value: &'a Value, paths: &[&[&str]]) -> Option<&'a str> {
    for path in paths {
        let mut current = value;
        let mut matched = true;
        for key in *path {
            match current.get(key) {
                Some(next) => current = next,
                None => {
                    matched = false;
                    break;
                }
            }
        }
        if matched
            && let Some(s) = current.as_str()
            && !s.is_empty()
        {
            return Some(s);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_repo(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tellur-webhook-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join(".tellur").join("traces")).unwrap();
        std::fs::create_dir_all(dir.join(".tellur").join("index")).unwrap();
        dir
    }

    #[test]
    fn test_devin_run_envelope_normalizes_events() {
        let repo = temp_repo("devin");
        let body = serde_json::json!({
            "devin_run_id": "run-99",
            "model": "devin-1",
            "messages": [
                {"type": "user", "message": "fix the failing test"},
                {"type": "shell", "command": "pytest -k secret AWS_SECRET=AKIAIOSFODNN7EXAMPLE"},
                {"type": "edit", "file_path": "app/calc.py"},
                {"type": "pull_request", "url": "https://example/pr/1"}
            ]
        });

        let result = ingest_webhook(&repo, "devin", &body).unwrap();
        assert_eq!(result.session_id, "run-99");
        assert_eq!(result.event_ids.len(), 4);

        let index =
            TraceIndex::open(&repo.join(".tellur").join("index").join("tellur.db")).unwrap();
        let events = index.get_session_events("run-99").unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].event_type.as_wire(), "user.prompt");
        assert!(events[0].payload.get("prompt_hash").is_some());
        // Raw prompt text is never stored.
        assert!(events[0].payload.get("message").is_none());
        assert_eq!(events[1].event_type.as_wire(), "command.exec");
        let command = events[1].payload["command"].as_str().unwrap();
        assert!(command.contains("[REDACTED]"));
        assert!(!command.contains("AKIAIOSFODNN7EXAMPLE"));
        assert_eq!(events[2].event_type.as_wire(), "file.write");
        assert_eq!(events[2].payload["file_path"], "app/calc.py");
        assert_eq!(events[3].event_type.as_wire(), "devin.git");

        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_single_object_and_unknown_kind() {
        let repo = temp_repo("single");
        let body = serde_json::json!({
            "session_id": "s-1",
            "type": "telemetry_ping",
            "file_path": "README.md"
        });
        let result = ingest_webhook(&repo, "devin", &body).unwrap();
        assert_eq!(result.event_ids.len(), 1);
        let index =
            TraceIndex::open(&repo.join(".tellur").join("index").join("tellur.db")).unwrap();
        let events = index.get_session_events("s-1").unwrap();
        assert_eq!(events[0].event_type.as_wire(), "devin.telemetry_ping");
        assert_eq!(events[0].payload["file_path"], "README.md");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_generated_session_id_when_absent() {
        let repo = temp_repo("nosess");
        let body = serde_json::json!([{ "type": "shell", "command": "ls" }]);
        let result = ingest_webhook(&repo, "devin", &body).unwrap();
        assert!(result.session_id.starts_with("sess_") || !result.session_id.is_empty());
        assert_eq!(result.event_ids.len(), 1);
        let _ = std::fs::remove_dir_all(&repo);
    }
}
