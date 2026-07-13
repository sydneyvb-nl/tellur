//! GitHub Copilot adapter — metadata event import.

use std::path::Path;

use anyhow::Result;
use tellur_core::adapter::{AdapterCapabilities, AdapterInfo, AgentAdapter};
use tellur_core::schema::types::*;

pub struct CopilotAdapter {
    info: AdapterInfo,
}

impl Default for CopilotAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl CopilotAdapter {
    pub fn new() -> Self {
        Self {
            info: AdapterInfo {
                name: "copilot".to_string(),
                version: "0.1.0".to_string(),
                tool_name: "GitHub Copilot".to_string(),
            },
        }
    }

    /// Parse Copilot metadata exported as a JSON array or JSONL.
    ///
    /// Copilot integrations vary by editor, so this importer accepts the common
    /// metadata fields teams can export from editor logs or telemetry pipelines.
    pub fn parse_metadata_file(&self, path: &Path, session_id: &str) -> Result<Vec<TraceEvent>> {
        crate::import::parse_stream(
            path,
            "Copilot metadata",
            "github-copilot",
            session_id,
            &[&["session_id"], &["sessionId"], &["conversation_id"]],
            copilot_event_type,
        )
    }
}

fn copilot_event_type(entry: &serde_json::Value) -> EventType {
    let kind = entry
        .get("type")
        .or_else(|| entry.get("event"))
        .or_else(|| entry.get("kind"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    match kind {
        "chat.prompt" | "prompt" | "prompt.submitted" => EventType::UserPrompt,
        "suggestion.accepted" | "completion.accepted" | "accepted" => EventType::FileWrite,
        "command" | "command.executed" => EventType::CommandExecution,
        "suggestion.shown" | "completion.shown" => {
            EventType::Custom("copilot.suggestion".to_string())
        }
        other if !other.is_empty() => EventType::Custom(format!("copilot.{other}")),
        _ => EventType::Custom("copilot.unknown".to_string()),
    }
}

#[async_trait::async_trait]
impl AgentAdapter for CopilotAdapter {
    fn info(&self) -> &AdapterInfo {
        &self.info
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            can_capture_file_writes: true,
            can_capture_commands: false,
            can_capture_prompts: true,
            can_replay_session: false,
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
        let adapter = CopilotAdapter::new();
        assert_eq!(adapter.info().name, "copilot");
        assert_eq!(adapter.info().tool_name, "GitHub Copilot");
        assert!(adapter.capabilities().can_capture_prompts);
    }

    #[test]
    fn test_parse_copilot_metadata_jsonl() {
        let adapter = CopilotAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_copilot");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("copilot.jsonl");

        let lines = [
            serde_json::json!({
                "timestamp": "2026-05-31T11:00:00Z",
                "type": "chat.prompt",
                "prompt_hash": "sha256:abc",
                "model": "gpt-4.1"
            }),
            serde_json::json!({
                "timestamp": "2026-05-31T11:00:05Z",
                "type": "suggestion.accepted",
                "file": "src/main.ts",
                "language": "typescript",
                "model": "gpt-4.1",
                "completion_id": "cmp_123"
            }),
        ]
        .into_iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n");
        std::fs::write(&path, lines).unwrap();

        let events = adapter.parse_metadata_file(&path, "sess_copilot").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, EventType::UserPrompt);
        assert_eq!(events[0].payload["prompt_hash"], "sha256:abc");
        assert_eq!(events[1].event_type, EventType::FileWrite);
        assert_eq!(events[1].payload["file_path"], "src/main.ts");
        assert_eq!(events[1].payload["completion_id"], "cmp_123");
    }

    #[test]
    fn test_parse_copilot_redacts_raw_prompt_material() {
        let adapter = CopilotAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_copilot_redaction");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("copilot.jsonl");
        std::fs::write(
            &path,
            serde_json::json!({
                "timestamp": "2026-05-31T11:00:00Z",
                "type": "chat.prompt",
                "prompt": "token=abcdefghijklmnopqrstuvwxyz123456",
                "model": "gpt-4.1"
            })
            .to_string(),
        )
        .unwrap();

        let events = adapter.parse_metadata_file(&path, "sess_copilot").unwrap();
        let payload = &events[0].payload;
        assert!(payload.get("prompt_hash").is_some());
        assert!(payload.get("raw").is_none());
        assert!(
            !serde_json::to_string(payload)
                .unwrap()
                .contains("abcdefghijklmnopqrstuvwxyz123456")
        );
    }

    #[test]
    fn test_parse_copilot_envelope_inherits_harness_session() {
        let adapter = CopilotAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_copilot_envelope");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("telemetry.json");
        std::fs::write(
            &path,
            serde_json::json!({
                "sessionId": "editor-harness-4",
                "records": [{
                    "eventId": "accepted-5",
                    "createdAt": "2026-07-12T08:00:00Z",
                    "kind": "completion.accepted",
                    "filePath": "src/editor.ts",
                    "suggestion_id": "suggestion-5"
                }]
            })
            .to_string(),
        )
        .unwrap();

        let events = adapter.parse_metadata_file(&path, "fallback").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "accepted-5");
        assert_eq!(events[0].session_id, "editor-harness-4");
        assert_eq!(events[0].timestamp, "2026-07-12T08:00:00Z");
        assert_eq!(events[0].event_type, EventType::FileWrite);
        assert_eq!(events[0].payload["file_path"], "src/editor.ts");
        assert_eq!(events[0].payload["suggestion_id"], "suggestion-5");
    }
}
