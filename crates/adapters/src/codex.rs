//! Codex CLI adapter — JSONL event stream/session transcript import.

use std::path::Path;

use anyhow::Result;
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
        let values = crate::import::read_json_values(path, "Codex")?;
        let mut events = Vec::new();
        let mut session_id = fallback_session_id.to_string();

        for raw in &values {
            if let Some(id) = crate::import::first_string(raw, &[&["session_id"], &["sessionId"]]) {
                session_id = id.to_string();
            }
            if raw.get("type").and_then(|v| v.as_str()) == Some("session_meta")
                && let Some(id) = raw
                    .get("payload")
                    .and_then(|p| p.get("id"))
                    .and_then(|v| v.as_str())
            {
                session_id = id.to_string();
            }
            events.push(crate::import::build_event(
                raw,
                &session_id,
                codex_event_type(raw),
                "codex",
            ));
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

    #[test]
    fn test_parse_codex_envelope_preserves_source_identity_and_epoch_time() {
        let adapter = CodexAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_codex_envelope");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "events": [
                    {"type": "session_meta", "payload": {"id": "rollout-7"}},
                    {
                        "id": "source-event-9",
                        "ts": 1_700_000_000_000_i64,
                        "type": "event_msg",
                        "payload": {"type": "file_change", "path": "src/main.rs"}
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();

        let events = adapter.parse_jsonl(&path, "fallback").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].session_id, "rollout-7");
        assert_eq!(events[1].id, "source-event-9");
        assert!(events[1].timestamp.starts_with("2023-11-"));
        assert_eq!(events[1].payload["file_path"], "src/main.rs");
    }

    #[test]
    fn test_parse_codex_envelope_inherits_wrapper_session_without_meta_event() {
        let adapter = CodexAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_codex_wrapper_session");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("telemetry.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "sessionId": "editor-session-42",
                "events": [{
                    "type": "event_msg",
                    "payload": {"type": "file_change", "path": "src/lib.rs"}
                }]
            })
            .to_string(),
        )
        .unwrap();

        let events = adapter.parse_jsonl(&path, "fallback").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].session_id, "editor-session-42");
    }
}
