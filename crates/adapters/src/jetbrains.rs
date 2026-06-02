//! JetBrains AI Assistant / Junie adapter — action log JSON/JSONL import.
//!
//! Covers the AI Assistant plugin and the Junie autonomous agent across IntelliJ
//! IDEA, PyCharm, WebStorm, GoLand, and related JetBrains IDEs. Exports arrive as
//! a JSON array, an envelope object, or JSONL; this module owns only the
//! JetBrains-specific event-type mapping.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;
use tellur_core::adapter::{AdapterCapabilities, AdapterInfo, AgentAdapter};
use tellur_core::schema::types::*;

use crate::import::first_string;

const TOOL: &str = "jetbrains-ai";
const SESSION_ID_PATHS: &[&[&str]] = &[
    &["chatId"],
    &["chat_id"],
    &["conversationId"],
    &["conversation_id"],
    &["sessionId"],
    &["session_id"],
    &["session", "id"],
];

pub struct JetBrainsAdapter {
    info: AdapterInfo,
}

impl Default for JetBrainsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl JetBrainsAdapter {
    pub fn new() -> Self {
        Self {
            info: AdapterInfo {
                name: "jetbrains".to_string(),
                version: "0.1.0".to_string(),
                tool_name: "JetBrains AI / Junie".to_string(),
            },
        }
    }

    /// Parse a JetBrains AI Assistant / Junie export (JSON array, envelope, or JSONL).
    pub fn parse_export(&self, path: &Path, fallback_session_id: &str) -> Result<Vec<TraceEvent>> {
        crate::import::parse_stream(
            path,
            "JetBrains AI",
            TOOL,
            fallback_session_id,
            SESSION_ID_PATHS,
            event_type,
        )
    }
}

fn event_type(raw: &Value) -> EventType {
    let kind = first_string(
        raw,
        &[
            &["type"],
            &["event"],
            &["eventType"],
            &["event_type"],
            &["kind"],
            &["action"],
            &["role"],
        ],
    );

    match kind {
        Some("session_start" | "start") => EventType::SessionStart,
        Some("session_end" | "end") => EventType::SessionEnd,
        Some("user" | "prompt" | "chat" | "user_message" | "userMessage" | "question") => {
            EventType::UserPrompt
        }
        Some(
            "code_generation" | "codeGeneration" | "insert_code" | "apply" | "accept"
            | "accept_suggestion" | "file_write" | "edit" | "edit_file",
        ) => EventType::FileWrite,
        Some("read" | "read_file" | "open_file") => EventType::FileRead,
        Some("command" | "run" | "run_command" | "terminal") => EventType::CommandExecution,
        Some("assistant" | "ai_response" | "answer" | "completion") => {
            EventType::Custom("jetbrains.response".to_string())
        }
        Some(other) => EventType::Custom(format!("jetbrains.{other}")),
        None => EventType::Custom("jetbrains.unknown".to_string()),
    }
}

#[async_trait::async_trait]
impl AgentAdapter for JetBrainsAdapter {
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
        let adapter = JetBrainsAdapter::new();
        assert_eq!(adapter.info().name, "jetbrains");
        assert!(adapter.capabilities().can_capture_prompts);
    }

    #[test]
    fn test_parse_jetbrains_array_export() {
        let adapter = JetBrainsAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_jetbrains");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ai-assistant.json");
        let doc = serde_json::json!([
            {
                "chatId": "jb-chat",
                "type": "user",
                "text": "explain this function",
                "model": "anthropic-claude"
            },
            {
                "chatId": "jb-chat",
                "type": "accept_suggestion",
                "file_path": "Main.java"
            }
        ]);
        std::fs::write(&path, doc.to_string()).unwrap();

        let events = adapter.parse_export(&path, "fallback").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].session_id, "jb-chat");
        assert_eq!(events[0].event_type, EventType::UserPrompt);
        assert!(events[0].payload.get("prompt_hash").is_some());
        assert_eq!(events[1].event_type, EventType::FileWrite);
        assert_eq!(events[1].payload["file_path"], "Main.java");
    }

    #[test]
    fn test_parse_jetbrains_envelope() {
        let adapter = JetBrainsAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_jetbrains_envelope");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("junie.json");
        let doc = serde_json::json!({
            "sessionId": "junie-1",
            "events": [
                {"type": "run_command", "command": "./gradlew test"}
            ]
        });
        std::fs::write(&path, doc.to_string()).unwrap();

        let events = adapter.parse_export(&path, "fallback").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::CommandExecution);
        assert_eq!(events[0].payload["command"], "./gradlew test");
    }
}
