//! SQLite implementation of [`Store`] — the default single-node backend.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use tellur_core::schema::ids;

use super::{AuditEntry, Org, Store};
use crate::auth::{self, GeneratedToken, Principal, Role};

/// Current schema version. Bumped as migrations are added in later phases.
const SCHEMA_VERSION: &str = "1";

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
        let conn = self.conn()?;
        let row = conn
            .query_row(
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
            .context("token lookup failed")?;

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

    fn append_audit(&self, entry: &AuditEntry) -> Result<()> {
        let ts = chrono::Utc::now().to_rfc3339();
        let conn = self.conn()?;
        let prev: String = conn
            .query_row(
                "SELECT entry_hash FROM audit_log ORDER BY seq DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_default();
        let entry_hash = audit_hash(
            &prev,
            &ts,
            entry.org_id.as_deref(),
            entry.actor_member_id.as_deref(),
            &entry.action,
            &entry.detail,
        );
        conn.execute(
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
        }
        Ok(true)
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
