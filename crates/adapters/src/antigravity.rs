//! Google Antigravity adapter — hook/agent JSONL import parser.

use std::path::Path;

use anyhow::{Context, Result};
use tellur_core::adapter::{AdapterCapabilities, AdapterInfo, AgentAdapter};
use tellur_core::schema::types::*;

pub struct AntigravityAdapter {
    info: AdapterInfo,
}

impl Default for AntigravityAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl AntigravityAdapter {
    pub fn new() -> Self {
        Self {
            info: AdapterInfo {
                name: "antigravity".to_string(),
                version: "0.1.0".to_string(),
                tool_name: "Google Antigravity".to_string(),
            },
        }
    }

    /// Parse Antigravity hook/export JSONL into Tellur events.
    pub fn parse_jsonl(&self, path: &Path, fallback_session_id: &str) -> Result<Vec<TraceEvent>> {
        let content = std::fs::read_to_string(path)?;
        let mut events = Vec::new();
        let mut session_id = fallback_session_id.to_string();

        for (idx, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let raw = serde_json::from_str::<serde_json::Value>(line)
                .with_context(|| format!("invalid Antigravity JSONL at line {}", idx + 1))?;
            if let Some(id) = crate::import::first_string(
                &raw,
                &[
                    &["conversationId"],
                    &["conversation_id"],
                    &["session_id"],
                    &["sessionId"],
                    &["session", "id"],
                ],
            ) {
                session_id = id.to_string();
            }
            events.push(TraceEvent {
                schema: "tellur.event.v1".to_string(),
                id: crate::import::first_string(&raw, &[&["id"], &["event_id"], &["eventId"]])
                    .map(ToString::to_string)
                    .unwrap_or_else(tellur_core::schema::ids::generate_event_id),
                session_id: session_id.clone(),
                timestamp: crate::import::first_string(
                    &raw,
                    &[&["timestamp"], &["time"], &["created_at"]],
                )
                .map(ToString::to_string)
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                event_type: crate::gemini::google_agent_event_type(&raw, "antigravity"),
                actor: EventActor::Agent,
                payload: crate::gemini::google_agent_payload(&raw, "antigravity"),
                redaction: None,
                prev_hash: None,
                event_hash: None,
            });
        }

        Ok(events)
    }
}

#[async_trait::async_trait]
impl AgentAdapter for AntigravityAdapter {
    fn info(&self) -> &AdapterInfo {
        &self.info
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            can_capture_file_writes: true,
            can_capture_commands: true,
            can_capture_prompts: true,
            can_replay_session: true,
            supports_hooks: true,
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
        let adapter = AntigravityAdapter::new();
        assert_eq!(adapter.info().name, "antigravity");
        assert!(adapter.capabilities().supports_hooks);
    }

    #[test]
    fn test_parse_antigravity_jsonl() {
        let adapter = AntigravityAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_antigravity");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        let lines = [
            serde_json::json!({
                "conversationId": "ag-session",
                "hook_event_name": "PreToolUse",
                "tool_name": "run_command",
                "tool_input": {"command": "npm test"}
            }),
            serde_json::json!({
                "conversationId": "ag-session",
                "hook_event_name": "PostToolUse",
                "tool_name": "write_file",
                "tool_input": {"path": "app.ts"}
            }),
        ];
        std::fs::write(
            &path,
            lines
                .iter()
                .map(serde_json::Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();

        let events = adapter.parse_jsonl(&path, "fallback").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].session_id, "ag-session");
        assert_eq!(events[0].event_type, EventType::ToolPreCall);
        assert_eq!(events[0].payload["command"], "npm test");
        assert_eq!(events[1].payload["file_path"], "app.ts");
    }
}
