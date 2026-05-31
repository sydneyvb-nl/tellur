//! Built-in adapter implementations

use std::path::Path;

use crate::adapter::{AdapterCapabilities, AgentAdapter, DetectionResult};
use crate::schema::types::TraceEvent;
use async_trait::async_trait;

/// Claude Code adapter — hooks into Claude Code's hook system
pub struct ClaudeCodeAdapter;

#[async_trait]
impl AgentAdapter for ClaudeCodeAdapter {
    fn id(&self) -> &str { "claude-code" }
    fn name(&self) -> &str { "Claude Code" }

    async fn detect(&self, workspace_path: &Path) -> DetectionResult {
        let settings = workspace_path.join(".claude").join("settings.json");
        DetectionResult {
            detected: settings.exists(),
            tool_name: "Claude Code".to_string(),
            version: None,
            config_path: if settings.exists() { Some(settings.to_string_lossy().to_string()) } else { None },
        }
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            session_lifecycle: true,
            prompt_capture: true,
            file_read_capture: true,
            file_write_capture: true,
            shell_command_capture: true,
            tool_call_capture: true,
            mcp_capture: true,
            model_metadata: true,
            cost_capture: false,
            test_result_capture: false,
            external_context_capture: true,
            branch_commit_capture: true,
            native_attribution_import: false,
        }
    }
}

/// Aider adapter — reads Aider's git commits and chat logs
pub struct AiderAdapter;

#[async_trait]
impl AgentAdapter for AiderAdapter {
    fn id(&self) -> &str { "aider" }
    fn name(&self) -> &str { "Aider" }

    async fn detect(&self, workspace_path: &Path) -> DetectionResult {
        let conf = workspace_path.join(".aider.conf.yml");
        DetectionResult {
            detected: conf.exists(),
            tool_name: "Aider".to_string(),
            version: None,
            config_path: if conf.exists() { Some(conf.to_string_lossy().to_string()) } else { None },
        }
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            session_lifecycle: false,
            prompt_capture: false,
            file_read_capture: false,
            file_write_capture: true,
            shell_command_capture: false,
            tool_call_capture: false,
            mcp_capture: false,
            model_metadata: true,
            cost_capture: false,
            test_result_capture: false,
            external_context_capture: false,
            branch_commit_capture: true,
            native_attribution_import: true,
        }
    }
}

/// Cursor adapter — imports Agent Trace JSON
pub struct CursorAdapter;

#[async_trait]
impl AgentAdapter for CursorAdapter {
    fn id(&self) -> &str { "cursor" }
    fn name(&self) -> &str { "Cursor" }

    async fn detect(&self, workspace_path: &Path) -> DetectionResult {
        let cursor_dir = workspace_path.join(".cursor");
        DetectionResult {
            detected: cursor_dir.exists(),
            tool_name: "Cursor".to_string(),
            version: None,
            config_path: if cursor_dir.exists() { Some(cursor_dir.to_string_lossy().to_string()) } else { None },
        }
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            session_lifecycle: true,
            prompt_capture: true,
            file_read_capture: true,
            file_write_capture: true,
            shell_command_capture: true,
            tool_call_capture: true,
            mcp_capture: false,
            model_metadata: true,
            cost_capture: false,
            test_result_capture: false,
            external_context_capture: true,
            branch_commit_capture: true,
            native_attribution_import: true,
        }
    }
}

/// Generic adapter — CLI and HTTP event ingestion for any tool
pub struct GenericAdapter;

#[async_trait]
impl AgentAdapter for GenericAdapter {
    fn id(&self) -> &str { "generic" }
    fn name(&self) -> &str { "Generic (CLI/HTTP)" }

    async fn detect(&self, _workspace_path: &Path) -> DetectionResult {
        // Generic adapter is always "detected" — it's the fallback
        DetectionResult {
            detected: true,
            tool_name: "Generic".to_string(),
            version: None,
            config_path: None,
        }
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            session_lifecycle: true,
            prompt_capture: true,
            file_read_capture: true,
            file_write_capture: true,
            shell_command_capture: true,
            tool_call_capture: true,
            mcp_capture: true,
            model_metadata: true,
            cost_capture: true,
            test_result_capture: true,
            external_context_capture: true,
            branch_commit_capture: true,
            native_attribution_import: false,
        }
    }
}

/// Get all built-in adapters
pub fn all_adapters() -> Vec<Box<dyn AgentAdapter>> {
    vec![
        Box::new(ClaudeCodeAdapter),
        Box::new(AiderAdapter),
        Box::new(CursorAdapter),
        Box::new(GenericAdapter),
    ]
}
