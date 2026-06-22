//! SQLite implementation of [`Store`] — the default single-node backend.
//!
//! The `Store` trait impl in this module is a thin set of delegators: each
//! method locks the connection-bearing struct and forwards to an inherent
//! method grouped by domain in a submodule (`orgs`, `repos`, `events`, …). The
//! domain submodules hold the actual SQL; shared query helpers and the schema
//! migration live here and in [`schema`].

pub(crate) use std::collections::BTreeMap;
pub(crate) use std::path::Path;
pub(crate) use std::sync::Mutex;

pub(crate) use anyhow::{Context, Result, bail};
pub(crate) use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
pub(crate) use tellur_core::schema::ids;
pub(crate) use tellur_core::schema::types::FileAttribution;

pub(crate) use super::chain;
pub(crate) use super::device_expired;
pub(crate) use super::{
    ActivityBucket, ActivityGroup, AuditEntry, AuditRecord, ComplianceSnapshot, DeviceAuth,
    DevicePoll, GithubInstallation, IngestEvent, Job, LoginTx, MemberInfo, Org, OrgReport,
    PolicyDoc, PolicySummary, Repo, RepoFacts, RepoRoleGrant, RepoSource, RepoSummary, ScimGroup,
    ScimUser, SessionSummary, Store, StoredEvent, role_from_group_name,
};
pub(crate) use crate::auth::{self, GeneratedToken, Principal, Role};

mod schema;
mod orgs;
mod repos;
mod events;
mod policy;
mod audit;
mod auth_sessions;
mod jobs;
mod scim;

/// Current schema version. Bumped as migrations are added in later phases.
pub(crate) const SCHEMA_VERSION: &str = "20";

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
pub(crate) fn split_csv(s: Option<String>) -> Vec<String> {
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
pub(crate) fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
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
pub(crate) fn audit_checkpoint(conn: &Connection) -> Result<(String, i64)> {
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
pub(crate) fn audit_hash(
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
pub(crate) fn group_counts(conn: &Connection, column: &str, org_id: &str) -> Result<BTreeMap<String, u64>> {
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
        self.migrate()
    }

    fn health_check(&self) -> Result<()> {
        self.health_check()
    }

    fn create_org(&self, name: &str) -> Result<Org> {
        self.create_org(name)
    }

    fn create_member(&self, org_id: &str, display_name: &str, role: Role) -> Result<String> {
        self.create_member(org_id, display_name, role)
    }

    fn create_token(&self, member_id: &str) -> Result<GeneratedToken> {
        self.create_token(member_id)
    }

    fn authenticate(&self, token: &str) -> Result<Option<Principal>> {
        self.authenticate(token)
    }

    fn ensure_repo(&self, org_id: &str, name: &str) -> Result<Repo> {
        self.ensure_repo(org_id, name)
    }

    fn find_repo(&self, org_id: &str, repo: &str) -> Result<Option<Repo>> {
        self.find_repo(org_id, repo)
    }

    fn get_repo_source(&self, org_id: &str, repo_id: &str) -> Result<RepoSource> {
        self.get_repo_source(org_id, repo_id)
    }

    fn set_repo_source( &self, org_id: &str, repo_id: &str, link: Option<&str>, raw: Option<&str>, token: Option<&str>, ) -> Result<()> {
        self.set_repo_source(org_id, repo_id, link, raw, token)
    }

    fn set_github_installation( &self, org_id: &str, installation_id: i64, account_login: &str, ) -> Result<()> {
        self.set_github_installation(org_id, installation_id, account_login)
    }

    fn github_installation(&self, installation_id: i64) -> Result<Option<GithubInstallation>> {
        self.github_installation(installation_id)
    }

    fn mark_github_note_harvested( &self, org_id: &str, repo_id: &str, commit_sha: &str, note_sha: &str, ) -> Result<bool> {
        self.mark_github_note_harvested(org_id, repo_id, commit_sha, note_sha)
    }

    fn set_repo_role( &self, org_id: &str, repo_id: &str, member_id: &str, role: Role, ) -> Result<()> {
        self.set_repo_role(org_id, repo_id, member_id, role)
    }

    fn remove_repo_role(&self, org_id: &str, repo_id: &str, member_id: &str) -> Result<bool> {
        self.remove_repo_role(org_id, repo_id, member_id)
    }

    fn get_repo_role(&self, org_id: &str, repo_id: &str, member_id: &str) -> Result<Option<Role>> {
        self.get_repo_role(org_id, repo_id, member_id)
    }

    fn list_repo_roles(&self, org_id: &str, repo_id: &str) -> Result<Vec<RepoRoleGrant>> {
        self.list_repo_roles(org_id, repo_id)
    }

    fn append_events( &self, org_id: &str, repo_id: &str, events: &[IngestEvent], ) -> Result<Vec<String>> {
        self.append_events(org_id, repo_id, events)
    }

    fn event_count(&self, org_id: &str, repo_id: &str) -> Result<u64> {
        self.event_count(org_id, repo_id)
    }

    fn verify_event_chain(&self, org_id: &str, repo_id: &str) -> Result<bool> {
        self.verify_event_chain(org_id, repo_id)
    }

    fn put_attributions( &self, org_id: &str, repo_id: &str, files: &[FileAttribution], ) -> Result<usize> {
        self.put_attributions(org_id, repo_id, files)
    }

    fn list_attributions(&self, org_id: &str, repo_id: &str) -> Result<Vec<FileAttribution>> {
        self.list_attributions(org_id, repo_id)
    }

    fn list_repos(&self, org_id: &str) -> Result<Vec<RepoSummary>> {
        self.list_repos(org_id)
    }

    fn list_events( &self, org_id: &str, repo_id: &str, limit: u32, before_seq: Option<i64>, ) -> Result<Vec<StoredEvent>> {
        self.list_events(org_id, repo_id, limit, before_seq)
    }

    fn org_report(&self, org_id: &str) -> Result<OrgReport> {
        self.org_report(org_id)
    }

    fn recent_org_events(&self, org_id: &str, limit: u32) -> Result<Vec<StoredEvent>> {
        self.recent_org_events(org_id, limit)
    }

    fn activity_by_day( &self, org_id: &str, since_rfc3339: &str, group: ActivityGroup, ) -> Result<Vec<ActivityBucket>> {
        self.activity_by_day(org_id, since_rfc3339, group)
    }

    fn repo_facts(&self, org_id: &str, repo_id: &str) -> Result<RepoFacts> {
        self.repo_facts(org_id, repo_id)
    }

    fn list_sessions( &self, org_id: &str, repo_id: Option<&str>, actor: Option<&str>, since_rfc3339: Option<&str>, limit: u32, ) -> Result<Vec<SessionSummary>> {
        self.list_sessions(org_id, repo_id, actor, since_rfc3339, limit)
    }

    fn session_events( &self, org_id: &str, session_id: &str, limit: u32, ) -> Result<Vec<StoredEvent>> {
        self.session_events(org_id, session_id, limit)
    }

    fn put_policy(&self, org_id: &str, name: &str, content: &str) -> Result<i64> {
        self.put_policy(org_id, name, content)
    }

    fn list_policies(&self, org_id: &str) -> Result<Vec<PolicySummary>> {
        self.list_policies(org_id)
    }

    fn get_policy(&self, org_id: &str, name: &str) -> Result<Option<PolicyDoc>> {
        self.get_policy(org_id, name)
    }

    fn export_events(&self, org_id: &str) -> Result<Vec<StoredEvent>> {
        self.export_events(org_id)
    }

    fn export_audit(&self, org_id: &str) -> Result<Vec<AuditRecord>> {
        self.export_audit(org_id)
    }

    fn list_audit( &self, org_id: &str, actor: Option<&str>, action: Option<&str>, since_rfc3339: Option<&str>, before_seq: Option<i64>, limit: u32, ) -> Result<Vec<AuditRecord>> {
        self.list_audit(org_id, actor, action, since_rfc3339, before_seq, limit)
    }

    fn append_audit(&self, entry: &AuditEntry) -> Result<()> {
        self.append_audit(entry)
    }

    fn audit_len(&self) -> Result<u64> {
        self.audit_len()
    }

    fn verify_audit_chain(&self) -> Result<bool> {
        self.verify_audit_chain()
    }

    fn seal_audit_before(&self, cutoff_rfc3339: &str) -> Result<u64> {
        self.seal_audit_before(cutoff_rfc3339)
    }

    fn provision_member( &self, org_id: &str, display_name: &str, role: Role, email: &str, ) -> Result<String> {
        self.provision_member(org_id, display_name, role, email)
    }

    fn find_member_by_email(&self, email: &str) -> Result<Option<Principal>> {
        self.find_member_by_email(email)
    }

    fn find_member_by_oidc_subject( &self, issuer: &str, subject: &str, ) -> Result<Option<Principal>> {
        self.find_member_by_oidc_subject(issuer, subject)
    }

    fn bind_oidc_subject(&self, member_id: &str, issuer: &str, subject: &str) -> Result<bool> {
        self.bind_oidc_subject(member_id, issuer, subject)
    }

    fn put_login( &self, state: &str, pkce_verifier: &str, nonce: &str, browser_binding: &str, ) -> Result<()> {
        self.put_login(state, pkce_verifier, nonce, browser_binding)
    }

    fn count_logins(&self) -> Result<u64> {
        self.count_logins()
    }

    fn prune_expired_logins(&self, ttl_secs: i64) -> Result<u64> {
        self.prune_expired_logins(ttl_secs)
    }

    fn take_login(&self, state: &str) -> Result<Option<LoginTx>> {
        self.take_login(state)
    }

    fn create_session(&self, member_id: &str, ttl_secs: i64) -> Result<String> {
        self.create_session(member_id, ttl_secs)
    }

    fn session_principal(&self, session_id: &str) -> Result<Option<Principal>> {
        self.session_principal(session_id)
    }

    fn delete_session(&self, session_id: &str) -> Result<bool> {
        self.delete_session(session_id)
    }

    fn member_principal(&self, member_id: &str) -> Result<Option<Principal>> {
        self.member_principal(member_id)
    }

    fn create_device_auth(&self, device_code: &str, user_code: &str) -> Result<()> {
        self.create_device_auth(device_code, user_code)
    }

    fn count_device_auths(&self) -> Result<u64> {
        self.count_device_auths()
    }

    fn prune_expired_device_auths(&self, ttl_secs: i64) -> Result<u64> {
        self.prune_expired_device_auths(ttl_secs)
    }

    fn find_device_by_user_code(&self, user_code: &str) -> Result<Option<DeviceAuth>> {
        self.find_device_by_user_code(user_code)
    }

    fn set_device_decision(&self, user_code: &str, member_id: &str, approve: bool) -> Result<bool> {
        self.set_device_decision(user_code, member_id, approve)
    }

    fn poll_device(&self, device_code: &str, ttl_secs: i64) -> Result<DevicePoll> {
        self.poll_device(device_code, ttl_secs)
    }

    fn prune_expired_sessions(&self) -> Result<u64> {
        self.prune_expired_sessions()
    }

    fn prune_finished_jobs(&self, older_than_rfc3339: &str) -> Result<u64> {
        self.prune_finished_jobs(older_than_rfc3339)
    }

    fn enqueue_job(&self, org_id: &str, kind: &str, job_params: Option<&str>) -> Result<String> {
        self.enqueue_job(org_id, kind, job_params)
    }

    fn claim_next_job(&self) -> Result<Option<Job>> {
        self.claim_next_job()
    }

    fn complete_job(&self, job_id: &str, result_json: &str) -> Result<()> {
        self.complete_job(job_id, result_json)
    }

    fn fail_job(&self, job_id: &str, error: &str) -> Result<()> {
        self.fail_job(job_id, error)
    }

    fn get_job(&self, org_id: &str, job_id: &str) -> Result<Option<Job>> {
        self.get_job(org_id, job_id)
    }

    fn list_jobs(&self, org_id: &str, limit: u32) -> Result<Vec<Job>> {
        self.list_jobs(org_id, limit)
    }

    fn list_members(&self, org_id: &str) -> Result<Vec<MemberInfo>> {
        self.list_members(org_id)
    }

    fn scim_token_created_at(&self, org_id: &str) -> Result<Option<String>> {
        self.scim_token_created_at(org_id)
    }

    fn put_compliance_snapshots(&self, org_id: &str, snaps: &[ComplianceSnapshot]) -> Result<()> {
        self.put_compliance_snapshots(org_id, snaps)
    }

    fn latest_compliance(&self, org_id: &str) -> Result<Vec<ComplianceSnapshot>> {
        self.latest_compliance(org_id)
    }

    fn scim_create_group( &self, org_id: &str, display_name: &str, external_id: Option<&str>, members: &[String], ) -> Result<ScimGroup> {
        self.scim_create_group(org_id, display_name, external_id, members)
    }

    fn scim_list_groups(&self, org_id: &str, name_filter: Option<&str>) -> Result<Vec<ScimGroup>> {
        self.scim_list_groups(org_id, name_filter)
    }

    fn scim_get_group(&self, org_id: &str, group_id: &str) -> Result<Option<ScimGroup>> {
        self.scim_get_group(org_id, group_id)
    }

    fn scim_update_group( &self, org_id: &str, group_id: &str, display_name: Option<&str>, external_id: Option<&str>, members: Option<&[String]>, ) -> Result<Option<ScimGroup>> {
        self.scim_update_group(org_id, group_id, display_name, external_id, members)
    }

    fn scim_delete_group(&self, org_id: &str, group_id: &str) -> Result<bool> {
        self.scim_delete_group(org_id, group_id)
    }

    fn requeue_running_jobs(&self) -> Result<u64> {
        self.requeue_running_jobs()
    }

    fn create_scim_token(&self, org_id: &str) -> Result<GeneratedToken> {
        self.create_scim_token(org_id)
    }

    fn authenticate_scim(&self, token: &str) -> Result<Option<String>> {
        self.authenticate_scim(token)
    }

    fn scim_create_user( &self, org_id: &str, email: &str, display_name: &str, role: Role, external_id: Option<&str>, ) -> Result<ScimUser> {
        self.scim_create_user(org_id, email, display_name, role, external_id)
    }

    fn scim_list_users(&self, org_id: &str, email_filter: Option<&str>) -> Result<Vec<ScimUser>> {
        self.scim_list_users(org_id, email_filter)
    }

    fn scim_get_user(&self, org_id: &str, member_id: &str) -> Result<Option<ScimUser>> {
        self.scim_get_user(org_id, member_id)
    }

    fn scim_update_user( &self, org_id: &str, member_id: &str, email: Option<&str>, display_name: Option<&str>, role: Option<Role>, active: Option<bool>, external_id: Option<&str>, ) -> Result<Option<ScimUser>> {
        self.scim_update_user(org_id, member_id, email, display_name, role, active, external_id)
    }
}


/// Read the member ids belonging to a group.
pub(crate) fn group_member_ids(conn: &Connection, group_id: &str) -> Result<Vec<String>> {
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
pub(crate) fn set_group_members(
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
pub(crate) fn recompute_member_role(conn: &Connection, member_id: &str) -> Result<()> {
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
pub(crate) fn principal_row(conn: &Connection, sql: &str, key: &str) -> Result<Option<Principal>> {
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
