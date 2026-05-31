//! SQLite index for fast queries over event and attribution data

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use serde::{Deserialize, Serialize};

use crate::schema::types::TraceEvent;

/// Summary row for a session, used by `tracegit sessions --json`, the MCP
/// server, and the editor extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub agent_id: String,
    pub agent_name: String,
    pub model_name: Option<String>,
    pub status: String,
    pub event_count: u64,
}

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
                event_count INTEGER NOT NULL DEFAULT 0,
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
                event_ids TEXT DEFAULT '[]',
                agent_id TEXT NOT NULL,
                model_id TEXT,
                prompt_hash TEXT,
                context_set_id TEXT,
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

        // Ensure a session row exists (a minimal placeholder until the real
        // Session is indexed via `index_session`), and keep its event count
        // current. We never overwrite richer fields populated by index_session.
        self.conn.execute(
            "INSERT OR IGNORE INTO sessions (id, repo_id, started_at, agent_id, agent_name, status, event_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![
                event.session_id,
                "unknown",
                event.timestamp,
                "unknown",
                "unknown",
                "active",
            ],
        )?;
        self.conn.execute(
            "UPDATE sessions SET event_count = (SELECT COUNT(*) FROM events WHERE session_id = ?1) WHERE id = ?1",
            params![event.session_id],
        )?;

        Ok(())
    }

    /// Index (insert or update) a full session record with agent/model metadata.
    pub fn index_session(&self, session: &crate::schema::types::Session) -> Result<()> {
        let model_name = session.model.as_ref().map(|m| {
            match &m.version {
                Some(v) => format!("{}:{} ({})", m.provider, m.name, v),
                None => format!("{}:{}", m.provider, m.name),
            }
        });
        self.conn.execute(
            "INSERT INTO sessions (id, repo_id, started_at, ended_at, agent_id, agent_name, model_name, status, event_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, COALESCE((SELECT event_count FROM sessions WHERE id = ?1), 0))
             ON CONFLICT(id) DO UPDATE SET
                repo_id = excluded.repo_id,
                started_at = excluded.started_at,
                ended_at = excluded.ended_at,
                agent_id = excluded.agent_id,
                agent_name = excluded.agent_name,
                model_name = excluded.model_name,
                status = excluded.status",
            params![
                session.id,
                session.repo_id,
                session.started_at,
                session.ended_at,
                session.agent.id,
                session.agent.name,
                model_name,
                enum_to_str(&session.status),
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

    /// List sessions, newest first.
    pub fn list_sessions(&self, limit: u32) -> Result<Vec<SessionSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, started_at, ended_at, agent_id, agent_name, model_name, status, event_count
             FROM sessions ORDER BY started_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                started_at: row.get(1)?,
                ended_at: row.get(2)?,
                agent_id: row.get(3)?,
                agent_name: row.get(4)?,
                model_name: row.get(5)?,
                status: row.get(6)?,
                event_count: row.get::<_, i64>(7)? as u64,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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

        let event_ids = serde_json::to_string(&attr.event_ids).unwrap_or_else(|_| "[]".to_string());

        self.conn.execute(
            "INSERT OR REPLACE INTO attributions
             (file_path, git_blob_sha, range_id, start_line, end_line, origin, evidence_strength,
              confidence, state, session_id, event_ids, agent_id, model_id, prompt_hash, context_set_id,
              policy_tags, risk_tags, risk_level, tests_run, tests_passed, reviewer, reviewed_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
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
                event_ids,
                attr.agent_id,
                attr.model_id,
                attr.prompt_hash,
                attr.context_set_id,
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

    /// Remove existing attribution ranges for a file (called before
    /// re-indexing the current state so ranges don't accumulate over captures).
    pub fn clear_file_attributions(&self, file_path: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM attributions WHERE file_path = ?1",
            params![file_path],
        )?;
        Ok(())
    }

    /// Get attribution for a specific file
    pub fn get_file_attributions(&self, file_path: &str) -> Result<Vec<(String, crate::schema::types::AttributionRange)>> {
        let mut stmt = self.conn.prepare(
            "SELECT git_blob_sha, range_id, start_line, end_line, origin, evidence_strength,
                    confidence, state, session_id, agent_id, model_id, policy_tags, risk_tags,
                    risk_level, tests_run, tests_passed, reviewer, reviewed_at, event_ids,
                    prompt_hash, context_set_id
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
            let event_ids_str: String = row.get(18)?;

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
                    event_ids: serde_json::from_str(&event_ids_str).unwrap_or_default(),
                    agent_id: row.get(9)?,
                    model_id: row.get(10)?,
                    prompt_hash: row.get(19)?,
                    context_set_id: row.get(20)?,
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

    #[test]
    fn test_index_session_populates_model_and_listing() {
        use crate::schema::types::*;
        let index = TraceIndex::open_in_memory().unwrap();
        let mut session = Session::new(
            "repo1".to_string(),
            Actor { name: "dev".to_string(), email: None, email_hash: None, actor_type: EventActor::Human },
            AgentInfo { id: "claude-code".to_string(), name: "Claude Code".to_string(), version: None },
        );
        session.model = Some(ModelInfo {
            provider: "anthropic".to_string(),
            name: "claude-opus".to_string(),
            version: Some("4.8".to_string()),
        });
        index.index_session(&session).unwrap();

        let sessions = index.list_sessions(10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].agent_name, "Claude Code");
        // The bug this guards: model_name used to always be NULL.
        assert_eq!(sessions[0].model_name.as_deref(), Some("anthropic:claude-opus (4.8)"));
    }

    #[test]
    fn test_attribution_round_trip_preserves_prompt_hash() {
        use crate::schema::types::*;
        let index = TraceIndex::open_in_memory().unwrap();
        let range = AttributionRange {
            range_id: "rng1".to_string(),
            start_line: 1,
            end_line: 5,
            origin: Origin::Ai,
            evidence_strength: EvidenceStrength::Recorded,
            confidence: 0.9,
            state: AttributionState::Exact,
            session_id: "s1".to_string(),
            event_ids: vec!["evt1".to_string()],
            agent_id: "claude-code".to_string(),
            model_id: Some("anthropic:claude-opus".to_string()),
            prompt_hash: Some("sha256:deadbeef".to_string()),
            context_set_id: None,
            policy_tags: vec!["auth".to_string()],
            risk_tags: vec![],
            risk_level: None,
            tests_run: vec![],
            tests_passed: false,
            reviewer: None,
            reviewed_at: None,
        };
        index.index_attribution(&range, "src/a.rs", "blob1", "2026-05-31T00:00:00Z").unwrap();
        let got = index.get_file_attributions("src/a.rs").unwrap();
        assert_eq!(got.len(), 1);
        // The bug this guards: prompt_hash/event_ids were dropped on read.
        assert_eq!(got[0].1.prompt_hash.as_deref(), Some("sha256:deadbeef"));
        assert_eq!(got[0].1.event_ids, vec!["evt1".to_string()]);
    }
}
