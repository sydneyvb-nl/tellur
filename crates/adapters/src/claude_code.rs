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

impl Default for ClaudeCodeAdapter {
    fn default() -> Self {
        Self::new()
    }
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

    /// Install TraceGit hooks into the repository's Claude Code settings
    /// (`.claude/settings.json`), using Claude Code's real hook schema:
    /// `PostToolUse` matchers and `SessionStart`, each invoking
    /// `tracegit hooks claude`, which reads the hook JSON from stdin.
    pub fn install_hooks(repo_root: &Path) -> Result<()> {
        let settings_path = repo_root.join(".claude").join("settings.json");
        if let Some(parent) = settings_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut settings: serde_json::Value = if settings_path.exists() {
            let content = std::fs::read_to_string(&settings_path)
                .context("Failed to read Claude Code settings")?;
            serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        let tracegit = find_tracegit();
        let command = format!("{} hooks claude", tracegit);

        // Ensure settings.hooks is an object.
        if !settings.get("hooks").map(|h| h.is_object()).unwrap_or(false) {
            settings["hooks"] = serde_json::json!({});
        }
        let hooks = settings["hooks"].as_object_mut().unwrap();

        // PostToolUse — fire after file-editing tools.
        let post_entry = serde_json::json!({
            "matcher": "Write|Edit|MultiEdit",
            "hooks": [ { "type": "command", "command": command } ]
        });
        merge_hook_array(hooks, "PostToolUse", post_entry, &command);

        // SessionStart — record the start of an AI session.
        let start_entry = serde_json::json!({
            "hooks": [ { "type": "command", "command": command } ]
        });
        merge_hook_array(hooks, "SessionStart", start_entry, &command);

        std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)?;
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

/// Parsed Claude Code hook payload (delivered on stdin to a hook command).
#[derive(Debug, Default, Deserialize)]
pub struct HookPayload {
    pub session_id: Option<String>,
    pub hook_event_name: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub cwd: Option<String>,
}

impl HookPayload {
    /// Parse a hook payload from JSON (typically read from stdin).
    pub fn parse(json: &str) -> Result<Self> {
        Ok(serde_json::from_str(json).unwrap_or_default())
    }

    /// File path touched by the tool, if any.
    pub fn file_path(&self) -> Option<String> {
        self.tool_input.as_ref().and_then(|v| {
            v.get("file_path")
                .or_else(|| v.get("path"))
                .and_then(|p| p.as_str())
                .map(|s| s.to_string())
        })
    }
}

/// Insert a hook entry into the array for `key`, unless an entry already invokes
/// the same command (idempotent re-install).
fn merge_hook_array(
    hooks: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    entry: serde_json::Value,
    command: &str,
) {
    let arr = hooks
        .entry(key.to_string())
        .or_insert_with(|| serde_json::json!([]));
    if !arr.is_array() {
        *arr = serde_json::json!([]);
    }
    let already = arr
        .as_array()
        .map(|items| {
            items.iter().any(|item| {
                item.get("hooks")
                    .and_then(|h| h.as_array())
                    .map(|hs| {
                        hs.iter().any(|h| {
                            h.get("command").and_then(|c| c.as_str()) == Some(command)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    if !already {
        arr.as_array_mut().unwrap().push(entry);
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
