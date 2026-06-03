//! SQLite implementation of [`Store`] — the default single-node backend.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use tellur_core::schema::ids;

use super::{AuditEntry, IngestEvent, Org, Repo, Store};
use crate::auth::{self, GeneratedToken, Principal, Role};

/// Current schema version. Bumped as migrations are added in later phases.
const SCHEMA_VERSION: &str = "4";

/// A SQLite-backed store. The connection is behind a `Mutex` so the store is
/// `Send + Sync` and usable as `Arc<dyn Store>`.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open (or create) a database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open database at {}", path.display()))?;
        Self::init(conn)
    }

    /// Open an ephemeral in-memory database (tests).
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().context("failed to open in-memory database")?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database connection lock poisoned"))
    }
}

/// Compute an audit entry hash over the previous hash and the entry fields.
fn audit_hash(
    prev: &str,
    ts: &str,
    org_id: Option<&str>,
    actor: Option<&str>,
    action: &str,
    detail: &str,
) -> String {
    let material = format!(
        "{prev}|{ts}|{}|{}|{action}|{detail}",
        org_id.unwrap_or(""),
        actor.unwrap_or("")
    );
    ids::hash_content(&material)
}

impl Store for SqliteStore {
    fn migrate(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_meta (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS org (
                 id         TEXT PRIMARY KEY,
                 name       TEXT NOT NULL,
                 created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS member (
                 id           TEXT PRIMARY KEY,
                 org_id       TEXT NOT NULL REFERENCES org(id),
                 display_name TEXT NOT NULL,
                 role         TEXT NOT NULL,
                 created_at   TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_member_org ON member(org_id);
             CREATE TABLE IF NOT EXISTS api_token (
                 token_id    TEXT PRIMARY KEY,
                 member_id   TEXT NOT NULL REFERENCES member(id),
                 secret_hash TEXT NOT NULL,
                 created_at  TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS audit_log (
                 seq             INTEGER PRIMARY KEY AUTOINCREMENT,
                 ts              TEXT NOT NULL,
                 org_id          TEXT,
                 actor_member_id TEXT,
                 action          TEXT NOT NULL,
                 detail          TEXT NOT NULL,
                 prev_hash       TEXT NOT NULL,
                 entry_hash      TEXT NOT NULL
             );
             -- Single-row checkpoint of the audit chain head + length, so tail
             -- truncation / rollback to an earlier prefix is detectable.
             CREATE TABLE IF NOT EXISTS audit_head (
                 id          INTEGER PRIMARY KEY CHECK (id = 1),
                 head_hash   TEXT NOT NULL,
                 entry_count INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS repo (
                 id         TEXT PRIMARY KEY,
                 org_id     TEXT NOT NULL REFERENCES org(id),
                 name       TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 UNIQUE (org_id, name)
             );
             CREATE TABLE IF NOT EXISTS event (
                 seq        INTEGER PRIMARY KEY AUTOINCREMENT,
                 id         TEXT NOT NULL UNIQUE,
                 org_id     TEXT NOT NULL,
                 repo_id    TEXT NOT NULL REFERENCES repo(id),
                 session_id TEXT NOT NULL,
                 ts         TEXT NOT NULL,
                 event_type TEXT NOT NULL,
                 actor      TEXT NOT NULL,
                 payload    TEXT NOT NULL,
                 prev_hash  TEXT NOT NULL,
                 entry_hash TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_event_repo ON event(repo_id, seq);
             -- Per-repo chain head + length checkpoint, so tail truncation /
             -- rollback of a repo's event log is detectable.
             CREATE TABLE IF NOT EXISTS event_head (
                 repo_id     TEXT PRIMARY KEY REFERENCES repo(id),
                 head_hash   TEXT NOT NULL,
                 entry_count INTEGER NOT NULL
             );",
        )
        .context("failed to create schema")?;
        conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [SCHEMA_VERSION],
        )?;
        Ok(())
    }

    fn health_check(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
            .context("database health check failed")?;
        Ok(())
    }

    fn create_org(&self, name: &str) -> Result<Org> {
        let org = Org {
            id: ids::generate_id("org"),
            name: name.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO org (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![org.id, org.name, org.created_at],
        )
        .context("failed to create org")?;
        Ok(org)
    }

    fn create_member(&self, org_id: &str, display_name: &str, role: Role) -> Result<String> {
        let member_id = ids::generate_id("mbr");
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO member (id, org_id, display_name, role, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                member_id,
                org_id,
                display_name,
                role.as_str(),
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .context("failed to create member (does the org exist?)")?;
        Ok(member_id)
    }

    fn create_token(&self, member_id: &str) -> Result<GeneratedToken> {
        let token = auth::generate_token()?;
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO api_token (token_id, member_id, secret_hash, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                token.token_id,
                member_id,
                token.secret_hash,
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .context("failed to create token (does the member exist?)")?;
        Ok(token)
    }

    fn authenticate(&self, token: &str) -> Result<Option<Principal>> {
        let Some((token_id, secret)) = auth::parse_token(token) else {
            return Ok(None);
        };

        // Look up the row, then release the DB lock *before* the (intentionally
        // expensive) Argon2 verification so it does not serialize other store
        // work (audits, readiness, other users' auth).
        let row = {
            let conn = self.conn()?;
            conn.query_row(
                "SELECT t.secret_hash, m.id, m.org_id, m.role
                 FROM api_token t JOIN member m ON m.id = t.member_id
                 WHERE t.token_id = ?1",
                [token_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .context("token lookup failed")?
        };

        let Some((secret_hash, member_id, org_id, role_str)) = row else {
            return Ok(None);
        };
        if !auth::verify_secret(&secret, &secret_hash) {
            return Ok(None);
        }
        Ok(Some(Principal {
            org_id,
            member_id,
            role: Role::parse(&role_str)?,
        }))
    }

    fn ensure_repo(&self, org_id: &str, name: &str) -> Result<Repo> {
        let conn = self.conn()?;
        let id = ids::generate_id("repo");
        // Race-safe get-or-create: insert if absent, then read the canonical id.
        conn.execute(
            "INSERT INTO repo (id, org_id, name, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(org_id, name) DO NOTHING",
            params![id, org_id, name, chrono::Utc::now().to_rfc3339()],
        )
        .context("failed to create repo")?;
        let real_id: String = conn.query_row(
            "SELECT id FROM repo WHERE org_id = ?1 AND name = ?2",
            params![org_id, name],
            |r| r.get(0),
        )?;
        Ok(Repo {
            id: real_id,
            org_id: org_id.to_string(),
            name: name.to_string(),
        })
    }

    fn append_events(
        &self,
        org_id: &str,
        repo_id: &str,
        events: &[IngestEvent],
    ) -> Result<Vec<String>> {
        let mut guard = self.conn()?;
        let tx = guard
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to begin ingest transaction")?;

        // Tenant scoping: the repo must belong to the caller's org.
        let belongs = tx
            .query_row(
                "SELECT 1 FROM repo WHERE id = ?1 AND org_id = ?2",
                params![repo_id, org_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !belongs {
            bail!("repo {repo_id} not found in org {org_id}");
        }

        // The head checkpoint is the authoritative chain tip + length.
        let (mut prev, mut count): (String, i64) = tx
            .query_row(
                "SELECT head_hash, entry_count FROM event_head WHERE repo_id = ?1",
                [repo_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .unwrap_or_else(|| (String::new(), 0));

        let mut new_ids = Vec::with_capacity(events.len());
        for ev in events {
            let id = ids::generate_event_id();
            let prev_opt = if prev.is_empty() {
                None
            } else {
                Some(prev.as_str())
            };
            // Server recomputes the chain hash — client hashes are never trusted.
            let entry_hash = ids::hash_event(
                &id,
                &ev.session_id,
                &ev.timestamp,
                &ev.event_type,
                &ev.actor,
                &ev.payload,
                prev_opt,
            );
            let payload_str = serde_json::to_string(&ev.payload)?;
            tx.execute(
                "INSERT INTO event
                     (id, org_id, repo_id, session_id, ts, event_type, actor, payload,
                      prev_hash, entry_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    org_id,
                    repo_id,
                    ev.session_id,
                    ev.timestamp,
                    ev.event_type,
                    ev.actor,
                    payload_str,
                    prev,
                    entry_hash
                ],
            )
            .context("failed to insert event")?;
            prev = entry_hash;
            count += 1;
            new_ids.push(id);
        }
        tx.execute(
            "INSERT INTO event_head (repo_id, head_hash, entry_count) VALUES (?1, ?2, ?3)
             ON CONFLICT(repo_id) DO UPDATE SET head_hash = excluded.head_hash,
                                                entry_count = excluded.entry_count",
            params![repo_id, prev, count],
        )
        .context("failed to update event head")?;
        tx.commit().context("failed to commit events")?;
        Ok(new_ids)
    }

    fn event_count(&self, org_id: &str, repo_id: &str) -> Result<u64> {
        let conn = self.conn()?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM event WHERE org_id = ?1 AND repo_id = ?2",
            params![org_id, repo_id],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    fn verify_event_chain(&self, org_id: &str, repo_id: &str) -> Result<bool> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, session_id, ts, event_type, actor, payload, prev_hash, entry_hash
             FROM event WHERE org_id = ?1 AND repo_id = ?2 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![org_id, repo_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;

        let mut expected_prev = String::new();
        let mut counted: i64 = 0;
        for row in rows {
            let (id, session_id, ts, event_type, actor, payload, prev_hash, entry_hash) = row?;
            if prev_hash != expected_prev {
                return Ok(false);
            }
            let payload_value: serde_json::Value = serde_json::from_str(&payload)?;
            let prev_opt = if prev_hash.is_empty() {
                None
            } else {
                Some(prev_hash.as_str())
            };
            let recomputed = ids::hash_event(
                &id,
                &session_id,
                &ts,
                &event_type,
                &actor,
                &payload_value,
                prev_opt,
            );
            if recomputed != entry_hash {
                return Ok(false);
            }
            expected_prev = entry_hash;
            counted += 1;
        }

        // Compare against the persisted head checkpoint so tail truncation /
        // rollback to an earlier prefix is detected.
        let head: Option<(String, i64)> = conn
            .query_row(
                "SELECT head_hash, entry_count FROM event_head WHERE repo_id = ?1",
                [repo_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match head {
            Some((head_hash, entry_count)) => {
                Ok(counted == entry_count && expected_prev == head_hash)
            }
            None => Ok(counted == 0),
        }
    }

    fn append_audit(&self, entry: &AuditEntry) -> Result<()> {
        let ts = chrono::Utc::now().to_rfc3339();
        let mut guard = self.conn()?;
        // IMMEDIATE acquires the write lock up front, so the read of the current
        // head and the insert are atomic even across separate connections to the
        // same database (e.g. the server and the admin CLI). Without this, two
        // writers could read the same head and create siblings with identical
        // `prev_hash`, which would later look like tampering.
        let tx = guard
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to begin audit transaction")?;

        let (prev, count): (String, i64) = tx
            .query_row(
                "SELECT head_hash, entry_count FROM audit_head WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .unwrap_or_else(|| (String::new(), 0));

        let entry_hash = audit_hash(
            &prev,
            &ts,
            entry.org_id.as_deref(),
            entry.actor_member_id.as_deref(),
            &entry.action,
            &entry.detail,
        );
        tx.execute(
            "INSERT INTO audit_log
                 (ts, org_id, actor_member_id, action, detail, prev_hash, entry_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                ts,
                entry.org_id,
                entry.actor_member_id,
                entry.action,
                entry.detail,
                prev,
                entry_hash
            ],
        )
        .context("failed to append audit entry")?;
        tx.execute(
            "INSERT INTO audit_head (id, head_hash, entry_count) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET head_hash = excluded.head_hash,
                                           entry_count = excluded.entry_count",
            params![entry_hash, count + 1],
        )
        .context("failed to update audit head")?;
        tx.commit().context("failed to commit audit entry")?;
        Ok(())
    }

    fn audit_len(&self) -> Result<u64> {
        let conn = self.conn()?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    fn verify_audit_chain(&self) -> Result<bool> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT ts, org_id, actor_member_id, action, detail, prev_hash, entry_hash
             FROM audit_log ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,         // ts
                r.get::<_, Option<String>>(1)?, // org_id
                r.get::<_, Option<String>>(2)?, // actor
                r.get::<_, String>(3)?,         // action
                r.get::<_, String>(4)?,         // detail
                r.get::<_, String>(5)?,         // prev_hash
                r.get::<_, String>(6)?,         // entry_hash
            ))
        })?;

        let mut expected_prev = String::new();
        let mut counted: i64 = 0;
        for row in rows {
            let (ts, org_id, actor, action, detail, prev_hash, entry_hash) = row?;
            if prev_hash != expected_prev {
                return Ok(false);
            }
            let recomputed = audit_hash(
                &prev_hash,
                &ts,
                org_id.as_deref(),
                actor.as_deref(),
                &action,
                &detail,
            );
            if recomputed != entry_hash {
                return Ok(false);
            }
            expected_prev = entry_hash;
            counted += 1;
        }

        // Compare against the persisted head checkpoint so deleting the newest
        // row(s) — a tail truncation that leaves an internally-consistent prefix
        // — is detected.
        let head: Option<(String, i64)> = conn
            .query_row(
                "SELECT head_hash, entry_count FROM audit_head WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match head {
            Some((head_hash, entry_count)) => {
                Ok(counted == entry_count && expected_prev == head_hash)
            }
            None => Ok(counted == 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> SqliteStore {
        let s = SqliteStore::open_in_memory().unwrap();
        s.migrate().unwrap();
        s
    }

    #[test]
    fn migrate_is_idempotent_and_records_version() {
        let s = store();
        s.migrate().unwrap();
        let conn = s.conn().unwrap();
        let version: String = conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn token_authenticates_to_its_member_and_org() {
        let s = store();
        let org = s.create_org("Acme").unwrap();
        let member = s.create_member(&org.id, "alice", Role::Admin).unwrap();
        let token = s.create_token(&member).unwrap();

        let principal = s.authenticate(&token.plaintext).unwrap().unwrap();
        assert_eq!(principal.org_id, org.id);
        assert_eq!(principal.member_id, member);
        assert_eq!(principal.role, Role::Admin);
    }

    #[test]
    fn invalid_and_tampered_tokens_are_rejected() {
        let s = store();
        let org = s.create_org("Acme").unwrap();
        let member = s.create_member(&org.id, "bob", Role::Viewer).unwrap();
        let token = s.create_token(&member).unwrap();

        assert!(s.authenticate("garbage").unwrap().is_none());
        assert!(s.authenticate("tlr_deadbeef_nope").unwrap().is_none());
        // Right id, wrong secret.
        let (id, _) = auth::parse_token(&token.plaintext).unwrap();
        let forged = format!("tlr_{id}_0000");
        assert!(s.authenticate(&forged).unwrap().is_none());
    }

    fn ingest_event(detail: &str) -> IngestEvent {
        IngestEvent {
            session_id: "sess_1".to_string(),
            timestamp: "2026-06-03T00:00:00Z".to_string(),
            event_type: "file.write".to_string(),
            actor: "agent".to_string(),
            payload: serde_json::json!({ "file_path": detail }),
        }
    }

    #[test]
    fn ensure_repo_is_idempotent() {
        let s = store();
        let org = s.create_org("Acme").unwrap();
        let r1 = s.ensure_repo(&org.id, "app").unwrap();
        let r2 = s.ensure_repo(&org.id, "app").unwrap();
        assert_eq!(r1.id, r2.id);
    }

    #[test]
    fn events_append_with_verifiable_chain_and_tenant_scope() {
        let s = store();
        let org = s.create_org("Acme").unwrap();
        let repo = s.ensure_repo(&org.id, "app").unwrap();
        let ids = s
            .append_events(
                &org.id,
                &repo.id,
                &[ingest_event("a.rs"), ingest_event("b.rs")],
            )
            .unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(s.event_count(&org.id, &repo.id).unwrap(), 2);
        assert!(s.verify_event_chain(&org.id, &repo.id).unwrap());
        // Another org sees nothing for this repo id (data-layer scoping).
        assert_eq!(s.event_count("org_other", &repo.id).unwrap(), 0);
    }

    #[test]
    fn append_events_rejects_repo_outside_org() {
        let s = store();
        let org = s.create_org("Acme").unwrap();
        let repo = s.ensure_repo(&org.id, "app").unwrap();
        // Wrong org for this repo id → rejected.
        let err = s.append_events("org_other", &repo.id, &[ingest_event("x")]);
        assert!(err.is_err());
    }

    #[test]
    fn event_chain_detects_tail_truncation() {
        let s = store();
        let org = s.create_org("Acme").unwrap();
        let repo = s.ensure_repo(&org.id, "app").unwrap();
        s.append_events(
            &org.id,
            &repo.id,
            &[ingest_event("a"), ingest_event("b"), ingest_event("c")],
        )
        .unwrap();
        assert!(s.verify_event_chain(&org.id, &repo.id).unwrap());
        // Delete the newest event but leave the head checkpoint intact.
        {
            let conn = s.conn().unwrap();
            conn.execute(
                "DELETE FROM event WHERE seq = (SELECT MAX(seq) FROM event WHERE repo_id = ?1)",
                [&repo.id],
            )
            .unwrap();
        }
        assert!(
            !s.verify_event_chain(&org.id, &repo.id).unwrap(),
            "tail truncation of events must be detected"
        );
    }

    #[test]
    fn event_chain_detects_tampering() {
        let s = store();
        let org = s.create_org("Acme").unwrap();
        let repo = s.ensure_repo(&org.id, "app").unwrap();
        s.append_events(&org.id, &repo.id, &[ingest_event("a.rs")])
            .unwrap();
        {
            let conn = s.conn().unwrap();
            conn.execute("UPDATE event SET payload = '{\"file_path\":\"evil\"}'", [])
                .unwrap();
        }
        assert!(!s.verify_event_chain(&org.id, &repo.id).unwrap());
    }

    #[test]
    fn audit_chain_appends_and_verifies() {
        let s = store();
        for i in 0..3 {
            s.append_audit(&AuditEntry {
                org_id: Some("org_1".to_string()),
                actor_member_id: Some("mbr_1".to_string()),
                action: "test".to_string(),
                detail: format!("entry {i}"),
            })
            .unwrap();
        }
        assert_eq!(s.audit_len().unwrap(), 3);
        assert!(s.verify_audit_chain().unwrap());
    }

    #[test]
    fn audit_chain_detects_tail_truncation() {
        let s = store();
        for i in 0..3 {
            s.append_audit(&AuditEntry {
                org_id: None,
                actor_member_id: None,
                action: "a".to_string(),
                detail: format!("{i}"),
            })
            .unwrap();
        }
        assert!(s.verify_audit_chain().unwrap());
        // Delete the newest row but leave the head checkpoint intact.
        {
            let conn = s.conn().unwrap();
            conn.execute(
                "DELETE FROM audit_log WHERE seq = (SELECT MAX(seq) FROM audit_log)",
                [],
            )
            .unwrap();
        }
        assert!(
            !s.verify_audit_chain().unwrap(),
            "tail truncation must be detected"
        );
    }

    #[test]
    fn audit_chain_across_two_connections_stays_valid() {
        // Two stores on the same file DB (e.g. server + admin CLI). IMMEDIATE
        // transactions serialize the appends so the chain stays consistent.
        let dir = std::env::temp_dir().join(format!(
            "tellur-audit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hub.db");

        let a = SqliteStore::open(&path).unwrap();
        a.migrate().unwrap();
        let b = SqliteStore::open(&path).unwrap();

        for i in 0..4 {
            let s = if i % 2 == 0 { &a } else { &b };
            s.append_audit(&AuditEntry {
                org_id: None,
                actor_member_id: None,
                action: "x".to_string(),
                detail: format!("{i}"),
            })
            .unwrap();
        }
        assert_eq!(a.audit_len().unwrap(), 4);
        assert!(a.verify_audit_chain().unwrap());
        assert!(b.verify_audit_chain().unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn audit_chain_detects_tampering() {
        let s = store();
        s.append_audit(&AuditEntry {
            org_id: None,
            actor_member_id: None,
            action: "a".to_string(),
            detail: "original".to_string(),
        })
        .unwrap();
        // Tamper with the stored detail without recomputing the hash.
        {
            let conn = s.conn().unwrap();
            conn.execute("UPDATE audit_log SET detail = 'tampered' WHERE seq = 1", [])
                .unwrap();
        }
        assert!(!s.verify_audit_chain().unwrap());
    }
}
