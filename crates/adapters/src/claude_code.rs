//! Claude Code adapter — hook installer and transcript parser
//!
//! Installs TraceGit hooks into Claude Code's configuration and
//! parses Claude Code transcripts into TraceGit events.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use tracegit_core::adapter::{AgentAdapter, AdapterInfo, AdapterCapabilities};
use tracegit_core::schema::types::*;

/// Claude Code transcript entry
#[derive(Debug, Deserialize)]
pub struct ClaudeTranscriptEntry {
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_use: Option<ClaudeToolUse>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ClaudeToolUse {
    pub name: String,
    pub input: serde_json::Value,
    pub output: Option<serde_json::Value>,
}

pub struct ClaudeCodeAdapter {
    info: AdapterInfo,
}

impl ClaudeCodeAdapter {
    pub fn new() -> Self {
        Self {
            info: AdapterInfo {
                name: "claude-code".to_string(),
                version: "0.1.0".to_string(),
                tool_name: "Claude Code".to_string(),
            },
        }
    }

    /// Detect if Claude Code is installed
    pub fn detect_installation() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        let claude_config = PathBuf::from(home).join(".claude");

        if claude_config.exists() {
            Some(claude_config)
        } else {
            None
        }
    }

    /// Install hooks into Claude Code's settings
    pub fn install_hooks(repo_root: &Path) -> Result<()> {
        let settings_path = repo_root.join(".claude").join("settings.json");

        // Create .claude directory if it doesn't exist
        if let Some(parent) = settings_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Read existing settings or create new
        let mut settings: serde_json::Value = if settings_path.exists() {
            let content = std::fs::read_to_string(&settings_path)
                .context("Failed to read Claude Code settings")?;
            serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        // Add TraceGit hooks
        let tracegit_path = find_tracegit();

        let hooks = serde_json::json!({
            "hooks": {
                "afterToolUse": [
                    {
                        "command": format!("{} event --event-type file.write --session $TRACEGIT_SESSION", tracegit_path),
                        "match": "Write|Edit|MultiEdit"
                    }
                ],
                "afterCommand": [
                    {
                        "command": format!("{} event --event-type command.exec --session $TRACEGIT_SESSION", tracegit_path),
                    }
                ],
                "sessionStart": [
                    {
                        "command": format!("{} event --event-type session.start --session $TRACEGIT_SESSION", tracegit_path),
                    }
                ]
            }
        });

        // Merge hooks into settings
        if let Some(existing) = settings.get_mut("hooks") {
            // Merge with existing hooks
            if let (Some(existing_obj), Some(new_obj)) = (existing.as_object_mut(), hooks.as_object()) {
                for (key, value) in new_obj {
                    existing_obj.insert(key.clone(), value.clone());
                }
            }
        } else {
            settings["hooks"] = hooks["hooks"].clone();
        }

        // Write back
        let output = serde_json::to_string_pretty(&settings)?;
        std::fs::write(&settings_path, output)?;

        Ok(())
    }

    /// Parse a Claude Code transcript file into events
    pub fn parse_transcript(
        &self,
        transcript_path: &Path,
        session_id: &str,
    ) -> Result<Vec<TraceEvent>> {
        let content = std::fs::read_to_string(transcript_path)
            .context("Failed to read transcript")?;

        let entries: Vec<ClaudeTranscriptEntry> = content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();

        let mut events = Vec::new();

        for entry in entries {
            if let Some(tool) = &entry.tool_use {
                let event_type = match tool.name.as_str() {
                    "Write" | "Edit" | "MultiEdit" => EventType::FileWrite,
                    "Bash" | "Shell" => EventType::CommandExecution,
                    "Read" => EventType::FileRead,
                    "Search" => EventType::CodeSearch,
                    _ => EventType::Custom(tool.name.clone()),
                };

                let file_path = tool.input.get("file_path")
                    .or_else(|| tool.input.get("path"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                events.push(TraceEvent {
                    schema: "tracegit.event.v1".to_string(),
                    id: tracegit_core::schema::ids::generate_event_id(),
                    session_id: session_id.to_string(),
                    timestamp: entry.timestamp.unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
                    event_type,
                    actor: EventActor::Agent,
                    payload: serde_json::json!({
                        "tool": tool.name,
                        "input": tool.input,
                        "output": tool.output,
                        "file_path": file_path,
                    }),
                    redaction: None,
                    prev_hash: None,
                    event_hash: None,
                });
            }
        }

        Ok(events)
    }
}

#[async_trait::async_trait]
impl AgentAdapter for ClaudeCodeAdapter {
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
        // Events are captured via hooks, not actively pushed
        Ok(())
    }
}

/// Find the tracegit binary
fn find_tracegit() -> String {
    std::process::Command::new("which")
        .arg("tracegit")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "tracegit".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapter_info() {
        let adapter = ClaudeCodeAdapter::new();
        assert_eq!(adapter.info().name, "claude-code");
        assert_eq!(adapter.info().tool_name, "Claude Code");
        assert!(adapter.capabilities().supports_hooks);
    }

    #[test]
    fn test_parse_empty_transcript() {
        let adapter = ClaudeCodeAdapter::new();
        let dir = std::env::temp_dir().join("tracegit_test_transcript");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        std::fs::write(&path, "").unwrap();

        let events = adapter.parse_transcript(&path, "sess_test").unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_write_event() {
        let adapter = ClaudeCodeAdapter::new();
        let dir = std::env::temp_dir().join("tracegit_test_write");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");

        let entry = serde_json::json!({
            "tool_use": {
                "name": "Write",
                "input": {"file_path": "/tmp/test.rs", "content": "fn main() {}"},
                "output": {"success": true}
            },
            "timestamp": "2026-05-31T15:00:00Z"
        });
        std::fs::write(&path, entry.to_string()).unwrap();

        let events = adapter.parse_transcript(&path, "sess_test").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::FileWrite);
    }
}
