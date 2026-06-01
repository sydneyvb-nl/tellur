//! JSONL event log — append-only, tamper-evident storage
//!
//! Events are stored as one JSON line per event in date-partitioned files.
//! Each event links to the previous via a SHA-256 hash chain.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;

/// Cross-process advisory lock for the event log directory.
///
/// Held only for the duration of a single append so that concurrent writers
/// (e.g. `tellur watch` and editor/CLI `tellur event` calls fired by hooks)
/// cannot fork or corrupt the hash chain.
struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn acquire_lock(dir: &Path) -> Result<LockGuard> {
    let path = dir.join(".write.lock");
    let start = std::time::Instant::now();
    loop {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(LockGuard { path }),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                // Reclaim a stale lock left behind by a crashed process.
                if let Ok(meta) = fs::metadata(&path)
                    && let Ok(age) = meta
                        .modified()
                        .and_then(|m| m.elapsed().map_err(|_| std::io::Error::other("clock")))
                    && age > Duration::from_secs(30)
                {
                    let _ = fs::remove_file(&path);
                    continue;
                }
                if start.elapsed() > Duration::from_secs(10) {
                    return Err(anyhow!("Timed out acquiring event log lock at {:?}", path));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

use crate::schema::ids::{self, hash_event};
use crate::schema::types::TraceEvent;

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
        fs::create_dir_all(&self.log_dir).context("Failed to create event log directory")?;

        // Find the last hash from existing logs for chain continuity
        self.last_hash = self.find_last_hash()?;

        // Open today's log file
        let log_file = self.append_log_path()?;
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
        // Serialise appends across processes and refresh the chain tip from
        // disk so the hash chain stays linear even with concurrent writers.
        let _lock = acquire_lock(&self.log_dir)?;
        self.last_hash = self.find_last_hash()?;

        let file = self
            .current_file
            .as_mut()
            .context("EventWriter not open. Call open() first.")?;

        let event_id = ids::generate_event_id();
        let timestamp = Utc::now().to_rfc3339();

        // Unknown event types round-trip through `Custom` instead of being
        // silently coerced. Unknown actors default to `Unknown` (not `Agent`),
        // so an unspecified actor is never mislabelled as the AI agent.
        let parsed_event_type = crate::schema::types::EventType::from_wire(event_type);
        let parsed_actor: crate::schema::types::EventActor =
            serde_json::from_value(serde_json::Value::String(actor.to_string()))
                .unwrap_or(crate::schema::types::EventActor::Unknown);

        // Use the canonical wire strings for hashing (deterministic).
        let event_type_for_hash = parsed_event_type.as_wire();
        let actor_for_hash = serde_json::to_value(&parsed_actor)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| actor.to_string());

        let event_hash = hash_event(
            &event_id,
            session_id,
            &timestamp,
            &event_type_for_hash,
            &actor_for_hash,
            &payload,
            self.last_hash.as_deref(),
        );

        let event = TraceEvent {
            schema: "tellur.event.v1".to_string(),
            id: event_id,
            session_id: session_id.to_string(),
            timestamp,
            event_type: parsed_event_type,
            actor: parsed_actor,
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

    /// Append an imported event while preserving its source identity and
    /// timestamp. The hash-chain fields are always recomputed for the local log.
    pub fn write_imported_event(&mut self, mut event: TraceEvent) -> Result<TraceEvent> {
        let _lock = acquire_lock(&self.log_dir)?;
        self.last_hash = self.find_last_hash()?;

        let event_type_for_hash = event.event_type.as_wire();
        let actor_for_hash = serde_json::to_value(&event.actor)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        let event_hash = hash_event(
            &event.id,
            &event.session_id,
            &event.timestamp,
            &event_type_for_hash,
            &actor_for_hash,
            &event.payload,
            self.last_hash.as_deref(),
        );

        event.prev_hash = self.last_hash.clone();
        event.event_hash = Some(event_hash.clone());

        let log_file = self.today_log_path();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)
            .context("Failed to open imported event log file")?;
        let line = serde_json::to_string(&event)? + "\n";
        file.write_all(line.as_bytes())
            .context("Failed to write imported event to log")?;
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

    fn append_log_path(&self) -> Result<PathBuf> {
        let today = self.today_log_path();
        let mut files: Vec<PathBuf> = Vec::new();
        if self.log_dir.exists() {
            for entry in fs::read_dir(&self.log_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "jsonl") {
                    files.push(path);
                }
            }
        }
        files.push(today);
        files.sort();
        Ok(files.pop().unwrap_or_else(|| self.today_log_path()))
    }

    /// Find the last event hash from existing log files
    fn find_last_hash(&self) -> Result<Option<String>> {
        let mut files: Vec<PathBuf> = Vec::new();
        if self.log_dir.exists() {
            for entry in fs::read_dir(&self.log_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "jsonl") {
                    files.push(path);
                }
            }
        }
        files.sort();

        if let Some(last_file) = files.last() {
            let last_line = read_last_line(last_file)?;
            if let Some(line) = last_line {
                if let Ok(event) = serde_json::from_str::<TraceEvent>(&line) {
                    return Ok(event.event_hash);
                }
                // Corrupted last line — try earlier lines
                let file = File::open(last_file)?;
                let reader = BufReader::new(file);
                let mut last_good_hash = None;
                for line in reader.lines() {
                    let line = line?;
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(event) = serde_json::from_str::<TraceEvent>(&line) {
                        last_good_hash = event.event_hash;
                    }
                }
                return Ok(last_good_hash);
            }
        }
        Ok(None)
    }
}

/// Re-seal the hash chain across all logs in `log_dir`.
///
/// Reads every event in order, recomputes `prev_hash`/`event_hash`, and rewrites
/// the date-partitioned log files. Used after a legitimate, in-place mutation
/// such as `tellur redact`: the original chain necessarily breaks when content
/// changes (that's the point of tamper-evidence), so re-sealing produces a clean
/// chain that attests the post-redaction state. Returns the number of events.
pub fn reseal_chain(log_dir: &Path) -> Result<usize> {
    use std::collections::BTreeMap;

    let _lock = acquire_lock(log_dir)?;
    let mut events = read_events(log_dir)?;

    let mut prev: Option<String> = None;
    let mut by_file: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for ev in events.iter_mut() {
        let actor_wire = serde_json::to_value(&ev.actor)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string());
        let hash = hash_event(
            &ev.id,
            &ev.session_id,
            &ev.timestamp,
            &ev.event_type.as_wire(),
            &actor_wire,
            &ev.payload,
            prev.as_deref(),
        );
        ev.prev_hash = prev.clone();
        ev.event_hash = Some(hash.clone());
        prev = Some(hash);

        let date = ev.timestamp.get(0..10).unwrap_or("unknown").to_string();
        by_file
            .entry(format!("events-{}.jsonl", date))
            .or_default()
            .push(serde_json::to_string(ev)?);
    }

    // Replace existing logs with the re-sealed ones.
    for entry in fs::read_dir(log_dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "jsonl") {
            fs::remove_file(&path)?;
        }
    }
    for (fname, lines) in by_file {
        fs::write(log_dir.join(fname), lines.join("\n") + "\n")?;
    }

    Ok(events.len())
}

/// Outcome of verifying a hash chain.
#[derive(Debug, Clone, Default)]
pub struct ChainVerification {
    pub valid: usize,
    pub broken: usize,
    /// Human-readable descriptions of each broken link.
    pub problems: Vec<String>,
}

/// Verify a sequence of events: each event's hash recomputes correctly and the
/// `prev_hash` links form an unbroken chain. Shared by `tellur verify` and
/// the MCP `tellur_verify` tool.
pub fn verify_chain(events: &[TraceEvent]) -> ChainVerification {
    let mut result = ChainVerification::default();
    let mut prev_hash: Option<&str> = None;

    for event in events {
        let mut ok = true;

        if let Some(stored_hash) = event.event_hash.as_deref() {
            let recomputed = hash_event(
                &event.id,
                &event.session_id,
                &event.timestamp,
                &event.event_type.as_wire(),
                &serde_json::to_value(&event.actor)
                    .ok()
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .unwrap_or_default(),
                &event.payload,
                event.prev_hash.as_deref(),
            );
            if recomputed != stored_hash {
                ok = false;
                result
                    .problems
                    .push(format!("Hash mismatch at event {} (tampered?)", event.id));
            }
        }

        if let Some(prev) = prev_hash
            && event.prev_hash.as_deref() != Some(prev)
        {
            ok = false;
            result
                .problems
                .push(format!("Chain broken at event {}", event.id));
        }

        if ok {
            result.valid += 1;
        } else {
            result.broken += 1;
        }
        prev_hash = event.event_hash.as_deref();
    }

    result
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
        if path.extension().is_some_and(|e| e == "jsonl") {
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
            .write_event(
                "sess_1",
                "session.start",
                "agent",
                serde_json::json!({}),
                None,
            )
            .unwrap();
        writer.close();

        // Open again and write second — should continue chain
        let mut writer2 = EventWriter::new(&log_dir);
        writer2.open().unwrap();
        let event2 = writer2
            .write_event(
                "sess_2",
                "session.start",
                "agent",
                serde_json::json!({}),
                None,
            )
            .unwrap();

        assert_eq!(event2.prev_hash, event1.event_hash);
    }

    #[test]
    fn test_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let events = read_events(tmp.path()).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_verify_chain_detects_tampering() {
        let tmp = TempDir::new().unwrap();
        let log_dir = tmp.path().join("events");
        let mut writer = EventWriter::new(&log_dir);
        writer.open().unwrap();
        writer
            .write_event("s", "session.start", "agent", serde_json::json!({}), None)
            .unwrap();
        writer
            .write_event(
                "s",
                "file.write",
                "agent",
                serde_json::json!({"file": "a"}),
                None,
            )
            .unwrap();
        writer.close();

        let mut events = read_events(&log_dir).unwrap();
        // Clean chain verifies.
        let ok = verify_chain(&events);
        assert_eq!(ok.broken, 0);
        assert_eq!(ok.valid, 2);

        // Tamper with a payload — hash should no longer match.
        events[1].payload = serde_json::json!({"file": "evil"});
        let bad = verify_chain(&events);
        assert!(bad.broken >= 1);
    }
}
