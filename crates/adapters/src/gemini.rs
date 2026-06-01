//! Gemini CLI adapter — hook/stream JSONL import parser.

use std::path::Path;

use anyhow::{Context, Result};
use tellur_core::adapter::{AdapterCapabilities, AdapterInfo, AgentAdapter};
use tellur_core::schema::types::*;

pub struct GeminiAdapter {
    info: AdapterInfo,
}

impl Default for GeminiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl GeminiAdapter {
    pub fn new() -> Self {
        Self {
            info: AdapterInfo {
                name: "gemini-cli".to_string(),
                version: "0.1.0".to_string(),
                tool_name: "Gemini CLI".to_string(),
            },
        }
    }

    /// Parse Gemini CLI stream-json, telemetry JSONL, or hook-like JSONL.
    ///
    /// Gemini CLI event schemas evolve quickly, so this normalizes stable
    /// concepts and keeps sanitized raw payload for audit context.
    pub fn parse_jsonl(&self, path: &Path, fallback_session_id: &str) -> Result<Vec<TraceEvent>> {
        let content = std::fs::read_to_string(path)?;
        let mut events = Vec::new();
        let mut session_id = fallback_session_id.to_string();

        for (idx, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let raw = serde_json::from_str::<serde_json::Value>(line)
                .with_context(|| format!("invalid Gemini CLI JSONL at line {}", idx + 1))?;
            if let Some(id) = first_string(
                &raw,
                &[
                    &["session_id"],
                    &["sessionId"],
                    &["session", "id"],
                    &["conversationId"],
                    &["conversation_id"],
                ],
            ) {
                session_id = id.to_string();
            }
            events.push(TraceEvent {
                schema: "tellur.event.v1".to_string(),
                id: first_string(&raw, &[&["id"], &["event_id"], &["eventId"]])
                    .map(ToString::to_string)
                    .unwrap_or_else(tellur_core::schema::ids::generate_event_id),
                session_id: session_id.clone(),
                timestamp: first_string(&raw, &[&["timestamp"], &["time"], &["created_at"]])
                    .map(ToString::to_string)
                    .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                event_type: google_agent_event_type(&raw, "gemini-cli"),
                actor: EventActor::Agent,
                payload: google_agent_payload(&raw, "gemini-cli"),
                redaction: None,
                prev_hash: None,
                event_hash: None,
            });
        }

        Ok(events)
    }
}

pub(crate) fn google_agent_event_type(raw: &serde_json::Value, prefix: &str) -> EventType {
    let kind = first_string(
        raw,
        &[
            &["hook_event_name"],
            &["event"],
            &["event_name"],
            &["eventName"],
            &["type"],
            &["kind"],
        ],
    );
    let tool = first_string(raw, &[&["tool_name"], &["toolName"], &["tool", "name"]]);

    match kind.or(tool) {
        Some("SessionStart" | "session_start" | "start") => EventType::SessionStart,
        Some("SessionEnd" | "AfterAgent" | "session_end" | "end") => EventType::SessionEnd,
        Some("BeforeAgent" | "BeforeModel" | "user_prompt" | "prompt" | "prompt_submitted") => {
            EventType::UserPrompt
        }
        Some("BeforeTool" | "PreToolUse") => EventType::ToolPreCall,
        Some("AfterTool" | "PostToolUse") => EventType::ToolPostCall,
        Some("write_file" | "replace" | "edit" | "file_write" | "write") => EventType::FileWrite,
        Some("read_file" | "read") => EventType::FileRead,
        Some("run_command" | "run_shell_command" | "shell" | "command") => {
            EventType::CommandExecution
        }
        Some(other) => EventType::Custom(format!("{prefix}.{other}")),
        None => EventType::Custom(format!("{prefix}.unknown")),
    }
}

pub(crate) fn google_agent_payload(raw: &serde_json::Value, tool_name: &str) -> serde_json::Value {
    let mut out = serde_json::json!({
        "tool": tool_name,
        "raw_payload": crate::sanitize::sanitized_value(raw),
    });
    if let Some(prompt_hash) = crate::sanitize::first_prompt_hash(raw) {
        out["prompt_hash"] = serde_json::Value::String(prompt_hash);
    }
    if let Some(model) = first_string(raw, &[&["model"], &["model_id"], &["modelId"]]) {
        out["model"] = serde_json::Value::String(model.to_string());
    }
    if let Some(command) = first_string(
        raw,
        &[
            &["command"],
            &["cmd"],
            &["tool_input", "command"],
            &["toolInput", "command"],
            &["tool", "input", "command"],
        ],
    ) {
        out["command"] = crate::sanitize::sanitized_value(&serde_json::json!(command));
    }
    if let Some(file_path) = first_string(
        raw,
        &[
            &["file_path"],
            &["filePath"],
            &["path"],
            &["tool_input", "file_path"],
            &["tool_input", "path"],
            &["toolInput", "filePath"],
            &["tool", "input", "file_path"],
            &["tool", "input", "path"],
        ],
    ) {
        out["file_path"] = crate::sanitize::sanitized_value(&serde_json::json!(file_path));
    }
    out
}

pub(crate) fn first_string<'a>(value: &'a serde_json::Value, paths: &[&[&str]]) -> Option<&'a str> {
    paths
        .iter()
        .filter_map(|path| json_path(value, path))
        .find_map(|value| value.as_str())
}

fn json_path<'a>(mut value: &'a serde_json::Value, path: &[&str]) -> Option<&'a serde_json::Value> {
    for key in path {
        value = value.get(*key)?;
    }
    Some(value)
}

#[async_trait::async_trait]
impl AgentAdapter for GeminiAdapter {
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
        let adapter = GeminiAdapter::new();
        assert_eq!(adapter.info().name, "gemini-cli");
        assert!(adapter.capabilities().supports_hooks);
    }

    #[test]
    fn test_parse_gemini_jsonl_hashes_prompt_and_detects_tool() {
        let adapter = GeminiAdapter::new();
        let dir = std::env::temp_dir().join("tellur_test_gemini");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        let lines = [
            serde_json::json!({
                "session_id": "gemini-session",
                "event": "BeforeAgent",
                "prompt": "write tests",
                "model": "gemini-2.5-pro"
            }),
            serde_json::json!({
                "session_id": "gemini-session",
                "event": "AfterTool",
                "tool_name": "write_file",
                "tool_input": {"file_path": "src/main.rs", "content": "secret=abc"}
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
        assert_eq!(events[0].session_id, "gemini-session");
        assert_eq!(events[0].event_type, EventType::UserPrompt);
        assert!(events[0].payload.get("prompt_hash").is_some());
        assert_eq!(events[1].event_type, EventType::ToolPostCall);
        assert_eq!(events[1].payload["file_path"], "src/main.rs");
        assert!(events[1].payload.to_string().contains("hash"));
    }
}
