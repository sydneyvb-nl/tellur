//! SQLite implementation of [`Store`] — the default single-node backend.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use tellur_core::schema::ids;

use super::{AuditEntry, Org, Store};
use crate::auth::{self, GeneratedToken, Principal, Role};

/// Current schema version. Bumped as migrations are added in later phases.
const SCHEMA_VERSION: &str = "2";

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
