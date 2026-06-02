//! Continue adapter — `.continue/dev_data` development-data import.
//!
//! Continue (VS Code and JetBrains) logs development data as JSONL files such as
//! `chat.jsonl`, `autocomplete.jsonl`, and `editInteraction.jsonl`, where each
//! line carries a `name` and a nested `data` object. This module owns only the
//! Continue-specific event-type mapping; the shared [`crate::import`] loop reads
//! the JSONL and lifts nested `data.*` fields into the payload.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;
use tellur_core::adapter::{AdapterCapabilities, AdapterInfo, AgentAdapter};
use tellur_core::schema::types::*;

use crate::import::first_string;

const TOOL: &str = "continue";
const SESSION_ID_PATHS: &[&[&str]] = &[
    &["sessionId"],
    &["session_id"],
    &["data", "sessionId"],
    &["data", "session_id"],
    &["conversationId"],
];

pub struct ContinueAdapter {
    info: AdapterInfo,
}

impl Default for ContinueAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ContinueAdapter {
    pub fn new() -> Self {
        Self {
            info: AdapterInfo {
                name: "continue".to_string(),
                version: "0.1.0".to_string(),
                tool_name: "Continue".to_string(),
            },
        }
    }

    /// Parse a Continue `dev_data` JSONL file (or array export).
    pub fn parse_jsonl(&self, path: &Path, fallback_session_id: &str) -> Result<Vec<TraceEvent>> {
        crate::import::parse_stream(
            path,
            "Continue",
            TOOL,
            fallback_session_id,
            SESSION_ID_PATHS,
            event_type,
        )
    }
}

fn event_type(raw: &Value) -> EventType {
    // Continue dev_data keys the event on `name`; older streams use `type`.
    let kind = first_string(raw, &[&["name"], &["eventName"], &["event"], &["type"]]);

    match kind {
        Some("chat" | "chatInteraction" | "chatFeedback" | "userMessage" | "prompt") => {
            EventType::UserPrompt
        }
        Some("autocomplete" | "autocompleteFeedback") => {
            EventType::Custom("continue.autocomplete".to_string())
        }
        Some(
            "editInteraction"
            | "editOutcome"
            | "applyToFile"
            | "acceptEdit"
            | "quickEdit"
            | "nextEdit"
            | "nextEditOutcome"
            | "nextEditWithHistory",
        ) => EventType::FileWrite,
        Some("tokensGenerated") => EventType::Custom("continue.tokens".to_string()),
        Some("command" | "runCommand" | "toolUse" | "tool_call") => EventType::CommandExecution,
        Some(other) => EventType::Custom(format!("continue.{other}")),
        None => EventType::Custom("continue.unknown".to_string()),
    }
}

#[async_trait::async_trait]
impl AgentAdapter for ContinueAdapter {
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
        let adapter = ContinueAdapter::new();
        assert_eq!(adapter.info().name, "continue");
        assert_eq!(adapter.info().tool_name, "Continue");
    }

    #[test]
    fn test_parse_continue_dev_data() {
        let adapter = ContinueAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_continue");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("editInteraction.jsonl");
        let lines = [
            serde_json::json!({
                "name": "chat",
                "data": {"sessionId": "cont-1", "prompt": "add a docstring"}
            }),
            serde_json::json!({
                "name": "editInteraction",
                "data": {
                    "sessionId": "cont-1",
                    "filepath": "lib/util.py",
                    "modelTitle": "claude-3.7-sonnet"
                }
            }),
        ]
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        std::fs::write(&path, lines).unwrap();

        let events = adapter.parse_jsonl(&path, "fallback").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].session_id, "cont-1");
        assert_eq!(events[0].event_type, EventType::UserPrompt);
        assert!(events[0].payload.get("prompt_hash").is_some());
        assert_eq!(events[1].event_type, EventType::FileWrite);
        assert_eq!(events[1].payload["file_path"], "lib/util.py");
        assert_eq!(events[1].payload["model"], "claude-3.7-sonnet");
    }

    #[test]
    fn test_parse_continue_next_edit_with_history() {
        let adapter = ContinueAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_continue_nextedit");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("nextEditWithHistory.jsonl");
        std::fs::write(
            &path,
            serde_json::json!({
                "name": "nextEditWithHistory",
                "data": {"sessionId": "cont-2", "fileURI": "file:///repo/src/app.ts"}
            })
            .to_string(),
        )
        .unwrap();

        let events = adapter.parse_jsonl(&path, "fallback").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::FileWrite);
        assert_eq!(events[0].payload["file_path"], "file:///repo/src/app.ts");
    }
}
