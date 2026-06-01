//! Generic adapter — CLI and HTTP event ingestion for any AI tool

use std::path::Path;

use anyhow::{Context, Result};

/// Generic adapter for capturing events from any source
pub struct GenericAdapter;

impl Default for GenericAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl GenericAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Parse a JSONL event file into Tellur events
    pub fn import_jsonl(&self, path: &Path) -> Result<Vec<tellur_core::schema::types::TraceEvent>> {
        let content = std::fs::read_to_string(path)?;
        let mut events = Vec::new();
        for (idx, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let event = serde_json::from_str::<tellur_core::schema::types::TraceEvent>(line)
                .with_context(|| format!("invalid Tellur event JSONL at line {}", idx + 1))?;
            events.push(event);
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_import_jsonl_rejects_invalid_lines() {
        let dir = std::env::temp_dir().join("tellur_test_generic_invalid");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        std::fs::write(&path, "{not-json}\n").unwrap();

        let err = GenericAdapter::new().import_jsonl(&path).unwrap_err();
        assert!(err.to_string().contains("line 1"));
    }
}
