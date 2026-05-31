//! Cursor adapter — Agent Trace import parser
//!
//! Parses Cursor's Agent Trace JSON format into TraceGit events.
//! Cursor stores traces in .cursor/trace/ or exports them as JSON.

use std::path::Path;

use anyhow::Result;

use tracegit_core::adapter::{AgentAdapter, AdapterInfo, AdapterCapabilities};
use tracegit_core::schema::types::*;

/// Cursor Agent Trace entry
#[derive(Debug, serde::Deserialize)]
pub struct CursorTraceEntry {
    pub id: Option<String>,
    pub timestamp: Option<String>,
    pub kind: Option<String>,
    pub tool: Option<String>,
    pub input: Option<serde_json::Value>,
    pub output: Option<serde_json::Value>,
    pub model: Option<String>,
    pub file_path: Option<String>,
    pub duration_ms: Option<u64>,
}

pub struct CursorAdapter {
    info: AdapterInfo,
}

impl CursorAdapter {
    pub fn new() -> Self {
        Self {
            info: AdapterInfo {
                name: "cursor".to_string(),
                version: "0.1.0".to_string(),
                tool_name: "Cursor".to_string(),
            },
        }
    }

    /// Detect Cursor workspace
    pub fn detect_workspace(repo_root: &Path) -> bool {
        repo_root.join(".cursor").exists()
    }

    /// Parse a Cursor Agent Trace file
    pub fn parse_trace_file(
        &self,
        trace_path: &Path,
        session_id: &str,
    ) -> Result<Vec<TraceEvent>> {
        let content = std::fs::read_to_string(trace_path)?;
        let entries: Vec<CursorTraceEntry> = serde_json::from_str(&content)
            .unwrap_or_else(|_| {
                // Try JSONL format
                content
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .filter_map(|l| serde_json::from_str(l).ok())
                    .collect()
            });

        let mut events = Vec::new();

        for entry in entries {
            let event_type = match entry.kind.as_deref().or(entry.tool.as_deref()) {
                Some("edit" | "write" | "apply") => EventType::FileWrite,
                Some("read") => EventType::FileRead,
                Some("search" | "codebase_search") => EventType::CodeSearch,
                Some("terminal" | "bash" | "command") => EventType::CommandExecution,
                Some("chat" | "prompt") => EventType::UserPrompt,
                Some(other) => EventType::Custom(other.to_string()),
                None => continue,
            };

            let model = entry.model.unwrap_or("cursor".to_string());

            events.push(TraceEvent {
                schema: "tracegit.event.v1".to_string(),
                id: entry.id.unwrap_or_else(|| tracegit_core::schema::ids::generate_event_id()),
                session_id: session_id.to_string(),
                timestamp: entry.timestamp.unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                event_type,
                actor: EventActor::Agent,
                payload: serde_json::json!({
                    "tool": entry.tool,
                    "input": entry.input,
                    "output": entry.output,
                    "file_path": entry.file_path,
                    "model": model,
                    "duration_ms": entry.duration_ms,
                }),
                redaction: None,
                prev_hash: None,
                event_hash: None,
            });
        }

        Ok(events)
    }
}

#[async_trait::async_trait]
impl AgentAdapter for CursorAdapter {
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
    use std::path::PathBuf;

    #[test]
    fn test_adapter_info() {
        let adapter = CursorAdapter::new();
        assert_eq!(adapter.info().name, "cursor");
        assert!(!adapter.capabilities().supports_hooks);
    }

    #[test]
    fn test_parse_trace_json() {
        let adapter = CursorAdapter::new();
        let dir = std::env::temp_dir().join("tracegit_test_cursor");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("trace.json");

        let trace = serde_json::json!([
            {
                "id": "trc_1",
                "timestamp": "2026-05-31T15:00:00Z",
                "kind": "edit",
                "tool": "applyEdit",
                "input": {"file_path": "src/main.ts", "newText": "console.log('hello')"},
                "model": "cursor-small",
                "duration_ms": 1200
            },
            {
                "id": "trc_2",
                "timestamp": "2026-05-31T15:00:05Z",
                "kind": "terminal",
                "tool": "runCommand",
                "input": {"command": "npm test"},
                "model": "cursor-small"
            }
        ]);
        std::fs::write(&path, trace.to_string()).unwrap();

        let events = adapter.parse_trace_file(&path, "sess_cursor").unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event_type, EventType::FileWrite);
        assert_eq!(events[1].event_type, EventType::CommandExecution);
    }

    #[test]
    fn test_detect_workspace() {
        let dir = std::env::temp_dir().join("tracegit_test_detect");
        let _ = std::fs::create_dir_all(dir.join(".cursor"));
        assert!(CursorAdapter::detect_workspace(&dir));
    }
}
