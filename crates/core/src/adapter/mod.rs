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

/// Info about an adapter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterInfo {
    pub name: String,
    pub version: String,
    pub tool_name: String,
}

/// Capabilities that an adapter supports
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterCapabilities {
    pub can_capture_file_writes: bool,
    pub can_capture_commands: bool,
    pub can_capture_prompts: bool,
    pub can_replay_session: bool,
    pub supports_hooks: bool,
}

/// Adapter for a specific AI coding tool
#[async_trait]
pub trait AgentAdapter: Send + Sync {
    /// Adapter info
    fn info(&self) -> &AdapterInfo;

    /// List what this adapter can capture
    fn capabilities(&self) -> AdapterCapabilities;

    /// Detect if this tool is present in the workspace
    fn detect(&self, _workspace_path: &Path) -> DetectionResult {
        DetectionResult {
            detected: false,
            tool_name: self.info().tool_name.clone(),
            version: None,
            config_path: None,
        }
    }

    /// Install hooks/integration
    fn install(&self, _workspace_path: &Path) -> anyhow::Result<()> {
        Ok(())
    }

    /// Start a tracking session
    async fn start_session(&self, session: &Session) -> anyhow::Result<String>;

    /// End a tracking session
    async fn end_session(&self, session_id: &str) -> anyhow::Result<()>;

    /// Capture an event
    async fn capture_event(&self, event: &TraceEvent) -> anyhow::Result<()>;
}

/// Built-in adapters that ship with TraceGit
pub mod builtin;
