//! SQLite index for fast queries over event and attribution data

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::schema::types::TraceEvent;

/// Serialize a serde enum to its string value
fn enum_to_str<T: serde::Serialize>(val: &T) -> String {
    serde_json::to_value(val)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default()
}

/// SQLite-backed index for TraceGit data
pub struct TraceIndex {
    conn: Connection,
}

impl TraceIndex {
    /// Open or create the index at the given path
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)
            .context("Failed to open SQLite index")?;

        let index = Self { conn };
        index.init_tables()?;
        Ok(index)
    }

    /// Create an in-memory index (for testing)
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let index = Self { conn };
        index.init_tables()?;
        Ok(index)
    }

    /// Initialize database tables
    fn init_tables(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                type TEXT NOT NULL,
                actor TEXT NOT NULL,
                payload TEXT NOT NULL,
                prev_hash TEXT,
                event_hash TEXT,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id);
            CREATE INDEX IF NOT EXISTS idx_events_type ON events(type);
            CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                repo_id TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                agent_id TEXT NOT NULL,
                agent_name TEXT NOT NULL,
                model_name TEXT,
                status TEXT NOT NULL,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS attributions (
                file_path TEXT NOT NULL,
                git_blob_sha TEXT NOT NULL,
                range_id TEXT PRIMARY KEY,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                origin TEXT NOT NULL,
                evidence_strength TEXT NOT NULL,
                confidence REAL NOT NULL,
                state TEXT NOT NULL,
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                model_id TEXT,
                policy_tags TEXT,
                risk_tags TEXT,
                risk_level TEXT,
                tests_run TEXT DEFAULT '[]',
                tests_passed BOOLEAN,
                reviewer TEXT,
                reviewed_at TEXT,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_attributions_file ON attributions(file_path);
            CREATE INDEX IF NOT EXISTS idx_attributions_session ON attributions(session_id);
            CREATE INDEX IF NOT EXISTS idx_attributions_origin ON attributions(origin);
            "
        )?;
        Ok(())
    }

    /// Index an event
    pub fn index_event(&self, event: &TraceEvent) -> Result<()> {
        let payload_str = serde_json::to_string(&event.payload)?;
        let event_type_str = enum_to_str(&event.event_type);
        let actor_str = enum_to_str(&event.actor);

        self.conn.execute(
            "INSERT OR IGNORE INTO events (id, session_id, timestamp, type, actor, payload, prev_hash, event_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event.id,
                event.session_id,
                event.timestamp,
                event_type_str,
                actor_str,
                payload_str,
                event.prev_hash,
                event.event_hash,
            ],
        )?;

        // Auto-create session if not exists
        self.conn.execute(
            "INSERT OR IGNORE INTO sessions (id, repo_id, started_at, agent_id, agent_name, status)\n             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.session_id,
                "local",
                event.timestamp,
                actor_str,
                actor_str,
                "active",
            ],
        )?;

        Ok(())
    }

    /// Get events for a session
    pub fn get_session_events(&self, session_id: &str) -> Result<Vec<TraceEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, timestamp, type, actor, payload, prev_hash, event_hash
             FROM events WHERE session_id = ?1 ORDER BY timestamp"
        )?;

        let events = stmt.query_map(params![session_id], |row| {
            let type_str: String = row.get(3)?;
            let actor_str: String = row.get(4)?;
            let payload_str: String = row.get(5)?;

            Ok(TraceEvent {
                schema: "tracegit.event.v1".to_string(),
                id: row.get(0)?,
                session_id: row.get(1)?,
                timestamp: row.get(2)?,
                event_type: serde_json::from_value(serde_json::Value::String(type_str))
                    .unwrap_or(crate::schema::types::EventType::FileWrite),
                actor: serde_json::from_value(serde_json::Value::String(actor_str))
                    .unwrap_or(crate::schema::types::EventActor::Unknown),
                payload: serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null),
                redaction: None,
                prev_hash: row.get(6)?,
                event_hash: row.get(7)?,
            })
        })?;

        events.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Count total events
    pub fn event_count(&self) -> Result<u64> {
        let count: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM events",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Count sessions
    pub fn session_count(&self) -> Result<u64> {
        let count: u64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sessions",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    /// Index an attribution range
    pub fn index_attribution(&self, attr: &crate::schema::types::AttributionRange, file_path: &str, blob_sha: &str, updated_at: &str) -> Result<()> {
        let policy_tags = serde_json::to_string(&attr.policy_tags)?;
        let risk_tags = serde_json::to_string(&attr.risk_tags)?;

        self.conn.execute(
            "INSERT OR REPLACE INTO attributions
             (file_path, git_blob_sha, range_id, start_line, end_line, origin, evidence_strength,
              confidence, state, session_id, agent_id, model_id, policy_tags, risk_tags,
              risk_level, tests_run, tests_passed, reviewer, reviewed_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
            params![
                file_path,
                blob_sha,
                attr.range_id,
                attr.start_line,
                attr.end_line,
                enum_to_str(&attr.origin),
                enum_to_str(&attr.evidence_strength),
                attr.confidence,
                enum_to_str(&attr.state),
                attr.session_id,
                attr.agent_id,
                attr.model_id,
                policy_tags,
                risk_tags,
                attr.risk_level.as_ref().map(enum_to_str),
                serde_json::to_string(&attr.tests_run).unwrap_or_else(|_| "[]".to_string()),
                attr.tests_passed,
                attr.reviewer,
                attr.reviewed_at,
                updated_at,
            ],
        )?;
        Ok(())
    }

    /// Get attribution for a specific file
    pub fn get_file_attributions(&self, file_path: &str) -> Result<Vec<(String, crate::schema::types::AttributionRange)>> {
        let mut stmt = self.conn.prepare(
            "SELECT git_blob_sha, range_id, start_line, end_line, origin, evidence_strength,
                    confidence, state, session_id, agent_id, model_id, policy_tags, risk_tags,
                    risk_level, tests_run, tests_passed, reviewer, reviewed_at
             FROM attributions WHERE file_path = ?1 ORDER BY start_line"
        )?;

        let results = stmt.query_map(params![file_path], |row| {
            let origin_str: String = row.get(4)?;
            let evidence_str: String = row.get(5)?;
            let state_str: String = row.get(7)?;
            let policy_tags_str: String = row.get(11)?;
            let risk_tags_str: String = row.get(12)?;
            let risk_level_str: Option<String> = row.get(13)?;
            let tests_run_str: String = row.get(14)?;

            Ok((
                row.get::<_, String>(0)?, // blob_sha
                crate::schema::types::AttributionRange {
                    range_id: row.get(1)?,
                    start_line: row.get(2)?,
                    end_line: row.get(3)?,
                    origin: serde_json::from_value(serde_json::Value::String(origin_str)).unwrap_or(crate::schema::types::Origin::Unknown),
                    evidence_strength: serde_json::from_value(serde_json::Value::String(evidence_str)).unwrap_or(crate::schema::types::EvidenceStrength::Unknown),
                    confidence: row.get(6)?,
                    state: serde_json::from_value(serde_json::Value::String(state_str)).unwrap_or(crate::schema::types::AttributionState::Uncertain),
                    session_id: row.get(8)?,
                    event_ids: vec![],
                    agent_id: row.get(9)?,
                    model_id: row.get(10)?,
                    prompt_hash: None,
                    context_set_id: None,
                    policy_tags: serde_json::from_str(&policy_tags_str).unwrap_or_default(),
                    risk_tags: serde_json::from_str(&risk_tags_str).unwrap_or_default(),
                    risk_level: risk_level_str.and_then(|s| serde_json::from_value(serde_json::Value::String(s)).ok()),
                    tests_run: serde_json::from_str(&tests_run_str).unwrap_or_default(),
                    tests_passed: row.get(15)?,
                    reviewer: row.get(16)?,
                    reviewed_at: row.get(17)?,
                },
            ))
        })?;

        results.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::types::{EventActor, EventType};

    #[test]
    fn test_index_and_retrieve_events() {
        let index = TraceIndex::open_in_memory().unwrap();

        let event = TraceEvent {
            schema: "tracegit.event.v1".to_string(),
            id: "evt_test_001".to_string(),
            session_id: "sess_test".to_string(),
            timestamp: "2026-05-31T14:00:00Z".to_string(),
            event_type: EventType::FileWrite,
            actor: EventActor::Agent,
            payload: serde_json::json!({"file": "src/main.rs"}),
            redaction: None,
            prev_hash: None,
            event_hash: Some("abc123".to_string()),
        };

        index.index_event(&event).unwrap();

        let events = index.get_session_events("sess_test").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, "evt_test_001");
    }

    #[test]
    fn test_event_count() {
        let index = TraceIndex::open_in_memory().unwrap();
        assert_eq!(index.event_count().unwrap(), 0);
    }
}
