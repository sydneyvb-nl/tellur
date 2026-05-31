//! Generic adapter — CLI and HTTP event ingestion for any AI tool

use std::path::Path;

use anyhow::Result;

/// Generic adapter for capturing events from any source
pub struct GenericAdapter;

impl GenericAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Parse a JSONL event file into TraceGit events
    pub fn import_jsonl(&self, path: &Path) -> Result<Vec<tracegit_core::schema::types::TraceEvent>> {
        let content = std::fs::read_to_string(path)?;
        let mut events = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<tracegit_core::schema::types::TraceEvent>(line) {
                events.push(event);
            }
        }
        Ok(events)
    }
}
