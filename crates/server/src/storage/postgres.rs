//! Postgres implementation of [`Store`] — the horizontally-scalable backend.
//!
//! Mirrors `SqliteStore` semantics exactly (same hash chains, tenant scoping,
//! and tamper-evidence) over a connection pool. Chain appends take a per-scope
//! `pg_advisory_xact_lock` so the read-head + insert + head-update are atomic
//! across pooled connections (the Postgres equivalent of SQLite's
//! `BEGIN IMMEDIATE`). Uses `NoTls`: run behind a TLS-terminating proxy.

use anyhow::{Context, Result, bail};
use r2d2_postgres::PostgresConnectionManager;
use r2d2_postgres::postgres::NoTls;
use r2d2_postgres::postgres::types::ToSql;
use tellur_core::schema::ids;
use tellur_core::schema::types::FileAttribution;

use r2d2_postgres::postgres::GenericClient;

use super::{
    ActivityBucket, ActivityGroup, AuditEntry, AuditRecord, ComplianceSnapshot, DeviceAuth,
    DevicePoll, IngestEvent, Job, LoginTx, MemberInfo, Org, OrgReport, PolicyDoc, PolicySummary,
    Repo, RepoFacts, RepoRoleGrant, RepoSource, RepoSummary, ScimGroup, ScimUser, SessionSummary,
    Store, StoredEvent, role_from_group_name,
};
use crate::auth::{self, GeneratedToken, Principal, Role};

type Pool = r2d2::Pool<PostgresConnectionManager<NoTls>>;
type PooledClient = r2d2::PooledConnection<PostgresConnectionManager<NoTls>>;

/// A Postgres-backed store.
pub struct PostgresStore {
    pool: Pool,
}

impl PostgresStore {
    /// Connect to `database_url` and build a connection pool.
    pub fn connect(database_url: &str) -> Result<Self> {
        let config: r2d2_postgres::postgres::Config = database_url
            .parse()
            .with_context(|| "invalid Postgres connection string")?;
        let manager = PostgresConnectionManager::new(config, NoTls);
        let pool = r2d2::Pool::builder()
            .build(manager)
            .context("failed to build Postgres connection pool")?;
        Ok(Self { pool })
    }

    fn client(&self) -> Result<PooledClient> {
        self.pool.get().context("failed to get Postgres connection")
    }
}

/// Derive a stable 64-bit advisory-lock key from a chain scope string.
///
/// Must be deterministic **across builds and Rust releases**: in a horizontally
/// scaled / rolling deploy, every hub process has to map the same scope to the
/// same lock integer, otherwise two writers could lock different keys for the
/// same chain and append from the same head concurrently. `DefaultHasher` makes
/// no such cross-version guarantee, so we derive the key from SHA-256 (via the
/// core `hash_content`) truncated to 64 bits.
fn advisory_key(scope: &str) -> i64 {
    let hex = ids::hash_content(scope);
    let bytes = u64::from_str_radix(&hex[..16], 16).expect("hash_content returns hex");
    bytes as i64
}

/// Read a chain head (tip hash + length) within a transaction, or genesis.
fn read_head(
    tx: &mut r2d2_postgres::postgres::Transaction,
    sql: &str,
    key: &(dyn ToSql + Sync),
) -> Result<(String, i64)> {
    match tx.query_opt(sql, &[key])? {
        Some(row) => Ok((row.get(0), row.get(1))),
        None => Ok((String::new(), 0)),
    }
}

impl Store for PostgresStore {
    fn migrate(&self) -> Result<()> {
        let mut client = self.client()?;
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS schema_meta (
                     key TEXT PRIMARY KEY, value TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS org (
                     id TEXT PRIMARY KEY, name TEXT NOT NULL, created_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS member (
                     id TEXT PRIMARY KEY,
                     org_id TEXT NOT NULL REFERENCES org(id),
                     display_name TEXT NOT NULL, role TEXT NOT NULL, created_at TEXT NOT NULL,
                     active BOOLEAN NOT NULL DEFAULT TRUE
                 );
                 CREATE INDEX IF NOT EXISTS idx_member_org ON member(org_id);
                 CREATE TABLE IF NOT EXISTS api_token (
                     token_id TEXT PRIMARY KEY,
                     member_id TEXT NOT NULL REFERENCES member(id),
                     secret_hash TEXT NOT NULL, created_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS audit_log (
                     seq BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                     ts TEXT NOT NULL, org_id TEXT, actor_member_id TEXT,
                     action TEXT NOT NULL, detail TEXT NOT NULL,
                     prev_hash TEXT NOT NULL, entry_hash TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS audit_head (
                     id INTEGER PRIMARY KEY CHECK (id = 1),
                     head_hash TEXT NOT NULL, entry_count BIGINT NOT NULL,
                     sealed_hash TEXT NOT NULL DEFAULT '',
                     sealed_count BIGINT NOT NULL DEFAULT 0
                 );
                 CREATE TABLE IF NOT EXISTS repo (
                     id TEXT PRIMARY KEY,
                     org_id TEXT NOT NULL REFERENCES org(id),
                     name TEXT NOT NULL, created_at TEXT NOT NULL,
                     UNIQUE (org_id, name)
                 );
                 CREATE TABLE IF NOT EXISTS event (
                     seq BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                     id TEXT NOT NULL UNIQUE,
                     org_id TEXT NOT NULL,
                     repo_id TEXT NOT NULL REFERENCES repo(id),
                     session_id TEXT NOT NULL, ts TEXT NOT NULL,
                     event_type TEXT NOT NULL, actor TEXT NOT NULL, payload TEXT NOT NULL,
                     prev_hash TEXT NOT NULL, entry_hash TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_event_repo ON event(repo_id, seq);
                 CREATE INDEX IF NOT EXISTS idx_event_org ON event(org_id);
                 CREATE TABLE IF NOT EXISTS event_head (
                     repo_id TEXT PRIMARY KEY REFERENCES repo(id),
                     head_hash TEXT NOT NULL, entry_count BIGINT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS policy (
                     org_id TEXT NOT NULL REFERENCES org(id),
                     name TEXT NOT NULL, content TEXT NOT NULL,
                     version BIGINT NOT NULL, updated_at TEXT NOT NULL,
                     PRIMARY KEY (org_id, name)
                 );
                 CREATE TABLE IF NOT EXISTS attribution (
                     org_id TEXT NOT NULL, repo_id TEXT NOT NULL REFERENCES repo(id),
                     file_path TEXT NOT NULL, git_blob_sha TEXT NOT NULL,
                     ranges_json TEXT NOT NULL, updated_at TEXT NOT NULL,
                     PRIMARY KEY (org_id, repo_id, file_path)
                 );
                 CREATE TABLE IF NOT EXISTS repo_source (
                     repo_id TEXT PRIMARY KEY REFERENCES repo(id),
                     org_id TEXT NOT NULL REFERENCES org(id),
                     template TEXT, raw_template TEXT, source_token TEXT,
                     updated_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS repo_role (
                     org_id TEXT NOT NULL REFERENCES org(id),
                     repo_id TEXT NOT NULL REFERENCES repo(id),
                     member_id TEXT NOT NULL REFERENCES member(id),
                     role TEXT NOT NULL, updated_at TEXT NOT NULL,
                     PRIMARY KEY (repo_id, member_id)
                 );
                 CREATE INDEX IF NOT EXISTS idx_repo_role_repo ON repo_role(repo_id);
                 CREATE TABLE IF NOT EXISTS member_identity (
                     member_id TEXT PRIMARY KEY REFERENCES member(id),
                     email TEXT UNIQUE, oidc_issuer TEXT, oidc_subject TEXT,
                     external_id TEXT,
                     UNIQUE (oidc_issuer, oidc_subject)
                 );
                 CREATE TABLE IF NOT EXISTS scim_token (
                     token_id TEXT PRIMARY KEY,
                     org_id TEXT NOT NULL REFERENCES org(id),
                     secret_hash TEXT NOT NULL, created_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS oidc_login (
                     state TEXT PRIMARY KEY, pkce_verifier TEXT NOT NULL,
                     nonce TEXT NOT NULL, browser_binding TEXT NOT NULL,
                     created_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS session (
                     id TEXT PRIMARY KEY, member_id TEXT NOT NULL REFERENCES member(id),
                     created_at TEXT NOT NULL, expires_at TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_session_member ON session(member_id);
                 CREATE TABLE IF NOT EXISTS device_auth (
                     device_code TEXT PRIMARY KEY, user_code TEXT NOT NULL UNIQUE,
                     status TEXT NOT NULL, member_id TEXT, created_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS job (
                     id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES org(id),
                     kind TEXT NOT NULL, status TEXT NOT NULL,
                     result TEXT, error TEXT, params TEXT,
                     created_at TEXT NOT NULL, updated_at TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_job_status ON job(status, created_at);
                 CREATE TABLE IF NOT EXISTS scim_group (
                     id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES org(id),
                     display_name TEXT NOT NULL, external_id TEXT,
                     created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                     UNIQUE (org_id, display_name)
                 );
                 CREATE TABLE IF NOT EXISTS scim_group_member (
                     group_id TEXT NOT NULL REFERENCES scim_group(id),
                     member_id TEXT NOT NULL REFERENCES member(id),
                     PRIMARY KEY (group_id, member_id)
                 );
                 CREATE INDEX IF NOT EXISTS idx_group_member ON scim_group_member(member_id);
                 CREATE TABLE IF NOT EXISTS compliance_snapshot (
                     id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES org(id),
                     repo_id TEXT NOT NULL, repo_name TEXT NOT NULL,
                     policy_name TEXT NOT NULL, policy_version BIGINT NOT NULL,
                     evaluated_at TEXT NOT NULL, ai_ranges BIGINT NOT NULL,
                     violations BIGINT NOT NULL, high BIGINT NOT NULL,
                     medium BIGINT NOT NULL, low BIGINT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_compliance_latest
                     ON compliance_snapshot(org_id, repo_id, evaluated_at);",
            )
            .context("failed to create schema")?;
        // Additive migrations for columns introduced after a table's first
        // version (CREATE TABLE IF NOT EXISTS no-ops on existing tables, so an
        // upgraded DB would otherwise be missing e.g. member.active).
        client
            .batch_execute(
                "ALTER TABLE audit_head ADD COLUMN IF NOT EXISTS sealed_hash TEXT NOT NULL DEFAULT '';
                 ALTER TABLE audit_head ADD COLUMN IF NOT EXISTS sealed_count BIGINT NOT NULL DEFAULT 0;
                 ALTER TABLE repo_source ADD COLUMN IF NOT EXISTS raw_template TEXT;
                 ALTER TABLE repo_source ADD COLUMN IF NOT EXISTS source_token TEXT;
                 ALTER TABLE repo_source ALTER COLUMN template DROP NOT NULL;
                 ALTER TABLE job ADD COLUMN IF NOT EXISTS params TEXT;
                 ALTER TABLE member ADD COLUMN IF NOT EXISTS active BOOLEAN NOT NULL DEFAULT TRUE;
                 ALTER TABLE member_identity ADD COLUMN IF NOT EXISTS oidc_issuer TEXT;
                 ALTER TABLE member_identity ADD COLUMN IF NOT EXISTS external_id TEXT;
                 ALTER TABLE oidc_login ADD COLUMN IF NOT EXISTS browser_binding TEXT NOT NULL DEFAULT '';
                 CREATE UNIQUE INDEX IF NOT EXISTS idx_member_identity_oidc
                     ON member_identity(oidc_issuer, oidc_subject);",
            )
            .context("failed to apply column migrations")?;
        client
            .execute(
                "INSERT INTO schema_meta (key, value) VALUES ('schema_version', '17')
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                &[],
            )
            .context("failed to record schema version")?;
        Ok(())
    }

    fn health_check(&self) -> Result<()> {
        // Verify the schema is migrated (not just that the DB is reachable), so
        // a connected-but-unmigrated backend reports not-ready instead of
        // failing real requests with missing-table errors.
        let mut client = self.client()?;
        client
            .query_one(
                "SELECT value FROM schema_meta WHERE key = 'schema_version'",
                &[],
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
        self.client()?
            .execute(
                "INSERT INTO org (id, name, created_at) VALUES ($1, $2, $3)",
                &[&org.id, &org.name, &org.created_at],
            )
            .context("failed to create org")?;
        Ok(org)
    }

    fn create_member(&self, org_id: &str, display_name: &str, role: Role) -> Result<String> {
        let member_id = ids::generate_id("mbr");
        self.client()?
            .execute(
                "INSERT INTO member (id, org_id, display_name, role, created_at)
                 VALUES ($1, $2, $3, $4, $5)",
                &[
                    &member_id,
                    &org_id,
                    &display_name,
                    &role.as_str(),
                    &chrono::Utc::now().to_rfc3339(),
                ],
            )
            .context("failed to create member (does the org exist?)")?;
        Ok(member_id)
    }

    fn create_token(&self, member_id: &str) -> Result<GeneratedToken> {
        let token = auth::generate_token()?;
        self.client()?
            .execute(
                "INSERT INTO api_token (token_id, member_id, secret_hash, created_at)
                 VALUES ($1, $2, $3, $4)",
                &[
                    &token.token_id,
                    &member_id,
                    &token.secret_hash,
                    &chrono::Utc::now().to_rfc3339(),
                ],
            )
            .context("failed to create token (does the member exist?)")?;
        Ok(token)
    }

    fn authenticate(&self, token: &str) -> Result<Option<Principal>> {
        let Some((token_id, secret)) = auth::parse_token(token) else {
            return Ok(None);
        };
        let row = self.client()?.query_opt(
            "SELECT t.secret_hash, m.id, m.org_id, m.role
             FROM api_token t JOIN member m ON m.id = t.member_id
             WHERE t.token_id = $1 AND m.active = TRUE",
            &[&token_id],
        )?;
        let Some(row) = row else { return Ok(None) };
        let secret_hash: String = row.get(0);
        if !auth::verify_secret(&secret, &secret_hash) {
            return Ok(None);
        }
        Ok(Some(Principal {
            org_id: row.get(2),
            member_id: row.get(1),
            role: Role::parse(&row.get::<_, String>(3))?,
        }))
    }

    fn ensure_repo(&self, org_id: &str, name: &str) -> Result<Repo> {
        let id = ids::generate_id("repo");
        let mut client = self.client()?;
        client
            .execute(
                "INSERT INTO repo (id, org_id, name, created_at) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (org_id, name) DO NOTHING",
                &[&id, &org_id, &name, &chrono::Utc::now().to_rfc3339()],
            )
            .context("failed to create repo")?;
        let real_id: String = client
            .query_one(
                "SELECT id FROM repo WHERE org_id = $1 AND name = $2",
                &[&org_id, &name],
            )?
            .get(0);
        Ok(Repo {
            id: real_id,
            org_id: org_id.to_string(),
            name: name.to_string(),
        })
    }

    fn find_repo(&self, org_id: &str, repo: &str) -> Result<Option<Repo>> {
        let mut client = self.client()?;
        let row = client.query_opt(
            "SELECT id, name FROM repo WHERE org_id = $1 AND (id = $2 OR name = $2)
             ORDER BY (id = $2) DESC LIMIT 1",
            &[&org_id, &repo],
        )?;
        Ok(row.map(|r| Repo {
            id: r.get(0),
            org_id: org_id.to_string(),
            name: r.get(1),
        }))
    }

    fn get_repo_source(&self, org_id: &str, repo_id: &str) -> Result<RepoSource> {
        let row = self.client()?.query_opt(
            "SELECT template, raw_template, source_token FROM repo_source
             WHERE org_id = $1 AND repo_id = $2",
            &[&org_id, &repo_id],
        )?;
        Ok(row
            .map(|r| RepoSource {
                link: r.get(0),
                raw: r.get(1),
                token: r.get(2),
            })
            .unwrap_or_default())
    }

    fn set_repo_source(
        &self,
        org_id: &str,
        repo_id: &str,
        link: Option<&str>,
        raw: Option<&str>,
        token: Option<&str>,
    ) -> Result<()> {
        if link.is_none() && raw.is_none() && token.is_none() {
            self.client()?.execute(
                "DELETE FROM repo_source WHERE org_id = $1 AND repo_id = $2",
                &[&org_id, &repo_id],
            )?;
        } else {
            let now = chrono::Utc::now().to_rfc3339();
            self.client()?.execute(
                "INSERT INTO repo_source (repo_id, org_id, template, raw_template, source_token, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (repo_id) DO UPDATE SET template = excluded.template,
                                                     raw_template = excluded.raw_template,
                                                     source_token = excluded.source_token,
                                                     updated_at = excluded.updated_at",
                &[&repo_id, &org_id, &link, &raw, &token, &now],
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
        let mut client = self.client()?;
        let repo_ok = client
            .query_opt(
                "SELECT 1 FROM repo WHERE id = $1 AND org_id = $2",
                &[&repo_id, &org_id],
            )?
            .is_some();
        if !repo_ok {
            bail!("repo {repo_id} not found in org {org_id}");
        }
        let member_ok = client
            .query_opt(
                "SELECT 1 FROM member WHERE id = $1 AND org_id = $2",
                &[&member_id, &org_id],
            )?
            .is_some();
        if !member_ok {
            bail!("member {member_id} not found in org {org_id}");
        }
        client.execute(
            "INSERT INTO repo_role (org_id, repo_id, member_id, role, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (repo_id, member_id) DO UPDATE SET role = excluded.role,
                                                            updated_at = excluded.updated_at",
            &[
                &org_id,
                &repo_id,
                &member_id,
                &role.as_str(),
                &chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    fn remove_repo_role(&self, org_id: &str, repo_id: &str, member_id: &str) -> Result<bool> {
        let n = self.client()?.execute(
            "DELETE FROM repo_role WHERE org_id = $1 AND repo_id = $2 AND member_id = $3",
            &[&org_id, &repo_id, &member_id],
        )?;
        Ok(n > 0)
    }

    fn get_repo_role(&self, org_id: &str, repo_id: &str, member_id: &str) -> Result<Option<Role>> {
        let row = self.client()?.query_opt(
            "SELECT role FROM repo_role WHERE org_id = $1 AND repo_id = $2 AND member_id = $3",
            &[&org_id, &repo_id, &member_id],
        )?;
        row.map(|r| Role::parse(&r.get::<_, String>(0))).transpose()
    }

    fn list_repo_roles(&self, org_id: &str, repo_id: &str) -> Result<Vec<RepoRoleGrant>> {
        let rows = self.client()?.query(
            "SELECT member_id, role, updated_at FROM repo_role
             WHERE org_id = $1 AND repo_id = $2 ORDER BY member_id",
            &[&org_id, &repo_id],
        )?;
        Ok(rows
            .iter()
            .map(|r| RepoRoleGrant {
                member_id: r.get(0),
                role: r.get(1),
                updated_at: r.get(2),
            })
            .collect())
    }

    fn append_events(
        &self,
        org_id: &str,
        repo_id: &str,
        events: &[IngestEvent],
    ) -> Result<Vec<String>> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&advisory_key(&format!("event:{repo_id}"))],
        )?;

        let belongs = tx
            .query_opt(
                "SELECT 1 FROM repo WHERE id = $1 AND org_id = $2",
                &[&repo_id, &org_id],
            )?
            .is_some();
        if !belongs {
            bail!("repo {repo_id} not found in org {org_id}");
        }

        let (mut prev, mut count) = read_head(
            &mut tx,
            "SELECT head_hash, entry_count FROM event_head WHERE repo_id = $1",
            &repo_id,
        )?;

        let mut new_ids = Vec::with_capacity(events.len());
        for ev in events {
            let id = ids::generate_event_id();
            let prev_opt = (!prev.is_empty()).then_some(prev.as_str());
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
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                &[
                    &id,
                    &org_id,
                    &repo_id,
                    &ev.session_id,
                    &ev.timestamp,
                    &ev.event_type,
                    &ev.actor,
                    &payload_str,
                    &prev,
                    &entry_hash,
                ],
            )
            .context("failed to insert event")?;
            prev = entry_hash;
            count += 1;
            new_ids.push(id);
        }
        tx.execute(
            "INSERT INTO event_head (repo_id, head_hash, entry_count) VALUES ($1, $2, $3)
             ON CONFLICT (repo_id) DO UPDATE SET head_hash = excluded.head_hash,
                                                 entry_count = excluded.entry_count",
            &[&repo_id, &prev, &count],
        )?;
        tx.commit().context("failed to commit events")?;
        Ok(new_ids)
    }

    fn event_count(&self, org_id: &str, repo_id: &str) -> Result<u64> {
        let n: i64 = self
            .client()?
            .query_one(
                "SELECT COUNT(*) FROM event WHERE org_id = $1 AND repo_id = $2",
                &[&org_id, &repo_id],
            )?
            .get(0);
        Ok(n as u64)
    }

    fn verify_event_chain(&self, org_id: &str, repo_id: &str) -> Result<bool> {
        let mut client = self.client()?;
        let rows = client.query(
            "SELECT id, session_id, ts, event_type, actor, payload, prev_hash, entry_hash
             FROM event WHERE org_id = $1 AND repo_id = $2 ORDER BY seq ASC",
            &[&org_id, &repo_id],
        )?;
        let mut expected_prev = String::new();
        let mut counted: i64 = 0;
        for r in &rows {
            let id: String = r.get(0);
            let payload: String = r.get(5);
            let prev_hash: String = r.get(6);
            let entry_hash: String = r.get(7);
            if prev_hash != expected_prev {
                return Ok(false);
            }
            let payload_value: serde_json::Value = serde_json::from_str(&payload)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            let prev_opt = (!prev_hash.is_empty()).then_some(prev_hash.as_str());
            let recomputed = ids::hash_event(
                &id,
                &r.get::<_, String>(1),
                &r.get::<_, String>(2),
                &r.get::<_, String>(3),
                &r.get::<_, String>(4),
                &payload_value,
                prev_opt,
            );
            if recomputed != entry_hash {
                return Ok(false);
            }
            expected_prev = entry_hash;
            counted += 1;
        }
        let head = client.query_opt(
            "SELECT head_hash, entry_count FROM event_head WHERE repo_id = $1",
            &[&repo_id],
        )?;
        match head {
            Some(row) => {
                Ok(counted == row.get::<_, i64>(1) && expected_prev == row.get::<_, String>(0))
            }
            None => Ok(counted == 0),
        }
    }

    fn put_attributions(
        &self,
        org_id: &str,
        repo_id: &str,
        files: &[FileAttribution],
    ) -> Result<usize> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        let belongs = tx
            .query_opt(
                "SELECT 1 FROM repo WHERE id = $1 AND org_id = $2",
                &[&repo_id, &org_id],
            )?
            .is_some();
        if !belongs {
            bail!("repo {repo_id} not found in org {org_id}");
        }
        let now = chrono::Utc::now().to_rfc3339();
        for file in files {
            // Empty ranges = tombstone: drop the row (file lost its attribution,
            // e.g. deleted from the repo) instead of leaving stale ranges.
            if file.ranges.is_empty() {
                tx.execute(
                    "DELETE FROM attribution
                     WHERE org_id = $1 AND repo_id = $2 AND file_path = $3",
                    &[&org_id, &repo_id, &file.file_path],
                )
                .context("failed to delete attribution")?;
                continue;
            }
            let ranges_json = serde_json::to_string(&file.ranges)?;
            tx.execute(
                "INSERT INTO attribution
                     (org_id, repo_id, file_path, git_blob_sha, ranges_json, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (org_id, repo_id, file_path) DO UPDATE SET
                     git_blob_sha = excluded.git_blob_sha,
                     ranges_json = excluded.ranges_json,
                     updated_at = excluded.updated_at",
                &[
                    &org_id,
                    &repo_id,
                    &file.file_path,
                    &file.git_blob_sha,
                    &ranges_json,
                    &now,
                ],
            )
            .context("failed to upsert attribution")?;
        }
        tx.commit()?;
        Ok(files.len())
    }

    fn list_attributions(&self, org_id: &str, repo_id: &str) -> Result<Vec<FileAttribution>> {
        let rows = self.client()?.query(
            "SELECT file_path, git_blob_sha, ranges_json, updated_at
             FROM attribution WHERE org_id = $1 AND repo_id = $2 ORDER BY file_path",
            &[&org_id, &repo_id],
        )?;
        let mut out = Vec::new();
        for r in &rows {
            let file_path: String = r.get(0);
            let ranges_json: String = r.get(2);
            let ranges = serde_json::from_str(&ranges_json)
                .with_context(|| format!("corrupt attribution ranges for {file_path}"))?;
            out.push(FileAttribution {
                schema: "tellur.attribution.v1".to_string(),
                file_path,
                git_blob_sha: r.get(1),
                ranges,
                updated_at: r.get(3),
            });
        }
        Ok(out)
    }

    fn list_repos(&self, org_id: &str) -> Result<Vec<RepoSummary>> {
        let rows = self.client()?.query(
            "SELECT r.id, r.name, COUNT(e.seq)
             FROM repo r LEFT JOIN event e ON e.repo_id = r.id
             WHERE r.org_id = $1 GROUP BY r.id, r.name ORDER BY r.name",
            &[&org_id],
        )?;
        Ok(rows
            .iter()
            .map(|r| RepoSummary {
                id: r.get(0),
                name: r.get(1),
                event_count: r.get::<_, i64>(2) as u64,
            })
            .collect())
    }

    fn list_events(
        &self,
        org_id: &str,
        repo_id: &str,
        limit: u32,
        before_seq: Option<i64>,
    ) -> Result<Vec<StoredEvent>> {
        let cursor = before_seq.unwrap_or(i64::MAX);
        let rows = self.client()?.query(
            "SELECT seq, id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = $1 AND repo_id = $2 AND seq < $3
             ORDER BY seq DESC LIMIT $4",
            &[&org_id, &repo_id, &cursor, &(limit as i64)],
        )?;
        let mut out = Vec::new();
        for r in &rows {
            let id: String = r.get(1);
            let payload_str: String = r.get(6);
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq: r.get(0),
                id,
                repo_id: repo_id.to_string(),
                session_id: r.get(2),
                timestamp: r.get(3),
                event_type: r.get(4),
                actor: r.get(5),
                payload,
            });
        }
        Ok(out)
    }

    fn org_report(&self, org_id: &str) -> Result<OrgReport> {
        let mut client = self.client()?;
        let total_events: i64 = client
            .query_one("SELECT COUNT(*) FROM event WHERE org_id = $1", &[&org_id])?
            .get(0);
        let distinct_sessions: i64 = client
            .query_one(
                "SELECT COUNT(DISTINCT session_id) FROM event WHERE org_id = $1",
                &[&org_id],
            )?
            .get(0);
        let by_type = group_counts(&mut client, "event_type", org_id)?;
        let by_actor = group_counts(&mut client, "actor", org_id)?;
        // Return this connection to the pool before `list_repos` checks out a
        // second one; otherwise concurrent reports (>= pool size) could each
        // hold one connection while waiting for another, deadlocking the pool.
        drop(client);
        let repos = self.list_repos(org_id)?;
        Ok(OrgReport {
            org_id: org_id.to_string(),
            total_events: total_events as u64,
            distinct_sessions: distinct_sessions as u64,
            by_type,
            by_actor,
            repos,
        })
    }

    fn recent_org_events(&self, org_id: &str, limit: u32) -> Result<Vec<StoredEvent>> {
        let rows = self.client()?.query(
            "SELECT seq, id, repo_id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = $1 ORDER BY seq DESC LIMIT $2",
            &[&org_id, &(limit as i64)],
        )?;
        let mut out = Vec::new();
        for r in &rows {
            let id: String = r.get(1);
            let payload_str: String = r.get(7);
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq: r.get(0),
                id,
                repo_id: r.get(2),
                session_id: r.get(3),
                timestamp: r.get(4),
                event_type: r.get(5),
                actor: r.get(6),
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
        // The grouping column is an allow-listed constant, never user input.
        let sql = format!(
            "SELECT left(ts, 10) AS day, {col} AS key, COUNT(*) AS n
             FROM event WHERE org_id = $1 AND ts >= $2
             GROUP BY day, key ORDER BY day ASC, key ASC",
            col = group.column()
        );
        let rows = self.client()?.query(&sql, &[&org_id, &since_rfc3339])?;
        Ok(rows
            .iter()
            .map(|r| ActivityBucket {
                day: r.get(0),
                key: r.get(1),
                count: r.get::<_, i64>(2) as u64,
            })
            .collect())
    }

    fn repo_facts(&self, org_id: &str, repo_id: &str) -> Result<RepoFacts> {
        let mut client = self.client()?;
        let event_count: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM event WHERE org_id = $1 AND repo_id = $2",
                &[&org_id, &repo_id],
            )?
            .get(0);
        let last_activity: Option<String> = client
            .query_one(
                "SELECT MAX(ts) FROM event WHERE org_id = $1 AND repo_id = $2",
                &[&org_id, &repo_id],
            )?
            .get(0);
        let rows = client.query(
            "SELECT DISTINCT actor FROM event
             WHERE org_id = $1 AND repo_id = $2 ORDER BY actor",
            &[&org_id, &repo_id],
        )?;
        Ok(RepoFacts {
            event_count: event_count as u64,
            contributors: rows.iter().map(|r| r.get(0)).collect(),
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
        let rows = self.client()?.query(
            "SELECT session_id, COUNT(*) AS n, MIN(ts) AS f, MAX(ts) AS l,
                    string_agg(DISTINCT actor, ',') AS actors,
                    string_agg(DISTINCT repo_id, ',') AS repos
             FROM event
             WHERE org_id = $1
               AND ($2::text IS NULL OR repo_id = $2)
               AND ($3::text IS NULL OR actor = $3)
               AND ($4::text IS NULL OR ts >= $4)
             GROUP BY session_id ORDER BY l DESC LIMIT $5",
            &[&org_id, &repo_id, &actor, &since_rfc3339, &(limit as i64)],
        )?;
        Ok(rows
            .iter()
            .map(|r| SessionSummary {
                session_id: r.get(0),
                event_count: r.get::<_, i64>(1) as u64,
                first_ts: r.get(2),
                last_ts: r.get(3),
                actors: split_csv(r.get::<_, Option<String>>(4)),
                repos: split_csv(r.get::<_, Option<String>>(5)),
            })
            .collect())
    }

    fn session_events(
        &self,
        org_id: &str,
        session_id: &str,
        limit: u32,
    ) -> Result<Vec<StoredEvent>> {
        let rows = self.client()?.query(
            "SELECT seq, id, repo_id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = $1 AND session_id = $2 ORDER BY seq ASC LIMIT $3",
            &[&org_id, &session_id, &(limit as i64)],
        )?;
        let mut out = Vec::new();
        for r in &rows {
            let id: String = r.get(1);
            let payload_str: String = r.get(7);
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq: r.get(0),
                id,
                repo_id: r.get(2),
                session_id: r.get(3),
                timestamp: r.get(4),
                event_type: r.get(5),
                actor: r.get(6),
                payload,
            });
        }
        Ok(out)
    }

    fn put_policy(&self, org_id: &str, name: &str, content: &str) -> Result<i64> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&advisory_key(&format!("policy:{org_id}:{name}"))],
        )?;
        let current: i64 = tx
            .query_opt(
                "SELECT version FROM policy WHERE org_id = $1 AND name = $2",
                &[&org_id, &name],
            )?
            .map(|r| r.get(0))
            .unwrap_or(0);
        let version = current + 1;
        tx.execute(
            "INSERT INTO policy (org_id, name, content, version, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (org_id, name) DO UPDATE SET content = excluded.content,
                 version = excluded.version, updated_at = excluded.updated_at",
            &[
                &org_id,
                &name,
                &content,
                &version,
                &chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        tx.commit()?;
        Ok(version)
    }

    fn list_policies(&self, org_id: &str) -> Result<Vec<PolicySummary>> {
        let rows = self.client()?.query(
            "SELECT name, version, updated_at FROM policy WHERE org_id = $1 ORDER BY name",
            &[&org_id],
        )?;
        Ok(rows
            .iter()
            .map(|r| PolicySummary {
                name: r.get(0),
                version: r.get(1),
                updated_at: r.get(2),
            })
            .collect())
    }

    fn get_policy(&self, org_id: &str, name: &str) -> Result<Option<PolicyDoc>> {
        let row = self.client()?.query_opt(
            "SELECT name, content, version, updated_at FROM policy
             WHERE org_id = $1 AND name = $2",
            &[&org_id, &name],
        )?;
        Ok(row.map(|r| PolicyDoc {
            name: r.get(0),
            content: r.get(1),
            version: r.get(2),
            updated_at: r.get(3),
        }))
    }

    fn export_events(&self, org_id: &str) -> Result<Vec<StoredEvent>> {
        let rows = self.client()?.query(
            "SELECT seq, id, repo_id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = $1 ORDER BY seq ASC",
            &[&org_id],
        )?;
        let mut out = Vec::new();
        for r in &rows {
            let id: String = r.get(1);
            let payload_str: String = r.get(7);
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq: r.get(0),
                id,
                repo_id: r.get(2),
                session_id: r.get(3),
                timestamp: r.get(4),
                event_type: r.get(5),
                actor: r.get(6),
                payload,
            });
        }
        Ok(out)
    }

    fn export_audit(&self, org_id: &str) -> Result<Vec<AuditRecord>> {
        let rows = self.client()?.query(
            "SELECT seq, ts, org_id, actor_member_id, action, detail, entry_hash
             FROM audit_log WHERE org_id = $1 ORDER BY seq ASC",
            &[&org_id],
        )?;
        Ok(rows
            .iter()
            .map(|r| AuditRecord {
                seq: r.get(0),
                ts: r.get(1),
                org_id: r.get(2),
                actor_member_id: r.get(3),
                action: r.get(4),
                detail: r.get(5),
                entry_hash: r.get(6),
            })
            .collect())
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
        // Dynamic filter; every clause is a bound parameter (no injection
        // surface). `org_id = $1` keeps it tenant-scoped (NULL-org rows excluded).
        let org = org_id.to_string();
        let lim = limit as i64;
        let mut sql = String::from(
            "SELECT seq, ts, org_id, actor_member_id, action, detail, entry_hash
             FROM audit_log WHERE org_id = $1",
        );
        let mut params: Vec<Box<dyn postgres::types::ToSql + Sync>> = vec![Box::new(org)];
        if let Some(a) = actor {
            params.push(Box::new(a.to_string()));
            sql.push_str(&format!(" AND actor_member_id = ${}", params.len()));
        }
        if let Some(a) = action {
            params.push(Box::new(a.to_string()));
            sql.push_str(&format!(" AND action = ${}", params.len()));
        }
        if let Some(s) = since_rfc3339 {
            params.push(Box::new(s.to_string()));
            sql.push_str(&format!(" AND ts >= ${}", params.len()));
        }
        if let Some(c) = before_seq {
            params.push(Box::new(c));
            sql.push_str(&format!(" AND seq < ${}", params.len()));
        }
        params.push(Box::new(lim));
        sql.push_str(&format!(" ORDER BY seq DESC LIMIT ${}", params.len()));

        let refs: Vec<&(dyn postgres::types::ToSql + Sync)> =
            params.iter().map(|b| b.as_ref()).collect();
        let rows = self.client()?.query(&sql, refs.as_slice())?;
        Ok(rows
            .iter()
            .map(|r| AuditRecord {
                seq: r.get(0),
                ts: r.get(1),
                org_id: r.get(2),
                actor_member_id: r.get(3),
                action: r.get(4),
                detail: r.get(5),
                entry_hash: r.get(6),
            })
            .collect())
    }

    fn append_audit(&self, entry: &AuditEntry) -> Result<()> {
        let ts = chrono::Utc::now().to_rfc3339();
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&advisory_key("audit")],
        )?;
        let (prev, count) = read_head(
            &mut tx,
            "SELECT head_hash, entry_count FROM audit_head WHERE id = $1",
            &1i32,
        )?;
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
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &ts,
                &entry.org_id,
                &entry.actor_member_id,
                &entry.action,
                &entry.detail,
                &prev,
                &entry_hash,
            ],
        )
        .context("failed to append audit entry")?;
        tx.execute(
            "INSERT INTO audit_head (id, head_hash, entry_count) VALUES (1, $1, $2)
             ON CONFLICT (id) DO UPDATE SET head_hash = excluded.head_hash,
                 entry_count = excluded.entry_count",
            &[&entry_hash, &(count + 1)],
        )?;
        tx.commit().context("failed to commit audit entry")?;
        Ok(())
    }

    fn audit_len(&self) -> Result<u64> {
        let n: i64 = self
            .client()?
            .query_one("SELECT COUNT(*) FROM audit_log", &[])?
            .get(0);
        Ok(n as u64)
    }

    fn verify_audit_chain(&self) -> Result<bool> {
        let mut client = self.client()?;
        // Seed from the sealed checkpoint (genesis `("", 0)` when nothing sealed).
        let (sealed_hash, sealed_count): (String, i64) = match client.query_opt(
            "SELECT sealed_hash, sealed_count FROM audit_head WHERE id = 1",
            &[],
        )? {
            Some(r) => (r.get(0), r.get(1)),
            None => (String::new(), 0),
        };
        let rows = client.query(
            "SELECT ts, org_id, actor_member_id, action, detail, prev_hash, entry_hash
             FROM audit_log ORDER BY seq ASC",
            &[],
        )?;
        let mut expected_prev = sealed_hash;
        let mut counted: i64 = sealed_count;
        for r in &rows {
            let prev_hash: String = r.get(5);
            let entry_hash: String = r.get(6);
            if prev_hash != expected_prev {
                return Ok(false);
            }
            let org_id: Option<String> = r.get(1);
            let actor: Option<String> = r.get(2);
            let recomputed = audit_hash(
                &prev_hash,
                &r.get::<_, String>(0),
                org_id.as_deref(),
                actor.as_deref(),
                &r.get::<_, String>(3),
                &r.get::<_, String>(4),
            );
            if recomputed != entry_hash {
                return Ok(false);
            }
            expected_prev = entry_hash;
            counted += 1;
        }
        let head = client.query_opt(
            "SELECT head_hash, entry_count FROM audit_head WHERE id = 1",
            &[],
        )?;
        match head {
            Some(row) => {
                Ok(counted == row.get::<_, i64>(1) && expected_prev == row.get::<_, String>(0))
            }
            None => Ok(counted == 0),
        }
    }

    fn seal_audit_before(&self, cutoff_rfc3339: &str) -> Result<u64> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        // Take the same advisory lock as append_audit so sealing and appends are
        // serialized: otherwise a concurrent append between the entry_count read
        // and the COUNT(seq > boundary) below would skew sealed_count and make
        // verify_audit_chain report a false break.
        tx.execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&advisory_key("audit")],
        )?;

        // Newest entry older than the cutoff becomes the new checkpoint boundary.
        let boundary = tx.query_opt(
            "SELECT seq, entry_hash FROM audit_log WHERE ts < $1 ORDER BY seq DESC LIMIT 1",
            &[&cutoff_rfc3339],
        )?;
        let Some(brow) = boundary else {
            return Ok(0);
        };
        let bseq: i64 = brow.get(0);
        let bhash: String = brow.get(1);

        let entry_count: i64 = tx
            .query_one("SELECT entry_count FROM audit_head WHERE id = 1", &[])?
            .get(0);
        let retained_after: i64 = tx
            .query_one("SELECT COUNT(*) FROM audit_log WHERE seq > $1", &[&bseq])?
            .get(0);
        let sealed_count = entry_count - retained_after;

        let pruned = tx.execute("DELETE FROM audit_log WHERE seq <= $1", &[&bseq])?;
        tx.execute(
            "UPDATE audit_head SET sealed_hash = $1, sealed_count = $2 WHERE id = 1",
            &[&bhash, &sealed_count],
        )?;
        tx.commit()?;
        Ok(pruned)
    }

    fn provision_member(
        &self,
        org_id: &str,
        display_name: &str,
        role: Role,
        email: &str,
    ) -> Result<String> {
        let member_id = ids::generate_id("mbr");
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "INSERT INTO member (id, org_id, display_name, role, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &member_id,
                &org_id,
                &display_name,
                &role.as_str(),
                &chrono::Utc::now().to_rfc3339(),
            ],
        )
        .context("failed to create member (does the org exist?)")?;
        tx.execute(
            "INSERT INTO member_identity (member_id, email) VALUES ($1, $2)",
            &[&member_id, &email],
        )
        .context("failed to set member email (already in use?)")?;
        tx.commit()?;
        Ok(member_id)
    }

    fn find_member_by_email(&self, email: &str) -> Result<Option<Principal>> {
        principal_row(
            &mut self.client()?,
            "SELECT m.id, m.org_id, m.role FROM member m
             JOIN member_identity i ON i.member_id = m.id WHERE i.email = $1 AND m.active = TRUE",
            email,
        )
    }

    fn find_member_by_oidc_subject(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<Principal>> {
        let row = self.client()?.query_opt(
            "SELECT m.id, m.org_id, m.role FROM member m
             JOIN member_identity i ON i.member_id = m.id
             WHERE i.oidc_issuer = $1 AND i.oidc_subject = $2 AND m.active = TRUE",
            &[&issuer, &subject],
        )?;
        match row {
            Some(r) => Ok(Some(Principal {
                member_id: r.get(0),
                org_id: r.get(1),
                role: Role::parse(&r.get::<_, String>(2))?,
            })),
            None => Ok(None),
        }
    }

    fn bind_oidc_subject(&self, member_id: &str, issuer: &str, subject: &str) -> Result<bool> {
        // Only bind when no subject is set yet (see SQLite impl for rationale).
        let n = self.client()?.execute(
            "UPDATE member_identity SET oidc_issuer = $2, oidc_subject = $3
             WHERE member_id = $1 AND oidc_subject IS NULL",
            &[&member_id, &issuer, &subject],
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
        self.client()?.execute(
            "INSERT INTO oidc_login (state, pkce_verifier, nonce, browser_binding, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &state,
                &pkce_verifier,
                &nonce,
                &browser_binding,
                &chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    fn count_logins(&self) -> Result<u64> {
        let n: i64 = self
            .client()?
            .query_one("SELECT COUNT(*) FROM oidc_login", &[])?
            .get(0);
        Ok(n as u64)
    }

    fn prune_expired_logins(&self, ttl_secs: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(ttl_secs)).to_rfc3339();
        let n = self
            .client()?
            .execute("DELETE FROM oidc_login WHERE created_at < $1", &[&cutoff])?;
        Ok(n)
    }

    fn take_login(&self, state: &str) -> Result<Option<LoginTx>> {
        let row = self.client()?.query_opt(
            "DELETE FROM oidc_login WHERE state = $1
             RETURNING pkce_verifier, nonce, browser_binding, created_at",
            &[&state],
        )?;
        Ok(row.map(|r| LoginTx {
            pkce_verifier: r.get(0),
            nonce: r.get(1),
            browser_binding: r.get(2),
            created_at: r.get(3),
        }))
    }

    fn create_session(&self, member_id: &str, ttl_secs: i64) -> Result<String> {
        let id = ids::generate_id("sess");
        let now = chrono::Utc::now();
        let expires = now + chrono::Duration::seconds(ttl_secs);
        self.client()?.execute(
            "INSERT INTO session (id, member_id, created_at, expires_at)
             VALUES ($1, $2, $3, $4)",
            &[&id, &member_id, &now.to_rfc3339(), &expires.to_rfc3339()],
        )?;
        Ok(id)
    }

    fn session_principal(&self, session_id: &str) -> Result<Option<Principal>> {
        let now = chrono::Utc::now().to_rfc3339();
        let row = self.client()?.query_opt(
            "SELECT m.id, m.org_id, m.role FROM session s
             JOIN member m ON m.id = s.member_id
             WHERE s.id = $1 AND s.expires_at > $2 AND m.active = TRUE",
            &[&session_id, &now],
        )?;
        match row {
            Some(r) => Ok(Some(Principal {
                member_id: r.get(0),
                org_id: r.get(1),
                role: Role::parse(&r.get::<_, String>(2))?,
            })),
            None => Ok(None),
        }
    }

    fn delete_session(&self, session_id: &str) -> Result<bool> {
        let n = self
            .client()?
            .execute("DELETE FROM session WHERE id = $1", &[&session_id])?;
        Ok(n > 0)
    }

    fn member_principal(&self, member_id: &str) -> Result<Option<Principal>> {
        let row = self.client()?.query_opt(
            "SELECT id, org_id, role FROM member WHERE id = $1 AND active = TRUE",
            &[&member_id],
        )?;
        match row {
            Some(r) => Ok(Some(Principal {
                member_id: r.get(0),
                org_id: r.get(1),
                role: Role::parse(&r.get::<_, String>(2))?,
            })),
            None => Ok(None),
        }
    }

    fn create_device_auth(&self, device_code: &str, user_code: &str) -> Result<()> {
        self.client()?.execute(
            "INSERT INTO device_auth (device_code, user_code, status, created_at)
             VALUES ($1, $2, 'pending', $3)",
            &[&device_code, &user_code, &chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    fn count_device_auths(&self) -> Result<u64> {
        let n: i64 = self
            .client()?
            .query_one("SELECT COUNT(*) FROM device_auth", &[])?
            .get(0);
        Ok(n as u64)
    }

    fn prune_expired_device_auths(&self, ttl_secs: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(ttl_secs)).to_rfc3339();
        let n = self
            .client()?
            .execute("DELETE FROM device_auth WHERE created_at < $1", &[&cutoff])?;
        Ok(n)
    }

    fn find_device_by_user_code(&self, user_code: &str) -> Result<Option<DeviceAuth>> {
        let row = self.client()?.query_opt(
            "SELECT user_code, status, member_id, created_at
             FROM device_auth WHERE user_code = $1",
            &[&user_code],
        )?;
        Ok(row.map(|r| DeviceAuth {
            user_code: r.get(0),
            status: r.get(1),
            member_id: r.get(2),
            created_at: r.get(3),
        }))
    }

    fn set_device_decision(&self, user_code: &str, member_id: &str, approve: bool) -> Result<bool> {
        let status = if approve { "approved" } else { "denied" };
        let n = self.client()?.execute(
            "UPDATE device_auth SET status = $2, member_id = $3
             WHERE user_code = $1 AND status = 'pending'",
            &[&user_code, &status, &member_id],
        )?;
        Ok(n > 0)
    }

    fn poll_device(&self, device_code: &str, ttl_secs: i64) -> Result<DevicePoll> {
        // A short transaction with an advisory lock serializes the read +
        // terminal delete so an approved token is delivered at most once.
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1))",
            &[&device_code],
        )?;
        let row = tx.query_opt(
            "SELECT status, member_id, created_at FROM device_auth WHERE device_code = $1",
            &[&device_code],
        )?;
        let Some(row) = row else {
            return Ok(DevicePoll::NotFound);
        };
        let status: String = row.get(0);
        let member_id: Option<String> = row.get(1);
        let created_at: String = row.get(2);
        if super::device_expired(&created_at, ttl_secs) {
            tx.execute(
                "DELETE FROM device_auth WHERE device_code = $1",
                &[&device_code],
            )?;
            tx.commit()?;
            return Ok(DevicePoll::NotFound);
        }
        let outcome = match status.as_str() {
            "approved" => DevicePoll::Approved(member_id.unwrap_or_default()),
            "denied" => DevicePoll::Denied,
            _ => DevicePoll::Pending,
        };
        if !matches!(outcome, DevicePoll::Pending) {
            tx.execute(
                "DELETE FROM device_auth WHERE device_code = $1",
                &[&device_code],
            )?;
        }
        tx.commit()?;
        Ok(outcome)
    }

    fn prune_expired_sessions(&self) -> Result<u64> {
        let now = chrono::Utc::now().to_rfc3339();
        let n = self
            .client()?
            .execute("DELETE FROM session WHERE expires_at < $1", &[&now])?;
        Ok(n)
    }

    fn prune_finished_jobs(&self, older_than_rfc3339: &str) -> Result<u64> {
        let n = self.client()?.execute(
            "DELETE FROM job
             WHERE status IN ('completed', 'failed') AND updated_at < $1",
            &[&older_than_rfc3339],
        )?;
        Ok(n)
    }

    fn enqueue_job(&self, org_id: &str, kind: &str, job_params: Option<&str>) -> Result<String> {
        let id = ids::generate_id("job");
        let now = chrono::Utc::now().to_rfc3339();
        self.client()?
            .execute(
                "INSERT INTO job (id, org_id, kind, status, params, created_at, updated_at)
                 VALUES ($1, $2, $3, 'queued', $4, $5, $5)",
                &[&id, &org_id, &kind, &job_params, &now],
            )
            .context("failed to enqueue job")?;
        Ok(id)
    }

    fn claim_next_job(&self) -> Result<Option<Job>> {
        // FOR UPDATE SKIP LOCKED lets multiple workers claim distinct jobs.
        let now = chrono::Utc::now().to_rfc3339();
        let row = self.client()?.query_opt(
            "UPDATE job SET status = 'running', updated_at = $1
             WHERE id = (
                 SELECT id FROM job WHERE status = 'queued'
                 ORDER BY created_at ASC, id ASC
                 FOR UPDATE SKIP LOCKED LIMIT 1
             )
             RETURNING id, org_id, kind, status, result, error, params, created_at, updated_at",
            &[&now],
        )?;
        Ok(row.map(|r| Job {
            id: r.get(0),
            org_id: r.get(1),
            kind: r.get(2),
            status: r.get(3),
            result: r.get(4),
            error: r.get(5),
            params: r.get(6),
            created_at: r.get(7),
            updated_at: r.get(8),
        }))
    }

    fn complete_job(&self, job_id: &str, result_json: &str) -> Result<()> {
        self.client()?.execute(
            "UPDATE job SET status = 'completed', result = $2, updated_at = $3 WHERE id = $1",
            &[&job_id, &result_json, &chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    fn fail_job(&self, job_id: &str, error: &str) -> Result<()> {
        self.client()?.execute(
            "UPDATE job SET status = 'failed', error = $2, updated_at = $3 WHERE id = $1",
            &[&job_id, &error, &chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    fn get_job(&self, org_id: &str, job_id: &str) -> Result<Option<Job>> {
        let row = self.client()?.query_opt(
            "SELECT id, org_id, kind, status, result, error, params, created_at, updated_at
             FROM job WHERE org_id = $1 AND id = $2",
            &[&org_id, &job_id],
        )?;
        Ok(row.map(|r| Job {
            id: r.get(0),
            org_id: r.get(1),
            kind: r.get(2),
            status: r.get(3),
            result: r.get(4),
            error: r.get(5),
            params: r.get(6),
            created_at: r.get(7),
            updated_at: r.get(8),
        }))
    }

    fn list_jobs(&self, org_id: &str, limit: u32) -> Result<Vec<Job>> {
        let lim = limit as i64;
        let rows = self.client()?.query(
            "SELECT id, org_id, kind, status, result, error, params, created_at, updated_at
             FROM job WHERE org_id = $1 ORDER BY created_at DESC, id DESC LIMIT $2",
            &[&org_id, &lim],
        )?;
        Ok(rows
            .iter()
            .map(|r| Job {
                id: r.get(0),
                org_id: r.get(1),
                kind: r.get(2),
                status: r.get(3),
                result: r.get(4),
                error: r.get(5),
                params: r.get(6),
                created_at: r.get(7),
                updated_at: r.get(8),
            })
            .collect())
    }

    fn list_members(&self, org_id: &str) -> Result<Vec<MemberInfo>> {
        let rows = self.client()?.query(
            "SELECT m.id, m.display_name, m.role, m.active, i.email, i.oidc_subject
             FROM member m
             LEFT JOIN member_identity i ON i.member_id = m.id
             WHERE m.org_id = $1
             ORDER BY m.display_name",
            &[&org_id],
        )?;
        Ok(rows
            .iter()
            .map(|r| {
                let oidc_subject: Option<String> = r.get(5);
                MemberInfo {
                    id: r.get(0),
                    display_name: r.get(1),
                    role: r.get(2),
                    active: r.get(3),
                    email: r.get(4),
                    sso_bound: oidc_subject.is_some(),
                }
            })
            .collect())
    }

    fn scim_token_created_at(&self, org_id: &str) -> Result<Option<String>> {
        let row = self.client()?.query_opt(
            "SELECT created_at FROM scim_token WHERE org_id = $1
             ORDER BY created_at DESC LIMIT 1",
            &[&org_id],
        )?;
        Ok(row.map(|r| r.get(0)))
    }

    fn put_compliance_snapshots(&self, org_id: &str, snaps: &[ComplianceSnapshot]) -> Result<()> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        for snap in snaps {
            tx.execute(
                "INSERT INTO compliance_snapshot
                     (id, org_id, repo_id, repo_name, policy_name, policy_version,
                      evaluated_at, ai_ranges, violations, high, medium, low)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                &[
                    &ids::generate_id("cmp"),
                    &org_id,
                    &snap.repo_id,
                    &snap.repo_name,
                    &snap.policy_name,
                    &snap.policy_version,
                    &snap.evaluated_at,
                    &snap.ai_ranges,
                    &snap.violations,
                    &snap.high,
                    &snap.medium,
                    &snap.low,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    fn latest_compliance(&self, org_id: &str) -> Result<Vec<ComplianceSnapshot>> {
        let rows = self.client()?.query(
            "SELECT cs.repo_id, cs.repo_name, cs.policy_name, cs.policy_version,
                    cs.evaluated_at, cs.ai_ranges, cs.violations, cs.high, cs.medium, cs.low
             FROM compliance_snapshot cs
             JOIN (
                 SELECT repo_id, MAX(evaluated_at || '|' || id) AS mk
                 FROM compliance_snapshot WHERE org_id = $1 GROUP BY repo_id
             ) latest
               ON cs.repo_id = latest.repo_id
              AND (cs.evaluated_at || '|' || cs.id) = latest.mk
             WHERE cs.org_id = $1
             ORDER BY cs.evaluated_at DESC, cs.repo_name",
            &[&org_id],
        )?;
        Ok(rows
            .iter()
            .map(|r| ComplianceSnapshot {
                repo_id: r.get(0),
                repo_name: r.get(1),
                policy_name: r.get(2),
                policy_version: r.get(3),
                evaluated_at: r.get(4),
                ai_ranges: r.get(5),
                violations: r.get(6),
                high: r.get(7),
                medium: r.get(8),
                low: r.get(9),
            })
            .collect())
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
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "INSERT INTO scim_group (id, org_id, display_name, external_id, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $5)",
            &[&id, &org_id, &display_name, &external_id, &now],
        )
        .context("failed to create group (duplicate displayName?)")?;
        let members = pg_set_group_members(&mut tx, org_id, &id, members)?;
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
        let mut client = self.client()?;
        let rows = client.query(
            "SELECT id, display_name, external_id FROM scim_group
             WHERE org_id = $1 AND ($2::text IS NULL OR display_name = $2) ORDER BY display_name",
            &[&org_id, &name_filter],
        )?;
        let metas: Vec<(String, String, Option<String>)> = rows
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect();
        let mut out = Vec::new();
        for (id, display_name, external_id) in metas {
            let members = pg_group_member_ids(&mut *client, &id)?;
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
        let mut client = self.client()?;
        let row = client.query_opt(
            "SELECT display_name, external_id FROM scim_group WHERE org_id = $1 AND id = $2",
            &[&org_id, &group_id],
        )?;
        match row {
            Some(r) => {
                let display_name: String = r.get(0);
                let external_id: Option<String> = r.get(1);
                let members = pg_group_member_ids(&mut *client, group_id)?;
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
            let mut client = self.client()?;
            let mut tx = client.transaction()?;
            let exists = tx
                .query_opt(
                    "SELECT 1 FROM scim_group WHERE org_id = $1 AND id = $2",
                    &[&org_id, &group_id],
                )?
                .is_some();
            if !exists {
                return Ok(None);
            }
            if let Some(name) = display_name {
                tx.execute(
                    "UPDATE scim_group SET display_name = $2 WHERE id = $1",
                    &[&group_id, &name],
                )?;
            }
            if let Some(ext) = external_id {
                tx.execute(
                    "UPDATE scim_group SET external_id = $2 WHERE id = $1",
                    &[&group_id, &ext],
                )?;
            }
            if let Some(new_members) = members {
                let old = pg_group_member_ids(&mut tx, group_id)?;
                tx.execute(
                    "DELETE FROM scim_group_member WHERE group_id = $1",
                    &[&group_id],
                )?;
                pg_set_group_members(&mut tx, org_id, group_id, new_members)?;
                let mut affected: std::collections::BTreeSet<String> = old.into_iter().collect();
                affected.extend(new_members.iter().cloned());
                for m in affected {
                    pg_recompute_member_role(&mut tx, &m)?;
                }
            } else if display_name.is_some() {
                for m in pg_group_member_ids(&mut tx, group_id)? {
                    pg_recompute_member_role(&mut tx, &m)?;
                }
            }
            tx.execute(
                "UPDATE scim_group SET updated_at = $2 WHERE id = $1",
                &[&group_id, &chrono::Utc::now().to_rfc3339()],
            )?;
            tx.commit()?;
        }
        self.scim_get_group(org_id, group_id)
    }

    fn scim_delete_group(&self, org_id: &str, group_id: &str) -> Result<bool> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        let exists = tx
            .query_opt(
                "SELECT 1 FROM scim_group WHERE org_id = $1 AND id = $2",
                &[&org_id, &group_id],
            )?
            .is_some();
        if !exists {
            return Ok(false);
        }
        let members = pg_group_member_ids(&mut tx, group_id)?;
        tx.execute(
            "DELETE FROM scim_group_member WHERE group_id = $1",
            &[&group_id],
        )?;
        tx.execute("DELETE FROM scim_group WHERE id = $1", &[&group_id])?;
        for m in members {
            pg_recompute_member_role(&mut tx, &m)?;
        }
        tx.commit()?;
        Ok(true)
    }

    fn requeue_running_jobs(&self) -> Result<u64> {
        let n = self.client()?.execute(
            "UPDATE job SET status = 'queued', updated_at = $1 WHERE status = 'running'",
            &[&chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(n)
    }

    fn create_scim_token(&self, org_id: &str) -> Result<GeneratedToken> {
        let token = auth::generate_token()?;
        self.client()?
            .execute(
                "INSERT INTO scim_token (token_id, org_id, secret_hash, created_at)
                 VALUES ($1, $2, $3, $4)",
                &[
                    &token.token_id,
                    &org_id,
                    &token.secret_hash,
                    &chrono::Utc::now().to_rfc3339(),
                ],
            )
            .context("failed to create SCIM token (does the org exist?)")?;
        Ok(token)
    }

    fn authenticate_scim(&self, token: &str) -> Result<Option<String>> {
        let Some((token_id, secret)) = auth::parse_token(token) else {
            return Ok(None);
        };
        let row = self.client()?.query_opt(
            "SELECT secret_hash, org_id FROM scim_token WHERE token_id = $1",
            &[&token_id],
        )?;
        let Some(row) = row else { return Ok(None) };
        let secret_hash: String = row.get(0);
        if !auth::verify_secret(&secret, &secret_hash) {
            return Ok(None);
        }
        Ok(Some(row.get(1)))
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
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "INSERT INTO member (id, org_id, display_name, role, created_at, active)
             VALUES ($1, $2, $3, $4, $5, TRUE)",
            &[
                &member_id,
                &org_id,
                &display_name,
                &role.as_str(),
                &chrono::Utc::now().to_rfc3339(),
            ],
        )
        .context("failed to create member")?;
        tx.execute(
            "INSERT INTO member_identity (member_id, email, external_id) VALUES ($1, $2, $3)",
            &[&member_id, &email, &external_id],
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
        let rows = self.client()?.query(
            "SELECT m.id, m.display_name, m.role, m.active, i.email, i.external_id
             FROM member m JOIN member_identity i ON i.member_id = m.id
             WHERE m.org_id = $1 AND ($2::text IS NULL OR i.email = $2)
             ORDER BY i.email",
            &[&org_id, &email_filter],
        )?;
        let mut out = Vec::new();
        for r in &rows {
            out.push(ScimUser {
                member_id: r.get(0),
                display_name: r.get(1),
                role: Role::parse(&r.get::<_, String>(2))?,
                active: r.get(3),
                email: r.get(4),
                external_id: r.get(5),
            });
        }
        Ok(out)
    }

    fn scim_get_user(&self, org_id: &str, member_id: &str) -> Result<Option<ScimUser>> {
        let row = self.client()?.query_opt(
            "SELECT m.display_name, m.role, m.active, i.email, i.external_id
             FROM member m JOIN member_identity i ON i.member_id = m.id
             WHERE m.org_id = $1 AND m.id = $2",
            &[&org_id, &member_id],
        )?;
        match row {
            Some(r) => Ok(Some(ScimUser {
                member_id: member_id.to_string(),
                display_name: r.get(0),
                role: Role::parse(&r.get::<_, String>(1))?,
                active: r.get(2),
                email: r.get(3),
                external_id: r.get(4),
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
            let mut client = self.client()?;
            let exists = client
                .query_opt(
                    "SELECT 1 FROM member WHERE id = $1 AND org_id = $2",
                    &[&member_id, &org_id],
                )?
                .is_some();
            if !exists {
                return Ok(None);
            }
            if let Some(addr) = email {
                client.execute(
                    "UPDATE member_identity SET email = $2 WHERE member_id = $1",
                    &[&member_id, &addr],
                )?;
            }
            if let Some(name) = display_name {
                client.execute(
                    "UPDATE member SET display_name = $2 WHERE id = $1",
                    &[&member_id, &name],
                )?;
            }
            if let Some(r) = role {
                client.execute(
                    "UPDATE member SET role = $2 WHERE id = $1",
                    &[&member_id, &r.as_str()],
                )?;
            }
            if let Some(a) = active {
                client.execute(
                    "UPDATE member SET active = $2 WHERE id = $1",
                    &[&member_id, &a],
                )?;
            }
            if let Some(ext) = external_id {
                client.execute(
                    "UPDATE member_identity SET external_id = $2 WHERE member_id = $1",
                    &[&member_id, &ext],
                )?;
            }
        }
        self.scim_get_user(org_id, member_id)
    }
}

/// Split a `string_agg` CSV (or `None`) into a de-duped, sorted, non-empty list.
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

/// Read the member ids belonging to a group.
fn pg_group_member_ids(c: &mut impl GenericClient, group_id: &str) -> Result<Vec<String>> {
    let rows = c.query(
        "SELECT member_id FROM scim_group_member WHERE group_id = $1 ORDER BY member_id",
        &[&group_id],
    )?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// Replace a group's membership with `members` (each must belong to `org_id`),
/// then recompute each member's role. Returns the members that were set.
fn pg_set_group_members(
    c: &mut impl GenericClient,
    org_id: &str,
    group_id: &str,
    members: &[String],
) -> Result<Vec<String>> {
    let mut set = Vec::new();
    for m in members {
        let in_org = c
            .query_opt(
                "SELECT 1 FROM member WHERE id = $1 AND org_id = $2",
                &[&m, &org_id],
            )?
            .is_some();
        if !in_org {
            bail!("member {m} not found in org {org_id}");
        }
        c.execute(
            "INSERT INTO scim_group_member (group_id, member_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
            &[&group_id, &m],
        )?;
        set.push(m.clone());
    }
    for m in &set {
        pg_recompute_member_role(c, m)?;
    }
    Ok(set)
}

/// Recompute a member's org role from the role-mapping groups they belong to.
/// Membership owns the role: highest mapped group role, or the `viewer` baseline
/// when no role-mapping group remains (so removal revokes elevated access).
fn pg_recompute_member_role(c: &mut impl GenericClient, member_id: &str) -> Result<()> {
    let rows = c.query(
        "SELECT g.display_name FROM scim_group_member gm
         JOIN scim_group g ON g.id = gm.group_id WHERE gm.member_id = $1",
        &[&member_id],
    )?;
    let mut role = Role::Viewer;
    for r in &rows {
        if let Some(found) = role_from_group_name(&r.get::<_, String>(0)) {
            role = role.max(found);
        }
    }
    c.execute(
        "UPDATE member SET role = $2 WHERE id = $1",
        &[&member_id, &role.as_str()],
    )?;
    Ok(())
}

/// Helper: read at most one `(member_id, org_id, role)` row into a [`Principal`].
fn principal_row(client: &mut PooledClient, sql: &str, key: &str) -> Result<Option<Principal>> {
    let row = client.query_opt(sql, &[&key])?;
    match row {
        Some(r) => Ok(Some(Principal {
            member_id: r.get(0),
            org_id: r.get(1),
            role: Role::parse(&r.get::<_, String>(2))?,
        })),
        None => Ok(None),
    }
}

/// Audit entry hash (identical scheme to the SQLite backend).
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

/// Count events grouped by an internal column, scoped to an org.
fn group_counts(
    client: &mut PooledClient,
    column: &str,
    org_id: &str,
) -> Result<std::collections::BTreeMap<String, u64>> {
    assert!(
        matches!(column, "event_type" | "actor"),
        "group_counts column must be allow-listed"
    );
    let sql = format!("SELECT {column}, COUNT(*) FROM event WHERE org_id = $1 GROUP BY {column}");
    let rows = client.query(&sql, &[&org_id])?;
    Ok(rows
        .iter()
        .map(|r| (r.get::<_, String>(0), r.get::<_, i64>(1) as u64))
        .collect())
}
