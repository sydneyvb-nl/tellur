//! Claude Code adapter
//!
//! Integrates with Claude Code's hook system to capture
//! tool calls, file edits, commands, and session events.

use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use tracegit_core::schema::types::TraceEvent;

/// Claude Code adapter for TraceGit
pub struct ClaudeCodeAdapter;

impl ClaudeCodeAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Install Claude Code hooks for event capture
    pub fn install_hooks(&self, workspace_path: &Path) -> Result<()> {
        // Hook installation will create a .claude/hooks/ directory
        // with scripts that emit TraceGit events
        let hooks_dir = workspace_path.join(".claude").join("hooks");
        std::fs::create_dir_all(&hooks_dir)?;

        // Pre-tool-use hook
        let pre_tool_hook = r#"#!/bin/bash
# TraceGit Claude Code Hook — Pre Tool Use
# Emits a tracegit event before each tool call
tracegit event --event-type tool.pre_call --session "$TRACEGIT_SESSION" --command "$TOOL_NAME"
"#;
        std::fs::write(hooks_dir.join("tracegit-pre-tool.sh"), pre_tool_hook)?;

        // Post-tool-use hook
        let post_tool_hook = r#"#!/bin/bash
# TraceGit Claude Code Hook — Post Tool Use
# Emits a tracegit event after each tool call
tracegit event --event-type tool.post_call --session "$TRACEGIT_SESSION" --command "$TOOL_NAME"
"#;
        std::fs::write(hooks_dir.join("tracegit-post-tool.sh"), post_tool_hook)?;

        Ok(())
    }

    /// Import Claude Code transcript
    pub fn import_transcript(&self, transcript_path: &Path) -> Result<Vec<TraceEvent>> {
        // Transcript import will parse Claude Code JSONL transcripts
        // and convert them to TraceGit events
        Ok(Vec::new())
    }
}
