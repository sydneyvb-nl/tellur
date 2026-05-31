//! JSONL event log — append-only, tamper-evident storage
//!
//! Events are stored as one JSON line per event in date-partitioned files.
//! Each event links to the previous via a SHA-256 hash chain.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;

use crate::schema::types::TraceEvent;
use crate::schema::ids::{self, hash_event};

/// Append-only JSONL event writer with hash chain
pub struct EventWriter {
    log_dir: PathBuf,
    current_file: Option<File>,
    last_hash: Option<String>,
}

impl EventWriter {
    /// Create a new EventWriter that stores events in the given directory
    pub fn new(log_dir: impl Into<PathBuf>) -> Self {
        Self {
            log_dir: log_dir.into(),
            current_file: None,
            last_hash: Option::None,
        }
    }

    /// Open the writer, creating the directory if needed
    pub fn open(&mut self) -> Result<()> {
        fs::create_dir_all(&self.log_dir)
            .context("Failed to create event log directory")?;

        // Find the last hash from existing logs for chain continuity
        self.last_hash = self.find_last_hash()?;

        // Open today's log file
        let log_file = self.today_log_path();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
            .context("Failed to open event log file")?;
        self.current_file = Some(file);

        Ok(())
    }

    /// Write an event to the log, returning the complete event with hashes
    pub fn write_event(
        &mut self,
        session_id: &str,
        event_type: &str,
        actor: &str,
        payload: serde_json::Value,
        redaction: Option<crate::schema::types::RedactionInfo>,
    ) -> Result<TraceEvent> {
        let file = self
            .current_file
            .as_mut()
            .context("EventWriter not open. Call open() first.")?;

        let event_id = ids::generate_event_id();
        let timestamp = Utc::now().to_rfc3339();

        let event_hash = hash_event(
            &event_id,
            session_id,
            &timestamp,
            event_type,
            actor,
            &payload,
            self.last_hash.as_deref(),
        );

        let event = TraceEvent {
            schema: "tracegit.event.v1".to_string(),
            id: event_id,
            session_id: session_id.to_string(),
            timestamp,
            event_type: serde_json::from_value(serde_json::Value::String(event_type.to_string()))
                .unwrap_or(crate::schema::types::EventType::FileWrite),
            actor: serde_json::from_value(serde_json::Value::String(actor.to_string()))
                .unwrap_or(crate::schema::types::EventActor::Agent),
            payload,
            redaction,
            prev_hash: self.last_hash.clone(),
            event_hash: Some(event_hash.clone()),
        };

        let line = serde_json::to_string(&event)? + "\n";
        file.write_all(line.as_bytes())
            .context("Failed to write event to log")?;
        file.flush()?;

        self.last_hash = Some(event_hash);
        Ok(event)
    }

    /// Close the writer
    pub fn close(&mut self) {
        self.current_file = None;
    }

    /// Get the path for today's log file
    fn today_log_path(&self) -> PathBuf {
        let date = Utc::now().format("%Y-%m-%d").to_string();
        self.log_dir.join(format!("events-{}.jsonl", date))
    }

    /// Find the last event hash from existing log files
    fn find_last_hash(&self) -> Result<Option<String>> {
        let mut files: Vec<PathBuf> = Vec::new();
        if self.log_dir.exists() {
            for entry in fs::read_dir(&self.log_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "jsonl") {
                    files.push(path);
                }
            }
        }
        files.sort();

        if let Some(last_file) = files.last() {
            let last_line = read_last_line(last_file)?;
            if let Some(line) = last_line {
                let event: TraceEvent = serde_json::from_str(&line)?;
                return Ok(event.event_hash);
            }
        }
        Ok(None)
    }
}

/// Read events from a JSONL log directory
pub fn read_events(log_dir: &Path) -> Result<Vec<TraceEvent>> {
    let mut events = Vec::new();
    if !log_dir.exists() {
        return Ok(events);
    }

    let mut files: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(log_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map_or(false, |e| e == "jsonl") {
            files.push(path);
        }
    }
    files.sort();

    for file in files {
        let reader = BufReader::new(File::open(&file)?);
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<TraceEvent>(&line) {
                events.push(event);
            }
        }
    }

    Ok(events)
}

/// Read the last non-empty line from a file
fn read_last_line(path: &Path) -> Result<Option<String>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut last_line: Option<String> = None;
    for line in reader.lines() {
        let line = line?;
        if !line.trim().is_empty() {
            last_line = Some(line);
        }
    }
    Ok(last_line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_write_and_read_events() {
        let tmp = TempDir::new().unwrap();
        let log_dir = tmp.path().join("events");

        let mut writer = EventWriter::new(&log_dir);
        writer.open().unwrap();

        let event1 = writer
            .write_event(
                "sess_test",
                "session.start",
                "agent",
                serde_json::json!({"tool": "claude-code"}),
                None,
            )
            .unwrap();

        let event2 = writer
            .write_event(
                "sess_test",
                "file.write",
                "agent",
                serde_json::json!({"file": "src/main.rs"}),
                None,
            )
            .unwrap();

        // Chain should be linked
        assert!(event1.prev_hash.is_none());
        assert!(event2.prev_hash.is_some());
        assert_eq!(event2.prev_hash.as_deref(), event1.event_hash.as_deref());

        writer.close();

        // Read back
        let events = read_events(&log_dir).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].id, event1.id);
        assert_eq!(events[1].id, event2.id);
    }

    #[test]
    fn test_hash_chain_continuity() {
        let tmp = TempDir::new().unwrap();
        let log_dir = tmp.path().join("events");

        // Write first event
        let mut writer = EventWriter::new(&log_dir);
        writer.open().unwrap();
        let event1 = writer
            .write_event("sess_1", "session.start", "agent", serde_json::json!({}), None)
            .unwrap();
        writer.close();

        // Open again and write second — should continue chain
        let mut writer2 = EventWriter::new(&log_dir);
        writer2.open().unwrap();
        let event2 = writer2
            .write_event("sess_2", "session.start", "agent", serde_json::json!({}), None)
            .unwrap();

        assert_eq!(event2.prev_hash, event1.event_hash);
    }

    #[test]
    fn test_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let events = read_events(tmp.path()).unwrap();
        assert!(events.is_empty());
    }
}
