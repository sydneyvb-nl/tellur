//! Windsurf / Cascade adapter — agent session JSONL/JSON import.
//!
//! Windsurf's Cascade agent exports tool calls, file edits, terminal commands,
//! and chat turns. Field names vary across releases, so parsing is delegated to
//! the tolerant shared [`crate::import`] loop and this module only owns the
//! Windsurf-specific event-type mapping.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;
use tellur_core::adapter::{AdapterCapabilities, AdapterInfo, AgentAdapter};
use tellur_core::schema::types::*;

use crate::import::first_string;

const TOOL: &str = "windsurf";
const SESSION_ID_PATHS: &[&[&str]] = &[
    &["cascadeId"],
    &["cascade_id"],
    &["conversationId"],
    &["conversation_id"],
    &["session_id"],
    &["sessionId"],
    &["session", "id"],
];

pub struct WindsurfAdapter {
    info: AdapterInfo,
}

impl Default for WindsurfAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl WindsurfAdapter {
    pub fn new() -> Self {
        Self {
            info: AdapterInfo {
                name: "windsurf".to_string(),
                version: "0.1.0".to_string(),
                tool_name: "Windsurf".to_string(),
            },
        }
    }

    /// Parse a Windsurf/Cascade session export (JSONL, JSON array, or envelope).
    pub fn parse_jsonl(&self, path: &Path, fallback_session_id: &str) -> Result<Vec<TraceEvent>> {
        crate::import::parse_stream(
            path,
            "Windsurf",
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
            &["event_type"],
            &["eventName"],
            &["kind"],
            &["action"],
        ],
    );
    let tool = first_string(raw, &[&["tool"], &["tool_name"], &["toolName"]]);

    match kind.or(tool) {
        Some("session_start" | "start" | "cascade_start") => EventType::SessionStart,
        Some("session_end" | "end" | "cascade_end") => EventType::SessionEnd,
        Some("user_message" | "user_prompt" | "prompt" | "prompt_submitted" | "message") => {
            EventType::UserPrompt
        }
        Some("tool_call_start" | "pre_tool" | "before_tool") => EventType::ToolPreCall,
        Some("tool_call_end" | "post_tool" | "after_tool") => EventType::ToolPostCall,
        Some(
            "write_file" | "edit_file" | "edit" | "file_edit" | "propose_code" | "apply_diff"
            | "write_to_file" | "write",
        ) => EventType::FileWrite,
        Some("read_file" | "view_file" | "view_code_item" | "read") => EventType::FileRead,
        Some("run_command" | "run_terminal_cmd" | "terminal" | "command") => {
            EventType::CommandExecution
        }
        Some("assistant_message" | "cascade_response" | "agent_message") => {
            EventType::Custom("windsurf.response".to_string())
        }
        Some(other) => EventType::Custom(format!("windsurf.{other}")),
        None => EventType::Custom("windsurf.unknown".to_string()),
    }
}

#[async_trait::async_trait]
impl AgentAdapter for WindsurfAdapter {
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
        let adapter = WindsurfAdapter::new();
        assert_eq!(adapter.info().name, "windsurf");
        assert_eq!(adapter.info().tool_name, "Windsurf");
        assert!(adapter.capabilities().can_capture_commands);
    }

    #[test]
    fn test_parse_windsurf_session() {
        let adapter = WindsurfAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_windsurf");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cascade.jsonl");
        let lines = [
            serde_json::json!({
                "cascadeId": "cascade-1",
                "type": "user_message",
                "message": "refactor the parser",
                "model": "windsurf-swe-1"
            }),
            serde_json::json!({
                "cascadeId": "cascade-1",
                "type": "tool_call_end",
                "tool": "write_file",
                "file_path": "src/parser.rs"
            }),
            serde_json::json!({
                "cascadeId": "cascade-1",
                "type": "run_command",
                "command": "cargo build"
            }),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        std::fs::write(&path, lines).unwrap();

        let events = adapter.parse_jsonl(&path, "fallback").unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].session_id, "cascade-1");
        assert_eq!(events[0].event_type, EventType::UserPrompt);
        assert!(events[0].payload.get("prompt_hash").is_some());
        assert_eq!(events[0].payload["model"], "windsurf-swe-1");
        assert_eq!(events[1].event_type, EventType::ToolPostCall);
        assert_eq!(events[1].payload["file_path"], "src/parser.rs");
        assert_eq!(events[2].event_type, EventType::CommandExecution);
        assert_eq!(events[2].payload["command"], "cargo build");
    }
}
