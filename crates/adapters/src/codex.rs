//! Codex CLI adapter — JSONL event stream/session transcript import.

use std::path::Path;

use anyhow::{Context, Result};
use tellur_core::adapter::{AdapterCapabilities, AdapterInfo, AgentAdapter};
use tellur_core::schema::types::*;

pub struct CodexAdapter {
    info: AdapterInfo,
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexAdapter {
    pub fn new() -> Self {
        Self {
            info: AdapterInfo {
                name: "codex".to_string(),
                version: "0.1.0".to_string(),
                tool_name: "Codex CLI".to_string(),
            },
        }
    }

    /// Parse Codex CLI JSONL output/session transcript into Tellur events.
    ///
    /// The adapter is intentionally tolerant because Codex stream fields evolve:
    /// it preserves the raw payload and normalizes stable concepts only.
    pub fn parse_jsonl(&self, path: &Path, fallback_session_id: &str) -> Result<Vec<TraceEvent>> {
        let content = std::fs::read_to_string(path)?;
        let mut events = Vec::new();
        let mut session_id = fallback_session_id.to_string();

        for (idx, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let raw = serde_json::from_str::<serde_json::Value>(line)
                .with_context(|| format!("invalid Codex JSONL at line {}", idx + 1))?;

            if raw.get("type").and_then(|v| v.as_str()) == Some("session_meta")
                && let Some(id) = raw
                    .get("payload")
                    .and_then(|p| p.get("id"))
                    .and_then(|v| v.as_str())
            {
                session_id = id.to_string();
            }

            let event_type = codex_event_type(&raw);
            let payload = normalized_payload(&raw);
            events.push(TraceEvent {
                schema: "tellur.event.v1".to_string(),
                id: tellur_core::schema::ids::generate_event_id(),
                session_id: session_id.clone(),
                timestamp: raw
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                event_type,
                actor: EventActor::Agent,
                payload,
                redaction: None,
                prev_hash: None,
                event_hash: None,
            });
        }

        Ok(events)
    }
}

fn codex_event_type(raw: &serde_json::Value) -> EventType {
    let top_type = raw.get("type").and_then(|v| v.as_str());
    let payload_type = raw
        .get("payload")
        .and_then(|p| p.get("type"))
        .and_then(|v| v.as_str());
    match (top_type, payload_type) {
        (Some("session_meta"), _) => EventType::SessionStart,
        (_, Some("user_message" | "user_input" | "prompt" | "prompt_submitted")) => {
            EventType::UserPrompt
        }
        (
            _,
            Some(
                "exec_command_begin" | "exec_command_end" | "command_begin" | "command_end"
                | "command",
            ),
        ) => EventType::CommandExecution,
        (
            _,
            Some(
                "file_change" | "file_write" | "write_file" | "apply_patch_begin"
                | "apply_patch_end" | "patch_apply_begin" | "patch_apply_end",
            ),
        ) => EventType::FileWrite,
        (_, Some("agent_message" | "assistant_message")) => {
            EventType::Custom("ai.response".to_string())
        }
        (_, Some(other)) => EventType::Custom(format!("codex.{other}")),
        (Some(other), _) => EventType::Custom(format!("codex.{other}")),
        _ => EventType::Custom("codex.unknown".to_string()),
    }
}

fn normalized_payload(raw: &serde_json::Value) -> serde_json::Value {
    let payload = raw.get("payload").cloned().unwrap_or_else(|| raw.clone());
    let prompt_hash = crate::sanitize::first_prompt_hash(&payload);
    let command = payload
        .get("command")
        .or_else(|| payload.get("cmd"))
        .or_else(|| payload.get("argv"))
        .cloned();
    let file_path = payload
        .get("file_path")
        .or_else(|| payload.get("path"))
        .or_else(|| payload.get("file"))
        .cloned();
    let mut out = serde_json::json!({
        "tool": "codex",
        "raw_type": raw.get("type"),
        "raw_payload": crate::sanitize::sanitized_value(&payload),
    });
    if let Some(command) = command {
        out["command"] = crate::sanitize::sanitized_value(&command);
    }
    if let Some(file_path) = file_path {
        out["file_path"] = crate::sanitize::sanitized_value(&file_path);
    }
    if let Some(prompt_hash) = prompt_hash {
        out["prompt_hash"] = serde_json::Value::String(prompt_hash);
    }
    if let Some(model) = raw
        .get("payload")
        .and_then(|p| p.get("model").or_else(|| p.get("model_name")))
        .cloned()
    {
        out["model"] = model;
    }
    out
}

#[async_trait::async_trait]
impl AgentAdapter for CodexAdapter {
    fn info(&self) -> &AdapterInfo {
        &self.info
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            can_capture_file_writes: true,
            can_capture_commands: true,
            can_capture_prompts: true,
            can_replay_session: true,
            supports_hooks: false,
        }
    }

    async fn start_session(&self, session: &Session) -> Result<String> {
        Ok(session.id.clone())
    }

    async fn end_session(&self, _session_id: &str) -> Result<()> {
        Ok(())
    }

    async fn capture_event(&self, _event: &TraceEvent) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_info() {
        let adapter = CodexAdapter::new();
        assert_eq!(adapter.info().name, "codex");
        assert_eq!(adapter.info().tool_name, "Codex CLI");
        assert!(adapter.capabilities().can_capture_commands);
    }

    #[test]
    fn test_parse_codex_jsonl_stream() {
        let adapter = CodexAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_codex");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");

        let lines = [
            serde_json::json!({
                "timestamp": "2026-05-31T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "codex-session",
                    "cwd": "/repo",
                    "originator": "codex_cli_rs",
                    "model_provider": "openai",
                    "model": "gpt-5-codex"
                }
            }),
            serde_json::json!({
                "timestamp": "2026-05-31T10:00:01Z",
                "type": "event_msg",
                "payload": {
                    "type": "user_message",
                    "message": "add tests"
                }
            }),
            serde_json::json!({
                "timestamp": "2026-05-31T10:00:02Z",
                "type": "event_msg",
                "payload": {
                    "type": "exec_command_begin",
                    "command": "cargo test"
                }
            }),
            serde_json::json!({
                "timestamp": "2026-05-31T10:00:03Z",
                "type": "event_msg",
                "payload": {
                    "type": "file_change",
                    "path": "src/lib.rs"
                }
            }),
        ]
        .into_iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        std::fs::write(&path, lines).unwrap();

        let events = adapter.parse_jsonl(&path, "fallback-session").unwrap();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].session_id, "codex-session");
        assert_eq!(events[0].event_type, EventType::SessionStart);
        assert_eq!(events[1].event_type, EventType::UserPrompt);
        assert_eq!(events[2].event_type, EventType::CommandExecution);
        assert_eq!(events[2].payload["command"], "cargo test");
        assert_eq!(events[3].event_type, EventType::FileWrite);
        assert_eq!(events[3].payload["file_path"], "src/lib.rs");
    }

    #[test]
    fn test_parse_codex_jsonl_rejects_invalid_lines() {
        let adapter = CodexAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_codex_invalid");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        std::fs::write(&path, "{\"type\":\"event_msg\"}\nnot-json\n").unwrap();

        let err = adapter.parse_jsonl(&path, "fallback-session").unwrap_err();
        assert!(err.to_string().contains("line 2"));
    }

    #[test]
    fn test_parse_codex_hashes_prompt_and_redacts_raw_payload() {
        let adapter = CodexAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_codex_redaction");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        std::fs::write(
            &path,
            serde_json::json!({
                "type": "event_msg",
                "payload": {
                    "type": "user_message",
                    "message": "use api_key=sk-abclongkeyvalue12345"
                }
            })
            .to_string(),
        )
        .unwrap();

        let events = adapter.parse_jsonl(&path, "sess").unwrap();
        let payload = &events[0].payload;
        assert!(payload.get("prompt_hash").is_some());
        let serialized = serde_json::to_string(payload).unwrap();
        assert!(!serialized.contains("sk-abclongkeyvalue12345"));
        assert!(!serialized.contains("use api_key"));
    }
}
