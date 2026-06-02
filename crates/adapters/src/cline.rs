//! Cline / Roo Code adapter — VS Code agent task-history import.
//!
//! Cline and its fork Roo Code persist each task as JSON under the extension's
//! `tasks/<id>/` storage: `ui_messages.json` (an array of `say`/`ask` messages,
//! each with a numeric `ts`) and `api_conversation_history.json` (an array of
//! role-tagged API messages). Both share this format, so one adapter covers
//! both. This module owns only the Cline-specific event-type mapping; the shared
//! [`crate::import`] loop handles the array/JSONL reading and numeric timestamps.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;
use tellur_core::adapter::{AdapterCapabilities, AdapterInfo, AgentAdapter};
use tellur_core::schema::types::*;

use crate::import::first_string;

const TOOL: &str = "cline";
const SESSION_ID_PATHS: &[&[&str]] = &[
    &["taskId"],
    &["task_id"],
    &["sessionId"],
    &["session_id"],
    &["conversationId"],
];

pub struct ClineAdapter {
    info: AdapterInfo,
}

impl Default for ClineAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ClineAdapter {
    pub fn new() -> Self {
        Self {
            info: AdapterInfo {
                name: "cline".to_string(),
                version: "0.1.0".to_string(),
                tool_name: "Cline / Roo Code".to_string(),
            },
        }
    }

    /// Parse a Cline/Roo Code task file (`ui_messages.json`,
    /// `api_conversation_history.json`, array, or JSONL).
    pub fn parse_task(&self, path: &Path, fallback_session_id: &str) -> Result<Vec<TraceEvent>> {
        crate::import::parse_stream(
            path,
            "Cline / Roo Code",
            TOOL,
            fallback_session_id,
            SESSION_ID_PATHS,
            event_type,
        )
    }
}

fn event_type(raw: &Value) -> EventType {
    // `ui_messages.json` carries the most specific signal in `say`/`ask`.
    if let Some(say) = first_string(raw, &[&["say"]]) {
        return match say {
            "user_feedback" => EventType::UserPrompt,
            "command" | "command_output" => EventType::CommandExecution,
            "tool" | "browser_action" => EventType::Custom("cline.tool".to_string()),
            "api_req_started" | "api_req_finished" | "api_req_retried" => {
                EventType::Custom("cline.api_req".to_string())
            }
            "text" | "reasoning" | "completion_result" => {
                EventType::Custom("cline.response".to_string())
            }
            other => EventType::Custom(format!("cline.say.{other}")),
        };
    }
    if let Some(ask) = first_string(raw, &[&["ask"]]) {
        return match ask {
            "command" => EventType::CommandExecution,
            "tool" | "use_mcp_server" => EventType::ToolPreCall,
            "followup" | "resume_task" | "resume_completed_task" => EventType::UserPrompt,
            other => EventType::Custom(format!("cline.ask.{other}")),
        };
    }
    // `api_conversation_history.json` carries a role instead.
    if let Some(role) = first_string(raw, &[&["role"]]) {
        return match role {
            "user" => EventType::UserPrompt,
            "assistant" => EventType::Custom("cline.response".to_string()),
            other => EventType::Custom(format!("cline.{other}")),
        };
    }
    let kind = first_string(raw, &[&["type"], &["event"], &["kind"]]);
    match kind {
        Some("command") => EventType::CommandExecution,
        Some(other) => EventType::Custom(format!("cline.{other}")),
        None => EventType::Custom("cline.unknown".to_string()),
    }
}

#[async_trait::async_trait]
impl AgentAdapter for ClineAdapter {
    fn info(&self) -> &AdapterInfo {
        &self.info
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            can_capture_file_writes: false,
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
        let adapter = ClineAdapter::new();
        assert_eq!(adapter.info().name, "cline");
        assert_eq!(adapter.info().tool_name, "Cline / Roo Code");
    }

    #[test]
    fn test_parse_cline_ui_messages() {
        let adapter = ClineAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_cline");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ui_messages.json");
        let doc = serde_json::json!([
            {"ts": 1_700_000_000_000_i64, "type": "say", "say": "user_feedback", "text": "build the CLI"},
            {"ts": 1_700_000_001_000_i64, "type": "ask", "ask": "command", "text": "npm run build"},
            {"ts": 1_700_000_002_000_i64, "type": "say", "say": "text", "text": "Done."}
        ]);
        std::fs::write(&path, doc.to_string()).unwrap();

        let events = adapter.parse_task(&path, "task-7").unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].session_id, "task-7");
        assert_eq!(events[0].event_type, EventType::UserPrompt);
        assert!(events[0].payload.get("prompt_hash").is_some());
        assert!(events[0].timestamp.starts_with("2023-11-"));
        assert_eq!(events[1].event_type, EventType::CommandExecution);
        assert_eq!(
            events[2].event_type,
            EventType::Custom("cline.response".to_string())
        );
    }

    #[test]
    fn test_parse_cline_api_history_roles() {
        let adapter = ClineAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_cline_api");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("api_conversation_history.json");
        let doc = serde_json::json!([
            {"role": "user", "content": "secret=abcdefghijklmnopqrstuvwxyz12345"},
            {"role": "assistant", "content": "ok"}
        ]);
        std::fs::write(&path, doc.to_string()).unwrap();

        let events = adapter.parse_task(&path, "task-9").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, EventType::UserPrompt);
        assert_eq!(
            events[1].event_type,
            EventType::Custom("cline.response".to_string())
        );
        let serialized = serde_json::to_string(&events[0].payload).unwrap();
        assert!(!serialized.contains("abcdefghijklmnopqrstuvwxyz12345"));
    }
}
