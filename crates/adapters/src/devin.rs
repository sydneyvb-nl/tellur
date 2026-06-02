//! Devin adapter — cloud agent session/run export import.
//!
//! Devin is an async cloud agent; its provenance value is the per-run audit
//! trail of messages, shell commands, and file edits. Exports arrive as a run
//! object wrapping an `events`/`messages` array, a bare array, or JSONL. This
//! module owns only the Devin-specific event-type mapping.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;
use tellur_core::adapter::{AdapterCapabilities, AdapterInfo, AgentAdapter};
use tellur_core::schema::types::*;

use crate::import::first_string;

const TOOL: &str = "devin";
const SESSION_ID_PATHS: &[&[&str]] = &[
    &["session_id"],
    &["sessionId"],
    &["devin_run_id"],
    &["run_id"],
    &["runId"],
    &["session", "id"],
];

pub struct DevinAdapter {
    info: AdapterInfo,
}

impl Default for DevinAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl DevinAdapter {
    pub fn new() -> Self {
        Self {
            info: AdapterInfo {
                name: "devin".to_string(),
                version: "0.1.0".to_string(),
                tool_name: "Devin".to_string(),
            },
        }
    }

    /// Parse a Devin run/session export (run object, array, or JSONL).
    pub fn parse_export(&self, path: &Path, fallback_session_id: &str) -> Result<Vec<TraceEvent>> {
        crate::import::parse_stream(
            path,
            "Devin",
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
            &["kind"],
            &["action"],
            &["role"],
        ],
    );

    match kind {
        Some("session_start" | "run_started" | "start") => EventType::SessionStart,
        Some("session_end" | "run_finished" | "run_completed" | "end") => EventType::SessionEnd,
        Some("user" | "user_message" | "prompt" | "message" | "request") => EventType::UserPrompt,
        Some(
            "shell" | "shell_command" | "command" | "run_command" | "exec" | "bash" | "terminal",
        ) => EventType::CommandExecution,
        Some(
            "edit" | "edit_file" | "file_write" | "write" | "write_file" | "create_file"
            | "apply_diff" | "patch",
        ) => EventType::FileWrite,
        Some("read" | "read_file" | "view_file" | "open") => EventType::FileRead,
        Some("pull_request" | "git_commit" | "commit") => {
            EventType::Custom("devin.git".to_string())
        }
        Some("assistant" | "devin_message" | "agent_message" | "ai_response") => {
            EventType::Custom("devin.response".to_string())
        }
        Some(other) => EventType::Custom(format!("devin.{other}")),
        None => EventType::Custom("devin.unknown".to_string()),
    }
}

#[async_trait::async_trait]
impl AgentAdapter for DevinAdapter {
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
        let adapter = DevinAdapter::new();
        assert_eq!(adapter.info().name, "devin");
        assert_eq!(adapter.info().tool_name, "Devin");
    }

    #[test]
    fn test_parse_devin_run_envelope() {
        let adapter = DevinAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_devin");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("run.json");
        let doc = serde_json::json!({
            "devin_run_id": "run-42",
            "messages": [
                {"type": "user", "message": "fix the failing test", "model": "devin"},
                {"type": "shell", "command": "pytest"},
                {"type": "edit", "file_path": "app/calc.py"}
            ]
        });
        std::fs::write(&path, doc.to_string()).unwrap();

        let events = adapter.parse_export(&path, "fallback").unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, EventType::UserPrompt);
        assert!(events[0].payload.get("prompt_hash").is_some());
        assert_eq!(events[1].event_type, EventType::CommandExecution);
        assert_eq!(events[1].payload["command"], "pytest");
        assert_eq!(events[2].event_type, EventType::FileWrite);
        assert_eq!(events[2].payload["file_path"], "app/calc.py");
    }
}
