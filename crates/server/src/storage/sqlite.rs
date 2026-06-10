//! SQLite implementation of [`Store`] — the default single-node backend.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use tellur_core::schema::ids;
use tellur_core::schema::types::FileAttribution;

use super::chain;
use super::{
    ActivityBucket, ActivityGroup, AuditEntry, AuditRecord, ComplianceSnapshot, IngestEvent, Job,
    LoginTx, MemberInfo, Org, OrgReport, PolicyDoc, PolicySummary, Repo, RepoFacts, RepoRoleGrant,
    RepoSource, RepoSummary, ScimGroup, ScimUser, SessionSummary, Store, StoredEvent,
    role_from_group_name,
};
use crate::auth::{self, GeneratedToken, Principal, Role};

/// Current schema version. Bumped as migrations are added in later phases.
const SCHEMA_VERSION: &str = "17";

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

/// Split a `group_concat` CSV (or `None`) into a de-duped, sorted, non-empty
/// list — used for per-session distinct actors/repos.
fn split_csv(s: Option<String>) -> Vec<String> {
    let mut v: Vec<String> = s
        .unwrap_or_default()
        .split(',')
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Add a column to a table if it is not already present (idempotent migration).
/// Table/column/definition are always internal constants, never user input.
fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
    debug_assert!(matches!(
        table,
        "member" | "member_identity" | "oidc_login" | "job" | "audit_head" | "repo_source"
    ));
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let present = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(std::result::Result::ok)
        .any(|name| name == column);
    if !present {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}

/// Read the audit chain's sealed checkpoint `(sealed_hash, sealed_count)` — the
/// tip hash and length of a pruned prefix. Genesis default `("", 0)`.
fn audit_checkpoint(conn: &Connection) -> Result<(String, i64)> {
    let row = conn
        .query_row(
            "SELECT sealed_hash, sealed_count FROM audit_head WHERE id = 1",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?;
    Ok(row.unwrap_or_else(|| (String::new(), 0)))
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

/// Count events grouped by an internal column (`event_type` or `actor`), scoped
/// to an org. The column is never user-controlled.
fn group_counts(conn: &Connection, column: &str, org_id: &str) -> Result<BTreeMap<String, u64>> {
    assert!(
        matches!(column, "event_type" | "actor"),
        "group_counts column must be an allow-listed identifier"
    );
    let sql = format!("SELECT {column}, COUNT(*) FROM event WHERE org_id = ?1 GROUP BY {column}");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([org_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })?;
    let mut map = BTreeMap::new();
    for row in rows {
        let (key, count) = row?;
        map.insert(key, count as u64);
    }
    Ok(map)
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
                 created_at   TEXT NOT NULL,
                 -- Deactivated members cannot authenticate (SCIM deprovisioning).
                 active       INTEGER NOT NULL DEFAULT 1
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
                 id           INTEGER PRIMARY KEY CHECK (id = 1),
                 head_hash    TEXT NOT NULL,
                 entry_count  INTEGER NOT NULL,
                 sealed_hash  TEXT NOT NULL DEFAULT '',
                 sealed_count INTEGER NOT NULL DEFAULT 0
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
             -- Speeds up org-scoped aggregates in org_report.
             CREATE INDEX IF NOT EXISTS idx_event_org ON event(org_id);
             -- Per-repo chain head + length checkpoint, so tail truncation /
             -- rollback of a repo's event log is detectable.
             CREATE TABLE IF NOT EXISTS event_head (
                 repo_id     TEXT PRIMARY KEY REFERENCES repo(id),
                 head_hash   TEXT NOT NULL,
                 entry_count INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS policy (
                 org_id     TEXT NOT NULL REFERENCES org(id),
                 name       TEXT NOT NULL,
                 content    TEXT NOT NULL,
                 version    INTEGER NOT NULL,
                 updated_at TEXT NOT NULL,
                 PRIMARY KEY (org_id, name)
             );
             CREATE TABLE IF NOT EXISTS attribution (
                 org_id       TEXT NOT NULL,
                 repo_id      TEXT NOT NULL REFERENCES repo(id),
                 file_path    TEXT NOT NULL,
                 git_blob_sha TEXT NOT NULL,
                 ranges_json  TEXT NOT NULL,
                 updated_at   TEXT NOT NULL,
                 PRIMARY KEY (org_id, repo_id, file_path)
             );
             -- Fine-grained per-repo role grants. Additive over the member's org
             -- role: effective role on a repo is max(org_role, repo grant).
             -- Opt-in source link template per repo (A12). Stores only a URL
             -- template — never source code. {path}/{start}/{end} are
             -- substituted client-side to deep-link the provider.
             CREATE TABLE IF NOT EXISTS repo_source (
                 repo_id      TEXT PRIMARY KEY REFERENCES repo(id),
                 org_id       TEXT NOT NULL REFERENCES org(id),
                 template     TEXT,
                 raw_template TEXT,
                 updated_at   TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS repo_role (
                 org_id     TEXT NOT NULL REFERENCES org(id),
                 repo_id    TEXT NOT NULL REFERENCES repo(id),
                 member_id  TEXT NOT NULL REFERENCES member(id),
                 role       TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 PRIMARY KEY (repo_id, member_id)
             );
             CREATE INDEX IF NOT EXISTS idx_repo_role_repo ON repo_role(repo_id);
             -- SSO identity: maps a member to a (globally unique) email and an
             -- optional OIDC subject bound on first login. Separate from member
             -- so the core identity table is untouched.
             CREATE TABLE IF NOT EXISTS member_identity (
                 member_id    TEXT PRIMARY KEY REFERENCES member(id),
                 email        TEXT UNIQUE,
                 oidc_issuer  TEXT,
                 oidc_subject TEXT,
                 external_id  TEXT,
                 -- A subject is only unique within an issuer.
                 UNIQUE (oidc_issuer, oidc_subject)
             );
             -- Org-scoped SCIM provisioning tokens (secret stored hashed).
             CREATE TABLE IF NOT EXISTS scim_token (
                 token_id    TEXT PRIMARY KEY,
                 org_id      TEXT NOT NULL REFERENCES org(id),
                 secret_hash TEXT NOT NULL,
                 created_at  TEXT NOT NULL
             );
             -- Pending OIDC login transactions (CSRF state -> PKCE/nonce +
             -- a browser-binding secret matched against a login cookie).
             CREATE TABLE IF NOT EXISTS oidc_login (
                 state           TEXT PRIMARY KEY,
                 pkce_verifier   TEXT NOT NULL,
                 nonce           TEXT NOT NULL,
                 browser_binding TEXT NOT NULL,
                 created_at      TEXT NOT NULL
             );
             -- Browser sessions (opaque id -> member, with expiry).
             CREATE TABLE IF NOT EXISTS session (
                 id         TEXT PRIMARY KEY,
                 member_id  TEXT NOT NULL REFERENCES member(id),
                 created_at TEXT NOT NULL,
                 expires_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_session_member ON session(member_id);
             -- Durable background jobs (e.g. large org exports).
             CREATE TABLE IF NOT EXISTS job (
                 id         TEXT PRIMARY KEY,
                 org_id     TEXT NOT NULL REFERENCES org(id),
                 kind       TEXT NOT NULL,
                 status     TEXT NOT NULL,
                 result     TEXT,
                 error      TEXT,
                 params     TEXT,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_job_status ON job(status, created_at);
             -- SCIM groups + membership (group displayName drives org roles).
             CREATE TABLE IF NOT EXISTS scim_group (
                 id           TEXT PRIMARY KEY,
                 org_id       TEXT NOT NULL REFERENCES org(id),
                 display_name TEXT NOT NULL,
                 external_id  TEXT,
                 created_at   TEXT NOT NULL,
                 updated_at   TEXT NOT NULL,
                 UNIQUE (org_id, display_name)
             );
             CREATE TABLE IF NOT EXISTS scim_group_member (
                 group_id  TEXT NOT NULL REFERENCES scim_group(id),
                 member_id TEXT NOT NULL REFERENCES member(id),
                 PRIMARY KEY (group_id, member_id)
             );
             CREATE INDEX IF NOT EXISTS idx_group_member ON scim_group_member(member_id);
             -- Policy-compliance snapshots (A8): append-only, timestamped per
             -- (org, repo, policy version); the dashboard reads the latest.
             CREATE TABLE IF NOT EXISTS compliance_snapshot (
                 id             TEXT PRIMARY KEY,
                 org_id         TEXT NOT NULL REFERENCES org(id),
                 repo_id        TEXT NOT NULL,
                 repo_name      TEXT NOT NULL,
                 policy_name    TEXT NOT NULL,
                 policy_version INTEGER NOT NULL,
                 evaluated_at   TEXT NOT NULL,
                 ai_ranges      INTEGER NOT NULL,
                 violations     INTEGER NOT NULL,
                 high           INTEGER NOT NULL,
                 medium         INTEGER NOT NULL,
                 low            INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_compliance_latest
                 ON compliance_snapshot(org_id, repo_id, evaluated_at);",
        )
        .context("failed to create schema")?;

        // Additive migrations for columns introduced after a table's first
        // version. `CREATE TABLE IF NOT EXISTS` is a no-op on an already-created
        // table, so columns added later must be applied with guarded ALTERs or
        // an upgraded database would be missing them (e.g. `member.active`, which
        // every auth lookup now queries).
        ensure_column(&conn, "member", "active", "INTEGER NOT NULL DEFAULT 1")?;
        ensure_column(&conn, "job", "params", "TEXT")?;
        ensure_column(&conn, "repo_source", "raw_template", "TEXT")?;
        // v16 created repo_source.template as NOT NULL; A12's raw-only configs
        // store a NULL there, so drop the constraint by rebuilding the table
        // (SQLite can't ALTER it away). Guarded so it runs only on old DBs.
        let template_notnull: i64 = conn
            .query_row(
                "SELECT \"notnull\" FROM pragma_table_info('repo_source') WHERE name = 'template'",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if template_notnull == 1 {
            conn.execute_batch(
                "CREATE TABLE repo_source_new (
                     repo_id      TEXT PRIMARY KEY REFERENCES repo(id),
                     org_id       TEXT NOT NULL REFERENCES org(id),
                     template     TEXT,
                     raw_template TEXT,
                     updated_at   TEXT NOT NULL
                 );
                 INSERT INTO repo_source_new (repo_id, org_id, template, raw_template, updated_at)
                     SELECT repo_id, org_id, template, raw_template, updated_at FROM repo_source;
                 DROP TABLE repo_source;
                 ALTER TABLE repo_source_new RENAME TO repo_source;",
            )?;
        }
        ensure_column(
            &conn,
            "audit_head",
            "sealed_hash",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        ensure_column(
            &conn,
            "audit_head",
            "sealed_count",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column(&conn, "member_identity", "oidc_issuer", "TEXT")?;
        ensure_column(&conn, "member_identity", "external_id", "TEXT")?;
        ensure_column(
            &conn,
            "oidc_login",
            "browser_binding",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        // Composite uniqueness for (issuer, subject) also applies to upgraded
        // DBs (inline UNIQUE in CREATE only covers fresh installs).
        conn.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_member_identity_oidc
                 ON member_identity(oidc_issuer, oidc_subject);",
        )?;

        conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [SCHEMA_VERSION],
        )?;
        Ok(())
    }

    fn health_check(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
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
                 WHERE t.token_id = ?1 AND m.active = 1",
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

    fn find_repo(&self, org_id: &str, repo: &str) -> Result<Option<Repo>> {
        let conn = self.conn()?;
        let found: Option<(String, String)> = conn
            .query_row(
                "SELECT id, name FROM repo WHERE org_id = ?1 AND id = ?2",
                params![org_id, repo],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let found = match found {
            Some(repo) => Some(repo),
            None => conn
                .query_row(
                    "SELECT id, name FROM repo WHERE org_id = ?1 AND name = ?2",
                    params![org_id, repo],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?,
        };
        Ok(found.map(|(id, name)| Repo {
            id,
            org_id: org_id.to_string(),
            name,
        }))
    }

    fn get_repo_source(&self, org_id: &str, repo_id: &str) -> Result<RepoSource> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT template, raw_template FROM repo_source
                 WHERE org_id = ?1 AND repo_id = ?2",
                params![org_id, repo_id],
                |r| {
                    Ok(RepoSource {
                        link: r.get::<_, Option<String>>(0)?,
                        raw: r.get::<_, Option<String>>(1)?,
                    })
                },
            )
            .optional()?;
        Ok(row.unwrap_or_default())
    }

    fn set_repo_source(
        &self,
        org_id: &str,
        repo_id: &str,
        link: Option<&str>,
        raw: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn()?;
        if link.is_none() && raw.is_none() {
            conn.execute(
                "DELETE FROM repo_source WHERE org_id = ?1 AND repo_id = ?2",
                params![org_id, repo_id],
            )?;
        } else {
            let now = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO repo_source (repo_id, org_id, template, raw_template, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(repo_id) DO UPDATE SET template = excluded.template,
                                                    raw_template = excluded.raw_template,
                                                    updated_at = excluded.updated_at",
                params![repo_id, org_id, link, raw, now],
            )?;
        }
        Ok(())
    }

    fn set_repo_role(
        &self,
        org_id: &str,
        repo_id: &str,
        member_id: &str,
        role: Role,
    ) -> Result<()> {
        let conn = self.conn()?;
        // Both the repo and the member must belong to the org (no cross-tenant
        // grants).
        let repo_ok: bool = conn
            .query_row(
                "SELECT 1 FROM repo WHERE id = ?1 AND org_id = ?2",
                params![repo_id, org_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !repo_ok {
            bail!("repo {repo_id} not found in org {org_id}");
        }
        let member_ok: bool = conn
            .query_row(
                "SELECT 1 FROM member WHERE id = ?1 AND org_id = ?2",
                params![member_id, org_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !member_ok {
            bail!("member {member_id} not found in org {org_id}");
        }
        conn.execute(
            "INSERT INTO repo_role (org_id, repo_id, member_id, role, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(repo_id, member_id) DO UPDATE SET role = excluded.role,
                                                           updated_at = excluded.updated_at",
            params![
                org_id,
                repo_id,
                member_id,
                role.as_str(),
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    fn remove_repo_role(&self, org_id: &str, repo_id: &str, member_id: &str) -> Result<bool> {
        let conn = self.conn()?;
        let n = conn.execute(
            "DELETE FROM repo_role WHERE org_id = ?1 AND repo_id = ?2 AND member_id = ?3",
            params![org_id, repo_id, member_id],
        )?;
        Ok(n > 0)
    }

    fn get_repo_role(&self, org_id: &str, repo_id: &str, member_id: &str) -> Result<Option<Role>> {
        let conn = self.conn()?;
        let role: Option<String> = conn
            .query_row(
                "SELECT role FROM repo_role WHERE org_id = ?1 AND repo_id = ?2 AND member_id = ?3",
                params![org_id, repo_id, member_id],
                |r| r.get(0),
            )
            .optional()?;
        role.map(|r| Role::parse(&r)).transpose()
    }

    fn list_repo_roles(&self, org_id: &str, repo_id: &str) -> Result<Vec<RepoRoleGrant>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT member_id, role, updated_at FROM repo_role
             WHERE org_id = ?1 AND repo_id = ?2 ORDER BY member_id",
        )?;
        let rows = stmt.query_map(params![org_id, repo_id], |r| {
            Ok(RepoRoleGrant {
                member_id: r.get(0)?,
                role: r.get(1)?,
                updated_at: r.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
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
        let head = chain::HeadRef {
            table: "event_head",
            key_col: "repo_id",
            key: &repo_id,
        };
        let (mut prev, mut count) = chain::read_head(&tx, &head)?;

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
        chain::write_head(&tx, &head, &prev, count)?;
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
        let head = chain::HeadRef {
            table: "event_head",
            key_col: "repo_id",
            key: &repo_id,
        };
        chain::verify(
            &conn,
            "SELECT id, session_id, ts, event_type, actor, payload, prev_hash, entry_hash
             FROM event WHERE org_id = ?1 AND repo_id = ?2 ORDER BY seq ASC",
            params![org_id, repo_id],
            &head,
            ("", 0),
            |r| {
                let id: String = r.get(0)?;
                let session_id: String = r.get(1)?;
                let ts: String = r.get(2)?;
                let event_type: String = r.get(3)?;
                let actor: String = r.get(4)?;
                let payload: String = r.get(5)?;
                let prev_hash: String = r.get(6)?;
                let entry_hash: String = r.get(7)?;
                let payload_value: serde_json::Value = serde_json::from_str(&payload)
                    .with_context(|| format!("corrupt event payload for event {id}"))?;
                let prev_opt = (!prev_hash.is_empty()).then_some(prev_hash.as_str());
                let recomputed = ids::hash_event(
                    &id,
                    &session_id,
                    &ts,
                    &event_type,
                    &actor,
                    &payload_value,
                    prev_opt,
                );
                Ok((prev_hash, entry_hash, recomputed))
            },
        )
    }

    fn put_attributions(
        &self,
        org_id: &str,
        repo_id: &str,
        files: &[FileAttribution],
    ) -> Result<usize> {
        let mut guard = self.conn()?;
        let tx = guard
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to begin attribution transaction")?;
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
        let now = chrono::Utc::now().to_rfc3339();
        for file in files {
            let ranges_json = serde_json::to_string(&file.ranges)?;
            tx.execute(
                "INSERT INTO attribution
                     (org_id, repo_id, file_path, git_blob_sha, ranges_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(org_id, repo_id, file_path) DO UPDATE SET
                     git_blob_sha = excluded.git_blob_sha,
                     ranges_json  = excluded.ranges_json,
                     updated_at   = excluded.updated_at",
                params![
                    org_id,
                    repo_id,
                    file.file_path,
                    file.git_blob_sha,
                    ranges_json,
                    now
                ],
            )
            .context("failed to upsert attribution")?;
        }
        tx.commit().context("failed to commit attributions")?;
        Ok(files.len())
    }

    fn list_attributions(&self, org_id: &str, repo_id: &str) -> Result<Vec<FileAttribution>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT file_path, git_blob_sha, ranges_json, updated_at
             FROM attribution WHERE org_id = ?1 AND repo_id = ?2 ORDER BY file_path",
        )?;
        let rows = stmt.query_map(params![org_id, repo_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (file_path, git_blob_sha, ranges_json, updated_at) = row?;
            let ranges = serde_json::from_str(&ranges_json)
                .with_context(|| format!("corrupt attribution ranges for {file_path}"))?;
            out.push(FileAttribution {
                schema: "tellur.attribution.v1".to_string(),
                file_path,
                git_blob_sha,
                ranges,
                updated_at,
            });
        }
        Ok(out)
    }

    fn list_repos(&self, org_id: &str) -> Result<Vec<RepoSummary>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT r.id, r.name, COUNT(e.seq)
             FROM repo r LEFT JOIN event e ON e.repo_id = r.id
             WHERE r.org_id = ?1
             GROUP BY r.id, r.name
             ORDER BY r.name",
        )?;
        let rows = stmt.query_map([org_id], |r| {
            Ok(RepoSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                event_count: r.get::<_, i64>(2)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn list_events(
        &self,
        org_id: &str,
        repo_id: &str,
        limit: u32,
        before_seq: Option<i64>,
    ) -> Result<Vec<StoredEvent>> {
        let cursor = before_seq.unwrap_or(i64::MAX);
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT seq, id, session_id, ts, event_type, actor, payload
             FROM event
             WHERE org_id = ?1 AND repo_id = ?2 AND seq < ?3
             ORDER BY seq DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![org_id, repo_id, cursor, limit], |r| {
            let payload_str: String = r.get(6)?;
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                payload_str,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, id, session_id, timestamp, event_type, actor, payload_str) = row?;
            // Surface integrity problems instead of masking them as `null`.
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq,
                id,
                repo_id: repo_id.to_string(),
                session_id,
                timestamp,
                event_type,
                actor,
                payload,
            });
        }
        Ok(out)
    }

    fn org_report(&self, org_id: &str) -> Result<OrgReport> {
        let conn = self.conn()?;
        let total_events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM event WHERE org_id = ?1",
            [org_id],
            |r| r.get(0),
        )?;
        let distinct_sessions: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT session_id) FROM event WHERE org_id = ?1",
            [org_id],
            |r| r.get(0),
        )?;

        let by_type = group_counts(&conn, "event_type", org_id)?;
        let by_actor = group_counts(&conn, "actor", org_id)?;
        drop(conn);

        Ok(OrgReport {
            org_id: org_id.to_string(),
            total_events: total_events as u64,
            distinct_sessions: distinct_sessions as u64,
            by_type,
            by_actor,
            repos: self.list_repos(org_id)?,
        })
    }

    fn recent_org_events(&self, org_id: &str, limit: u32) -> Result<Vec<StoredEvent>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT seq, id, repo_id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = ?1 ORDER BY seq DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![org_id, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, id, repo_id, session_id, timestamp, event_type, actor, payload_str) = row?;
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq,
                id,
                repo_id,
                session_id,
                timestamp,
                event_type,
                actor,
                payload,
            });
        }
        Ok(out)
    }

    fn activity_by_day(
        &self,
        org_id: &str,
        since_rfc3339: &str,
        group: ActivityGroup,
    ) -> Result<Vec<ActivityBucket>> {
        let conn = self.conn()?;
        // The grouping column is an allow-listed constant, never user input.
        let sql = format!(
            "SELECT substr(ts, 1, 10) AS day, {col} AS key, COUNT(*) AS n
             FROM event WHERE org_id = ?1 AND ts >= ?2
             GROUP BY day, key ORDER BY day ASC, key ASC",
            col = group.column()
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![org_id, since_rfc3339], |r| {
            Ok(ActivityBucket {
                day: r.get(0)?,
                key: r.get(1)?,
                count: r.get::<_, i64>(2)? as u64,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn repo_facts(&self, org_id: &str, repo_id: &str) -> Result<RepoFacts> {
        let conn = self.conn()?;
        let event_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM event WHERE org_id = ?1 AND repo_id = ?2",
            params![org_id, repo_id],
            |r| r.get(0),
        )?;
        let last_activity: Option<String> = conn.query_row(
            "SELECT MAX(ts) FROM event WHERE org_id = ?1 AND repo_id = ?2",
            params![org_id, repo_id],
            |r| r.get(0),
        )?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT actor FROM event
             WHERE org_id = ?1 AND repo_id = ?2 ORDER BY actor",
        )?;
        let contributors = stmt
            .query_map(params![org_id, repo_id], |r| r.get::<_, String>(0))?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(RepoFacts {
            event_count: event_count as u64,
            contributors,
            last_activity,
        })
    }

    fn list_sessions(
        &self,
        org_id: &str,
        repo_id: Option<&str>,
        actor: Option<&str>,
        since_rfc3339: Option<&str>,
        limit: u32,
    ) -> Result<Vec<SessionSummary>> {
        let conn = self.conn()?;
        // NOTE: SQLite's `group_concat(DISTINCT x)` only supports the default
        // comma separator, so an actor/repo id literally containing a comma
        // would split wrongly in `split_csv`. Repo ids are generated (no commas)
        // and actors are agent ids; revisit if free-form actor strings arrive.
        let mut stmt = conn.prepare(
            "SELECT session_id, COUNT(*) AS n, MIN(ts) AS f, MAX(ts) AS l,
                    group_concat(DISTINCT actor) AS actors,
                    group_concat(DISTINCT repo_id) AS repos
             FROM event
             WHERE org_id = ?1
               AND (?2 IS NULL OR repo_id = ?2)
               AND (?3 IS NULL OR actor = ?3)
               AND (?4 IS NULL OR ts >= ?4)
             GROUP BY session_id ORDER BY l DESC LIMIT ?5",
        )?;
        let rows = stmt.query_map(params![org_id, repo_id, actor, since_rfc3339, limit], |r| {
            Ok(SessionSummary {
                session_id: r.get(0)?,
                event_count: r.get::<_, i64>(1)? as u64,
                first_ts: r.get(2)?,
                last_ts: r.get(3)?,
                actors: split_csv(r.get::<_, Option<String>>(4)?),
                repos: split_csv(r.get::<_, Option<String>>(5)?),
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    fn session_events(
        &self,
        org_id: &str,
        session_id: &str,
        limit: u32,
    ) -> Result<Vec<StoredEvent>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT seq, id, repo_id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = ?1 AND session_id = ?2 ORDER BY seq ASC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![org_id, session_id, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, id, repo_id, session_id, timestamp, event_type, actor, payload_str) = row?;
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq,
                id,
                repo_id,
                session_id,
                timestamp,
                event_type,
                actor,
                payload,
            });
        }
        Ok(out)
    }

    fn put_policy(&self, org_id: &str, name: &str, content: &str) -> Result<i64> {
        let mut guard = self.conn()?;
        let tx = guard
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to begin policy transaction")?;
        let current: i64 = tx
            .query_row(
                "SELECT version FROM policy WHERE org_id = ?1 AND name = ?2",
                params![org_id, name],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let version = current + 1;
        tx.execute(
            "INSERT INTO policy (org_id, name, content, version, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(org_id, name) DO UPDATE SET content = excluded.content,
                                                     version = excluded.version,
                                                     updated_at = excluded.updated_at",
            params![
                org_id,
                name,
                content,
                version,
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .context("failed to write policy")?;
        tx.commit()?;
        Ok(version)
    }

    fn list_policies(&self, org_id: &str) -> Result<Vec<PolicySummary>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT name, version, updated_at FROM policy WHERE org_id = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map([org_id], |r| {
            Ok(PolicySummary {
                name: r.get(0)?,
                version: r.get(1)?,
                updated_at: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn get_policy(&self, org_id: &str, name: &str) -> Result<Option<PolicyDoc>> {
        let conn = self.conn()?;
        let doc = conn
            .query_row(
                "SELECT name, content, version, updated_at FROM policy
                 WHERE org_id = ?1 AND name = ?2",
                params![org_id, name],
                |r| {
                    Ok(PolicyDoc {
                        name: r.get(0)?,
                        content: r.get(1)?,
                        version: r.get(2)?,
                        updated_at: r.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(doc)
    }

    fn export_events(&self, org_id: &str) -> Result<Vec<StoredEvent>> {
        let conn = self.conn()?;
        // Include repo_id: an org-level export spans multiple repos, so each
        // event must carry which repo it belongs to.
        let mut stmt = conn.prepare(
            "SELECT seq, id, repo_id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map([org_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, id, repo_id, session_id, timestamp, event_type, actor, payload_str) = row?;
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq,
                id,
                repo_id,
                session_id,
                timestamp,
                event_type,
                actor,
                payload,
            });
        }
        Ok(out)
    }

    fn export_audit(&self, org_id: &str) -> Result<Vec<AuditRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT seq, ts, org_id, actor_member_id, action, detail, entry_hash
             FROM audit_log WHERE org_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map([org_id], |r| {
            Ok(AuditRecord {
                seq: r.get(0)?,
                ts: r.get(1)?,
                org_id: r.get(2)?,
                actor_member_id: r.get(3)?,
                action: r.get(4)?,
                detail: r.get(5)?,
                entry_hash: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn list_audit(
        &self,
        org_id: &str,
        actor: Option<&str>,
        action: Option<&str>,
        since_rfc3339: Option<&str>,
        before_seq: Option<i64>,
        limit: u32,
    ) -> Result<Vec<AuditRecord>> {
        let conn = self.conn()?;
        // Build the filter dynamically; every clause is a bound parameter so
        // there is no injection surface. `org_id = ?` keeps it tenant-scoped
        // (rows with a NULL org — e.g. pre-auth denials — are never returned).
        let mut sql = String::from(
            "SELECT seq, ts, org_id, actor_member_id, action, detail, entry_hash
             FROM audit_log WHERE org_id = ?1",
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(org_id.to_string())];
        if let Some(a) = actor {
            params.push(Box::new(a.to_string()));
            sql.push_str(&format!(" AND actor_member_id = ?{}", params.len()));
        }
        if let Some(a) = action {
            params.push(Box::new(a.to_string()));
            sql.push_str(&format!(" AND action = ?{}", params.len()));
        }
        if let Some(s) = since_rfc3339 {
            params.push(Box::new(s.to_string()));
            sql.push_str(&format!(" AND ts >= ?{}", params.len()));
        }
        if let Some(c) = before_seq {
            params.push(Box::new(c));
            sql.push_str(&format!(" AND seq < ?{}", params.len()));
        }
        params.push(Box::new(limit as i64));
        sql.push_str(&format!(" ORDER BY seq DESC LIMIT ?{}", params.len()));

        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(refs.as_slice(), |r| {
            Ok(AuditRecord {
                seq: r.get(0)?,
                ts: r.get(1)?,
                org_id: r.get(2)?,
                actor_member_id: r.get(3)?,
                action: r.get(4)?,
                detail: r.get(5)?,
                entry_hash: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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

        let audit_key: i64 = 1;
        let head = chain::HeadRef {
            table: "audit_head",
            key_col: "id",
            key: &audit_key,
        };
        let (prev, count) = chain::read_head(&tx, &head)?;

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
        chain::write_head(&tx, &head, &entry_hash, count + 1)?;
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
        let audit_key: i64 = 1;
        let head = chain::HeadRef {
            table: "audit_head",
            key_col: "id",
            key: &audit_key,
        };
        // Seed the walk from the sealed checkpoint (genesis when nothing sealed).
        let (sealed_hash, sealed_count) = audit_checkpoint(&conn)?;
        chain::verify(
            &conn,
            "SELECT ts, org_id, actor_member_id, action, detail, prev_hash, entry_hash
             FROM audit_log ORDER BY seq ASC",
            params![],
            &head,
            (sealed_hash.as_str(), sealed_count),
            |r| {
                let ts: String = r.get(0)?;
                let org_id: Option<String> = r.get(1)?;
                let actor: Option<String> = r.get(2)?;
                let action: String = r.get(3)?;
                let detail: String = r.get(4)?;
                let prev_hash: String = r.get(5)?;
                let entry_hash: String = r.get(6)?;
                let recomputed = audit_hash(
                    &prev_hash,
                    &ts,
                    org_id.as_deref(),
                    actor.as_deref(),
                    &action,
                    &detail,
                );
                Ok((prev_hash, entry_hash, recomputed))
            },
        )
    }

    fn seal_audit_before(&self, cutoff_rfc3339: &str) -> Result<u64> {
        let mut guard = self.conn()?;
        // IMMEDIATE takes the write lock up front, serializing with append_audit
        // (also IMMEDIATE) so the entry_count read and the boundary count below
        // can't race a concurrent append and skew sealed_count.
        let tx = guard.transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Newest entry older than the cutoff becomes the new checkpoint boundary.
        let boundary: Option<(i64, String)> = tx
            .query_row(
                "SELECT seq, entry_hash FROM audit_log WHERE ts < ?1
                 ORDER BY seq DESC LIMIT 1",
                params![cutoff_rfc3339],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((bseq, bhash)) = boundary else {
            return Ok(0);
        };

        // entry_count is the chain length (monotonic, survives pruning).
        let entry_count: i64 =
            tx.query_row("SELECT entry_count FROM audit_head WHERE id = 1", [], |r| {
                r.get(0)
            })?;
        let retained_after: i64 = tx.query_row(
            "SELECT COUNT(*) FROM audit_log WHERE seq > ?1",
            params![bseq],
            |r| r.get(0),
        )?;
        let sealed_count = entry_count - retained_after;

        let pruned = tx.execute("DELETE FROM audit_log WHERE seq <= ?1", params![bseq])?;
        tx.execute(
            "UPDATE audit_head SET sealed_hash = ?1, sealed_count = ?2 WHERE id = 1",
            params![bhash, sealed_count],
        )?;
        tx.commit()?;
        Ok(pruned as u64)
    }

    fn provision_member(
        &self,
        org_id: &str,
        display_name: &str,
        role: Role,
        email: &str,
    ) -> Result<String> {
        let member_id = ids::generate_id("mbr");
        let mut guard = self.conn()?;
        // Atomic: a failed identity insert (e.g. duplicate email) must roll back
        // the member row, so we never leave a half-provisioned account.
        let tx = guard.transaction()?;
        tx.execute(
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
        tx.execute(
            "INSERT INTO member_identity (member_id, email) VALUES (?1, ?2)",
            params![member_id, email],
        )
        .context("failed to set member email (already in use?)")?;
        tx.commit()?;
        Ok(member_id)
    }

    fn find_member_by_email(&self, email: &str) -> Result<Option<Principal>> {
        let conn = self.conn()?;
        principal_row(
            &conn,
            "SELECT m.id, m.org_id, m.role FROM member m
             JOIN member_identity i ON i.member_id = m.id WHERE i.email = ?1 AND m.active = 1",
            email,
        )
    }

    fn find_member_by_oidc_subject(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<Principal>> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT m.id, m.org_id, m.role FROM member m
                 JOIN member_identity i ON i.member_id = m.id
                 WHERE i.oidc_issuer = ?1 AND i.oidc_subject = ?2 AND m.active = 1",
                params![issuer, subject],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        match row {
            Some((member_id, org_id, role)) => Ok(Some(Principal {
                org_id,
                member_id,
                role: Role::parse(&role)?,
            })),
            None => Ok(None),
        }
    }

    fn bind_oidc_subject(&self, member_id: &str, issuer: &str, subject: &str) -> Result<bool> {
        // Only bind when no subject is set yet; never overwrite an existing
        // binding (that would let a different IdP account on the same email take
        // over the member).
        let n = self.conn()?.execute(
            "UPDATE member_identity SET oidc_issuer = ?2, oidc_subject = ?3
             WHERE member_id = ?1 AND oidc_subject IS NULL",
            params![member_id, issuer, subject],
        )?;
        Ok(n > 0)
    }

    fn put_login(
        &self,
        state: &str,
        pkce_verifier: &str,
        nonce: &str,
        browser_binding: &str,
    ) -> Result<()> {
        self.conn()?.execute(
            "INSERT INTO oidc_login (state, pkce_verifier, nonce, browser_binding, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                state,
                pkce_verifier,
                nonce,
                browser_binding,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    fn count_logins(&self) -> Result<u64> {
        let n: i64 = self
            .conn()?
            .query_row("SELECT COUNT(*) FROM oidc_login", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    fn prune_expired_logins(&self, ttl_secs: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(ttl_secs)).to_rfc3339();
        let n = self.conn()?.execute(
            "DELETE FROM oidc_login WHERE created_at < ?1",
            params![cutoff],
        )?;
        Ok(n as u64)
    }

    fn take_login(&self, state: &str) -> Result<Option<LoginTx>> {
        let conn = self.conn()?;
        let tx = conn
            .query_row(
                "SELECT pkce_verifier, nonce, browser_binding, created_at
                 FROM oidc_login WHERE state = ?1",
                params![state],
                |r| {
                    Ok(LoginTx {
                        pkce_verifier: r.get(0)?,
                        nonce: r.get(1)?,
                        browser_binding: r.get(2)?,
                        created_at: r.get(3)?,
                    })
                },
            )
            .optional()?;
        if tx.is_some() {
            conn.execute("DELETE FROM oidc_login WHERE state = ?1", params![state])?;
        }
        Ok(tx)
    }

    fn create_session(&self, member_id: &str, ttl_secs: i64) -> Result<String> {
        let id = ids::generate_id("sess");
        let now = chrono::Utc::now();
        let expires = now + chrono::Duration::seconds(ttl_secs);
        self.conn()?.execute(
            "INSERT INTO session (id, member_id, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, member_id, now.to_rfc3339(), expires.to_rfc3339()],
        )?;
        Ok(id)
    }

    fn session_principal(&self, session_id: &str) -> Result<Option<Principal>> {
        let conn = self.conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        let row = conn
            .query_row(
                "SELECT m.id, m.org_id, m.role FROM session s
                 JOIN member m ON m.id = s.member_id
                 WHERE s.id = ?1 AND s.expires_at > ?2 AND m.active = 1",
                params![session_id, now],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        match row {
            Some((member_id, org_id, role)) => Ok(Some(Principal {
                org_id,
                member_id,
                role: Role::parse(&role)?,
            })),
            None => Ok(None),
        }
    }

    fn delete_session(&self, session_id: &str) -> Result<bool> {
        let n = self
            .conn()?
            .execute("DELETE FROM session WHERE id = ?1", params![session_id])?;
        Ok(n > 0)
    }

    fn prune_expired_sessions(&self) -> Result<u64> {
        let now = chrono::Utc::now().to_rfc3339();
        let n = self
            .conn()?
            .execute("DELETE FROM session WHERE expires_at < ?1", params![now])?;
        Ok(n as u64)
    }

    fn prune_finished_jobs(&self, older_than_rfc3339: &str) -> Result<u64> {
        let n = self.conn()?.execute(
            "DELETE FROM job
             WHERE status IN ('completed', 'failed') AND updated_at < ?1",
            params![older_than_rfc3339],
        )?;
        Ok(n as u64)
    }

    fn enqueue_job(&self, org_id: &str, kind: &str, job_params: Option<&str>) -> Result<String> {
        let id = ids::generate_id("job");
        let now = chrono::Utc::now().to_rfc3339();
        self.conn()?
            .execute(
                "INSERT INTO job (id, org_id, kind, status, params, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?5)",
                params![id, org_id, kind, job_params, now],
            )
            .context("failed to enqueue job")?;
        Ok(id)
    }

    fn claim_next_job(&self) -> Result<Option<Job>> {
        let mut guard = self.conn()?;
        // IMMEDIATE so the select-then-update is atomic against other workers.
        let tx = guard.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = tx
            .query_row(
                "SELECT id, org_id, kind, params, created_at FROM job
                 WHERE status = 'queued' ORDER BY created_at ASC, id ASC LIMIT 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, org_id, kind, job_params, created_at)) = row else {
            return Ok(None);
        };
        let now = chrono::Utc::now().to_rfc3339();
        tx.execute(
            "UPDATE job SET status = 'running', updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        tx.commit()?;
        Ok(Some(Job {
            id,
            org_id,
            kind,
            status: "running".to_string(),
            result: None,
            error: None,
            params: job_params,
            created_at,
            updated_at: now,
        }))
    }

    fn complete_job(&self, job_id: &str, result_json: &str) -> Result<()> {
        self.conn()?.execute(
            "UPDATE job SET status = 'completed', result = ?2, updated_at = ?3 WHERE id = ?1",
            params![job_id, result_json, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    fn fail_job(&self, job_id: &str, error: &str) -> Result<()> {
        self.conn()?.execute(
            "UPDATE job SET status = 'failed', error = ?2, updated_at = ?3 WHERE id = ?1",
            params![job_id, error, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    fn get_job(&self, org_id: &str, job_id: &str) -> Result<Option<Job>> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT id, org_id, kind, status, result, error, params, created_at, updated_at
                 FROM job WHERE org_id = ?1 AND id = ?2",
                params![org_id, job_id],
                |r| {
                    Ok(Job {
                        id: r.get(0)?,
                        org_id: r.get(1)?,
                        kind: r.get(2)?,
                        status: r.get(3)?,
                        result: r.get(4)?,
                        error: r.get(5)?,
                        params: r.get(6)?,
                        created_at: r.get(7)?,
                        updated_at: r.get(8)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    fn list_jobs(&self, org_id: &str, limit: u32) -> Result<Vec<Job>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, org_id, kind, status, result, error, params, created_at, updated_at
             FROM job WHERE org_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![org_id, limit as i64], |r| {
            Ok(Job {
                id: r.get(0)?,
                org_id: r.get(1)?,
                kind: r.get(2)?,
                status: r.get(3)?,
                result: r.get(4)?,
                error: r.get(5)?,
                params: r.get(6)?,
                created_at: r.get(7)?,
                updated_at: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn list_members(&self, org_id: &str) -> Result<Vec<MemberInfo>> {
        let conn = self.conn()?;
        // LEFT JOIN so members without an SSO identity (e.g. CLI-created) still
        // appear; `sso_bound` is whether a verified OIDC subject is bound.
        let mut stmt = conn.prepare(
            "SELECT m.id, m.display_name, m.role, m.active, i.email, i.oidc_subject
             FROM member m
             LEFT JOIN member_identity i ON i.member_id = m.id
             WHERE m.org_id = ?1
             ORDER BY m.display_name",
        )?;
        let rows = stmt.query_map([org_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, display_name, role, active, email, oidc_subject) = row?;
            out.push(MemberInfo {
                id,
                display_name,
                role,
                email,
                sso_bound: oidc_subject.is_some(),
                active: active != 0,
            });
        }
        Ok(out)
    }

    fn scim_token_created_at(&self, org_id: &str) -> Result<Option<String>> {
        let conn = self.conn()?;
        let ts = conn
            .query_row(
                "SELECT created_at FROM scim_token WHERE org_id = ?1
                 ORDER BY created_at DESC LIMIT 1",
                [org_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(ts)
    }

    fn put_compliance_snapshots(&self, org_id: &str, snaps: &[ComplianceSnapshot]) -> Result<()> {
        let mut guard = self.conn()?;
        let tx = guard.transaction()?;
        for snap in snaps {
            tx.execute(
                "INSERT INTO compliance_snapshot
                     (id, org_id, repo_id, repo_name, policy_name, policy_version,
                      evaluated_at, ai_ranges, violations, high, medium, low)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    ids::generate_id("cmp"),
                    org_id,
                    snap.repo_id,
                    snap.repo_name,
                    snap.policy_name,
                    snap.policy_version,
                    snap.evaluated_at,
                    snap.ai_ranges,
                    snap.violations,
                    snap.high,
                    snap.medium,
                    snap.low,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn latest_compliance(&self, org_id: &str) -> Result<Vec<ComplianceSnapshot>> {
        let conn = self.conn()?;
        // Newest snapshot per repo: pick the max (evaluated_at, id) so a tie on
        // the timestamp is still deterministic.
        let mut stmt = conn.prepare(
            "SELECT cs.repo_id, cs.repo_name, cs.policy_name, cs.policy_version,
                    cs.evaluated_at, cs.ai_ranges, cs.violations, cs.high, cs.medium, cs.low
             FROM compliance_snapshot cs
             JOIN (
                 SELECT repo_id, MAX(evaluated_at || '|' || id) AS mk
                 FROM compliance_snapshot WHERE org_id = ?1 GROUP BY repo_id
             ) latest
               ON cs.repo_id = latest.repo_id
              AND (cs.evaluated_at || '|' || cs.id) = latest.mk
             WHERE cs.org_id = ?1
             ORDER BY cs.evaluated_at DESC, cs.repo_name",
        )?;
        let rows = stmt.query_map([org_id], |r| {
            Ok(ComplianceSnapshot {
                repo_id: r.get(0)?,
                repo_name: r.get(1)?,
                policy_name: r.get(2)?,
                policy_version: r.get(3)?,
                evaluated_at: r.get(4)?,
                ai_ranges: r.get(5)?,
                violations: r.get(6)?,
                high: r.get(7)?,
                medium: r.get(8)?,
                low: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn scim_create_group(
        &self,
        org_id: &str,
        display_name: &str,
        external_id: Option<&str>,
        members: &[String],
    ) -> Result<ScimGroup> {
        let id = ids::generate_id("grp");
        let now = chrono::Utc::now().to_rfc3339();
        let mut guard = self.conn()?;
        let tx = guard.transaction()?;
        tx.execute(
            "INSERT INTO scim_group (id, org_id, display_name, external_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, org_id, display_name, external_id, now],
        )
        .context("failed to create group (duplicate displayName?)")?;
        let members = set_group_members(&tx, org_id, &id, members)?;
        tx.commit()?;
        Ok(ScimGroup {
            id,
            org_id: org_id.to_string(),
            display_name: display_name.to_string(),
            external_id: external_id.map(str::to_string),
            members,
        })
    }

    fn scim_list_groups(&self, org_id: &str, name_filter: Option<&str>) -> Result<Vec<ScimGroup>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, display_name, external_id FROM scim_group
             WHERE org_id = ?1 AND (?2 IS NULL OR display_name = ?2) ORDER BY display_name",
        )?;
        let rows: Vec<(String, String, Option<String>)> = stmt
            .query_map(params![org_id, name_filter], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .filter_map(std::result::Result::ok)
            .collect();
        let mut out = Vec::new();
        for (id, display_name, external_id) in rows {
            let members = group_member_ids(&conn, &id)?;
            out.push(ScimGroup {
                id,
                org_id: org_id.to_string(),
                display_name,
                external_id,
                members,
            });
        }
        Ok(out)
    }

    fn scim_get_group(&self, org_id: &str, group_id: &str) -> Result<Option<ScimGroup>> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT display_name, external_id FROM scim_group WHERE org_id = ?1 AND id = ?2",
                params![org_id, group_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        match row {
            Some((display_name, external_id)) => {
                let members = group_member_ids(&conn, group_id)?;
                Ok(Some(ScimGroup {
                    id: group_id.to_string(),
                    org_id: org_id.to_string(),
                    display_name,
                    external_id,
                    members,
                }))
            }
            None => Ok(None),
        }
    }

    fn scim_update_group(
        &self,
        org_id: &str,
        group_id: &str,
        display_name: Option<&str>,
        external_id: Option<&str>,
        members: Option<&[String]>,
    ) -> Result<Option<ScimGroup>> {
        {
            let mut guard = self.conn()?;
            let tx = guard.transaction()?;
            let exists = tx
                .query_row(
                    "SELECT 1 FROM scim_group WHERE org_id = ?1 AND id = ?2",
                    params![org_id, group_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !exists {
                return Ok(None);
            }
            if let Some(name) = display_name {
                tx.execute(
                    "UPDATE scim_group SET display_name = ?2 WHERE id = ?1",
                    params![group_id, name],
                )?;
            }
            if let Some(ext) = external_id {
                tx.execute(
                    "UPDATE scim_group SET external_id = ?2 WHERE id = ?1",
                    params![group_id, ext],
                )?;
            }
            if let Some(new_members) = members {
                // Recompute the union of old + new members after the swap.
                let old = group_member_ids(&tx, group_id)?;
                tx.execute(
                    "DELETE FROM scim_group_member WHERE group_id = ?1",
                    params![group_id],
                )?;
                set_group_members(&tx, org_id, group_id, new_members)?;
                let mut affected: std::collections::BTreeSet<String> = old.into_iter().collect();
                affected.extend(new_members.iter().cloned());
                for m in affected {
                    recompute_member_role(&tx, &m)?;
                }
            } else if display_name.is_some() {
                // Renaming may change the group's role mapping.
                for m in group_member_ids(&tx, group_id)? {
                    recompute_member_role(&tx, &m)?;
                }
            }
            tx.execute(
                "UPDATE scim_group SET updated_at = ?2 WHERE id = ?1",
                params![group_id, chrono::Utc::now().to_rfc3339()],
            )?;
            tx.commit()?;
        }
        self.scim_get_group(org_id, group_id)
    }

    fn scim_delete_group(&self, org_id: &str, group_id: &str) -> Result<bool> {
        let mut guard = self.conn()?;
        let tx = guard.transaction()?;
        let exists = tx
            .query_row(
                "SELECT 1 FROM scim_group WHERE org_id = ?1 AND id = ?2",
                params![org_id, group_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Ok(false);
        }
        let members = group_member_ids(&tx, group_id)?;
        tx.execute(
            "DELETE FROM scim_group_member WHERE group_id = ?1",
            params![group_id],
        )?;
        tx.execute("DELETE FROM scim_group WHERE id = ?1", params![group_id])?;
        for m in members {
            recompute_member_role(&tx, &m)?;
        }
        tx.commit()?;
        Ok(true)
    }

    fn requeue_running_jobs(&self) -> Result<u64> {
        let n = self.conn()?.execute(
            "UPDATE job SET status = 'queued', updated_at = ?1 WHERE status = 'running'",
            params![chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(n as u64)
    }

    fn create_scim_token(&self, org_id: &str) -> Result<GeneratedToken> {
        let token = auth::generate_token()?;
        self.conn()?
            .execute(
                "INSERT INTO scim_token (token_id, org_id, secret_hash, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    token.token_id,
                    org_id,
                    token.secret_hash,
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .context("failed to create SCIM token (does the org exist?)")?;
        Ok(token)
    }

    fn authenticate_scim(&self, token: &str) -> Result<Option<String>> {
        let Some((token_id, secret)) = auth::parse_token(token) else {
            return Ok(None);
        };
        let row: Option<(String, String)> = {
            let conn = self.conn()?;
            conn.query_row(
                "SELECT secret_hash, org_id FROM scim_token WHERE token_id = ?1",
                [token_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .context("SCIM token lookup failed")?
        };
        let Some((secret_hash, org_id)) = row else {
            return Ok(None);
        };
        if !auth::verify_secret(&secret, &secret_hash) {
            return Ok(None);
        }
        Ok(Some(org_id))
    }

    fn scim_create_user(
        &self,
        org_id: &str,
        email: &str,
        display_name: &str,
        role: Role,
        external_id: Option<&str>,
    ) -> Result<ScimUser> {
        let member_id = ids::generate_id("mbr");
        let mut guard = self.conn()?;
        let tx = guard.transaction()?;
        tx.execute(
            "INSERT INTO member (id, org_id, display_name, role, created_at, active)
             VALUES (?1, ?2, ?3, ?4, ?5, 1)",
            params![
                member_id,
                org_id,
                display_name,
                role.as_str(),
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .context("failed to create member")?;
        tx.execute(
            "INSERT INTO member_identity (member_id, email, external_id) VALUES (?1, ?2, ?3)",
            params![member_id, email, external_id],
        )
        .context("failed to set member email (already in use?)")?;
        tx.commit()?;
        Ok(ScimUser {
            member_id,
            email: email.to_string(),
            display_name: display_name.to_string(),
            role,
            active: true,
            external_id: external_id.map(str::to_string),
        })
    }

    fn scim_list_users(&self, org_id: &str, email_filter: Option<&str>) -> Result<Vec<ScimUser>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT m.id, m.display_name, m.role, m.active, i.email, i.external_id
             FROM member m JOIN member_identity i ON i.member_id = m.id
             WHERE m.org_id = ?1 AND (?2 IS NULL OR i.email = ?2)
             ORDER BY i.email",
        )?;
        let rows = stmt.query_map(params![org_id, email_filter], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (member_id, display_name, role, active, email, external_id) = row?;
            out.push(ScimUser {
                member_id,
                email,
                display_name,
                role: Role::parse(&role)?,
                active: active != 0,
                external_id,
            });
        }
        Ok(out)
    }

    fn scim_get_user(&self, org_id: &str, member_id: &str) -> Result<Option<ScimUser>> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT m.display_name, m.role, m.active, i.email, i.external_id
                 FROM member m JOIN member_identity i ON i.member_id = m.id
                 WHERE m.org_id = ?1 AND m.id = ?2",
                params![org_id, member_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()?;
        match row {
            Some((display_name, role, active, email, external_id)) => Ok(Some(ScimUser {
                member_id: member_id.to_string(),
                email,
                display_name,
                role: Role::parse(&role)?,
                active: active != 0,
                external_id,
            })),
            None => Ok(None),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn scim_update_user(
        &self,
        org_id: &str,
        member_id: &str,
        email: Option<&str>,
        display_name: Option<&str>,
        role: Option<Role>,
        active: Option<bool>,
        external_id: Option<&str>,
    ) -> Result<Option<ScimUser>> {
        {
            let conn = self.conn()?;
            // Verify the member exists in this org first (tenant scoping).
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM member WHERE id = ?1 AND org_id = ?2",
                    params![member_id, org_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !exists {
                return Ok(None);
            }
            if let Some(addr) = email {
                conn.execute(
                    "UPDATE member_identity SET email = ?2 WHERE member_id = ?1",
                    params![member_id, addr],
                )?;
            }
            if let Some(name) = display_name {
                conn.execute(
                    "UPDATE member SET display_name = ?2 WHERE id = ?1",
                    params![member_id, name],
                )?;
            }
            if let Some(r) = role {
                conn.execute(
                    "UPDATE member SET role = ?2 WHERE id = ?1",
                    params![member_id, r.as_str()],
                )?;
            }
            if let Some(a) = active {
                conn.execute(
                    "UPDATE member SET active = ?2 WHERE id = ?1",
                    params![member_id, a as i64],
                )?;
            }
            if let Some(ext) = external_id {
                conn.execute(
                    "UPDATE member_identity SET external_id = ?2 WHERE member_id = ?1",
                    params![member_id, ext],
                )?;
            }
        }
        self.scim_get_user(org_id, member_id)
    }
}

/// Read the member ids belonging to a group.
fn group_member_ids(conn: &Connection, group_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT member_id FROM scim_group_member WHERE group_id = ?1 ORDER BY member_id",
    )?;
    let ids = stmt
        .query_map(params![group_id], |r| r.get::<_, String>(0))?
        .filter_map(std::result::Result::ok)
        .collect();
    Ok(ids)
}

/// Replace a group's membership with `members` (each must belong to `org_id`),
/// then recompute each member's role. Returns the members that were set.
fn set_group_members(
    conn: &Connection,
    org_id: &str,
    group_id: &str,
    members: &[String],
) -> Result<Vec<String>> {
    let mut set = Vec::new();
    for m in members {
        let in_org = conn
            .query_row(
                "SELECT 1 FROM member WHERE id = ?1 AND org_id = ?2",
                params![m, org_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !in_org {
            bail!("member {m} not found in org {org_id}");
        }
        conn.execute(
            "INSERT OR IGNORE INTO scim_group_member (group_id, member_id) VALUES (?1, ?2)",
            params![group_id, m],
        )?;
        set.push(m.clone());
    }
    for m in &set {
        recompute_member_role(conn, m)?;
    }
    Ok(set)
}

/// Recompute a member's org role from the role-mapping groups they belong to.
/// With group sync, membership **owns** the role: it is set to the highest
/// mapped group role, or to the `viewer` baseline when the member is in no
/// role-mapping group — so removing the last `tellur-admin` group revokes the
/// elevated role (no leftover access).
fn recompute_member_role(conn: &Connection, member_id: &str) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT g.display_name FROM scim_group_member gm
         JOIN scim_group g ON g.id = gm.group_id WHERE gm.member_id = ?1",
    )?;
    let names: Vec<String> = stmt
        .query_map(params![member_id], |r| r.get::<_, String>(0))?
        .filter_map(std::result::Result::ok)
        .collect();
    let mut role = Role::Viewer;
    for n in names {
        if let Some(r) = role_from_group_name(&n) {
            role = role.max(r);
        }
    }
    conn.execute(
        "UPDATE member SET role = ?2 WHERE id = ?1",
        params![member_id, role.as_str()],
    )?;
    Ok(())
}

/// Helper: read at most one `(member_id, org_id, role)` row into a [`Principal`].
fn principal_row(conn: &Connection, sql: &str, key: &str) -> Result<Option<Principal>> {
    let row = conn
        .query_row(sql, params![key], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .optional()?;
    match row {
        Some((member_id, org_id, role)) => Ok(Some(Principal {
            org_id,
            member_id,
            role: Role::parse(&role)?,
        })),
        None => Ok(None),
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
    fn migrate_upgrades_legacy_tables_with_new_columns() {
        // Simulate a pre-v11 database: member without `active`, member_identity
        // without oidc_issuer/external_id, oidc_login without browser_binding.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE org (id TEXT PRIMARY KEY, name TEXT NOT NULL, created_at TEXT NOT NULL);
             CREATE TABLE member (id TEXT PRIMARY KEY, org_id TEXT NOT NULL, display_name TEXT NOT NULL, role TEXT NOT NULL, created_at TEXT NOT NULL);
             CREATE TABLE member_identity (member_id TEXT PRIMARY KEY, email TEXT UNIQUE, oidc_subject TEXT UNIQUE);
             CREATE TABLE oidc_login (state TEXT PRIMARY KEY, pkce_verifier TEXT NOT NULL, nonce TEXT NOT NULL, created_at TEXT NOT NULL);
             INSERT INTO org VALUES ('o1','Acme','t');
             INSERT INTO member VALUES ('m1','o1','Alice','admin','t');
             INSERT INTO member_identity (member_id, email) VALUES ('m1','a@b.test');",
        )
        .unwrap();
        let store = SqliteStore {
            conn: Mutex::new(conn),
        };
        // migrate() must add the missing columns rather than just bump version.
        store.migrate().unwrap();

        // The pre-existing member is still authable (active defaulted to 1) and
        // resolvable by email (auth paths query `m.active`).
        assert!(store.find_member_by_email("a@b.test").unwrap().is_some());
        // The new columns now exist and round-trip.
        let token = store.create_token("m1").unwrap().plaintext;
        assert!(store.authenticate(&token).unwrap().is_some());
        store.put_login("s", "v", "n", "bind").unwrap();
        assert_eq!(
            store.take_login("s").unwrap().unwrap().browser_binding,
            "bind"
        );
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
    fn list_and_report_aggregate_events() {
        let s = store();
        let org = s.create_org("Acme").unwrap();
        let repo = s.ensure_repo(&org.id, "app").unwrap();
        s.append_events(
            &org.id,
            &repo.id,
            &[ingest_event("a"), ingest_event("b"), ingest_event("c")],
        )
        .unwrap();

        let repos = s.list_repos(&org.id).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].event_count, 3);

        // Pagination newest-first.
        let page = s.list_events(&org.id, &repo.id, 2, None).unwrap();
        assert_eq!(page.len(), 2);
        assert!(page[0].seq > page[1].seq);
        let next = s
            .list_events(&org.id, &repo.id, 2, Some(page[1].seq))
            .unwrap();
        assert_eq!(next.len(), 1);

        let report = s.org_report(&org.id).unwrap();
        assert_eq!(report.total_events, 3);
        assert_eq!(report.distinct_sessions, 1);
        assert_eq!(report.by_type.get("file.write"), Some(&3));
        assert_eq!(report.by_actor.get("agent"), Some(&3));
        assert_eq!(report.repos.len(), 1);
    }

    #[test]
    fn list_events_errors_on_corrupt_payload() {
        let s = store();
        let org = s.create_org("Acme").unwrap();
        let repo = s.ensure_repo(&org.id, "app").unwrap();
        s.append_events(&org.id, &repo.id, &[ingest_event("a")])
            .unwrap();
        {
            let conn = s.conn().unwrap();
            conn.execute("UPDATE event SET payload = 'not json'", [])
                .unwrap();
        }
        // Corruption is surfaced as an error, not masked as null.
        assert!(s.list_events(&org.id, &repo.id, 10, None).is_err());
    }

    #[test]
    fn policy_versions_bump_and_export_is_scoped() {
        let s = store();
        let org = s.create_org("Acme").unwrap();
        assert_eq!(s.put_policy(&org.id, "default", "version: 1").unwrap(), 1);
        assert_eq!(s.put_policy(&org.id, "default", "version: 1").unwrap(), 2);
        let doc = s.get_policy(&org.id, "default").unwrap().unwrap();
        assert_eq!(doc.version, 2);
        assert_eq!(s.list_policies(&org.id).unwrap().len(), 1);
        assert!(s.get_policy(&org.id, "nope").unwrap().is_none());

        let repo = s.ensure_repo(&org.id, "app").unwrap();
        s.append_events(&org.id, &repo.id, &[ingest_event("a")])
            .unwrap();
        assert_eq!(s.export_events(&org.id).unwrap().len(), 1);
        // Another org exports nothing.
        assert!(s.export_events("org_other").unwrap().is_empty());
        assert!(s.list_policies("org_other").unwrap().is_empty());
    }

    #[test]
    fn reads_are_tenant_scoped() {
        let s = store();
        let org = s.create_org("Acme").unwrap();
        let repo = s.ensure_repo(&org.id, "app").unwrap();
        s.append_events(&org.id, &repo.id, &[ingest_event("a")])
            .unwrap();
        // A different org sees none of it.
        assert!(s.list_repos("org_other").unwrap().is_empty());
        assert!(
            s.list_events("org_other", &repo.id, 10, None)
                .unwrap()
                .is_empty()
        );
        assert_eq!(s.org_report("org_other").unwrap().total_events, 0);
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
