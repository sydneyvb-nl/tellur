//! Adapter interface for AI tool integrations
//!
//! Every AI tool adapter (Claude Code, Cursor, Aider, etc.) implements
//! the AgentAdapter trait to normalize events into TraceGit format.

use std::path::Path;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::schema::types::{Session, TraceEvent};

/// Result of detecting an AI tool in a workspace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionResult {
    pub detected: bool,
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
}

/// Capabilities that an adapter supports
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterCapabilities {
    pub session_lifecycle: bool,
    pub prompt_capture: bool,
    pub file_read_capture: bool,
    pub file_write_capture: bool,
    pub shell_command_capture: bool,
    pub tool_call_capture: bool,
    pub mcp_capture: bool,
    pub model_metadata: bool,
    pub cost_capture: bool,
    pub test_result_capture: bool,
    pub external_context_capture: bool,
    pub branch_commit_capture: bool,
    pub native_attribution_import: bool,
}

/// Adapter for a specific AI coding tool
#[async_trait]
pub trait AgentAdapter: Send + Sync {
    /// Unique identifier for this adapter
    fn id(&self) -> &str;

    /// Human-readable name
    fn name(&self) -> &str;

    /// Detect if this tool is present in the workspace
    async fn detect(&self, workspace_path: &Path) -> DetectionResult;

    /// Install hooks/integration (optional)
    async fn install(&self, workspace_path: &Path) -> anyhow::Result<()> {
        Ok(())
    }

    /// Uninstall hooks/integration (optional)
    async fn uninstall(&self, workspace_path: &Path) -> anyhow::Result<()> {
        Ok(())
    }

    /// Import existing data from this tool
    async fn import(&self, source: &Path) -> anyhow::Result<Vec<TraceEvent>> {
        Ok(Vec::new())
    }

    /// List what this adapter can capture
    fn capabilities(&self) -> AdapterCapabilities;
}

/// Built-in adapters that ship with TraceGit
pub mod builtin;
