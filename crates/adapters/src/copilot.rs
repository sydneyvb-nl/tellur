//! GitHub Copilot adapter — metadata event import.

use std::path::Path;

use anyhow::{Context, Result};
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
        let content = std::fs::read_to_string(path)?;
        let entries: Vec<serde_json::Value> = match serde_json::from_str(&content) {
            Ok(entries) => entries,
            Err(array_err) => {
                let mut entries = Vec::new();
                for (idx, line) in content.lines().enumerate() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    entries.push(serde_json::from_str(line).with_context(|| {
                        format!(
                            "invalid Copilot metadata JSON/JSONL at line {} (array parse failed: {})",
                            idx + 1,
                            array_err
                        )
                    })?);
                }
                entries
            }
        };

        let events = entries
            .into_iter()
            .map(|entry| {
                let event_type = copilot_event_type(&entry);
                let payload = copilot_payload(&entry);
                TraceEvent {
                    schema: "tellur.event.v1".to_string(),
                    id: entry
                        .get("id")
                        .or_else(|| entry.get("event_id"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(tellur_core::schema::ids::generate_event_id),
                    session_id: entry
                        .get("session_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(session_id)
                        .to_string(),
                    timestamp: entry
                        .get("timestamp")
                        .or_else(|| entry.get("time"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                    event_type,
                    actor: EventActor::Agent,
                    payload,
                    redaction: None,
                    prev_hash: None,
                    event_hash: None,
                }
            })
            .collect();
        Ok(events)
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

fn copilot_payload(entry: &serde_json::Value) -> serde_json::Value {
    let file_path = entry
        .get("file_path")
        .or_else(|| entry.get("file"))
        .or_else(|| entry.get("path"))
        .cloned();
    let mut payload = serde_json::json!({
        "tool": "github-copilot",
    });
    for key in [
        "language",
        "model",
        "completion_id",
        "suggestion_id",
        "prompt_hash",
        "command",
    ] {
        if let Some(value) = entry.get(key).cloned() {
            payload[key] = crate::sanitize::sanitized_value(&value);
        }
    }
    if let Some(file_path) = file_path {
        payload["file_path"] = crate::sanitize::sanitized_value(&file_path);
    }
    if let Some(prompt_hash) = crate::sanitize::first_prompt_hash(entry) {
        payload["prompt_hash"] = serde_json::Value::String(prompt_hash);
    }
    payload
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
}
