//! Postgres implementation of [`Store`] — the horizontally-scalable backend.
//!
//! Mirrors `SqliteStore` semantics exactly (same hash chains, tenant scoping,
//! and tamper-evidence) over a connection pool. Chain appends take a per-scope
//! `pg_advisory_xact_lock` so the read-head + insert + head-update are atomic
//! across pooled connections (the Postgres equivalent of SQLite's
//! `BEGIN IMMEDIATE`). Uses `NoTls`: run behind a TLS-terminating proxy.
//!
//! As with the SQLite backend, the `Store` impl is a thin set of delegators to
//! inherent methods grouped by domain in submodules; this file keeps the pool,
//! the advisory-lock + chain helpers, and the schema migration.

pub(crate) use anyhow::{Context, Result, bail};
pub(crate) use r2d2_postgres::PostgresConnectionManager;
pub(crate) use r2d2_postgres::postgres::GenericClient;
pub(crate) use r2d2_postgres::postgres::NoTls;
pub(crate) use r2d2_postgres::postgres::types::ToSql;
pub(crate) use tellur_core::schema::ids;
pub(crate) use tellur_core::schema::types::FileAttribution;

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

pub(crate) type Pool = r2d2::Pool<PostgresConnectionManager<NoTls>>;
pub(crate) type PooledClient = r2d2::PooledConnection<PostgresConnectionManager<NoTls>>;

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
pub(crate) fn advisory_key(scope: &str) -> i64 {
    let hex = ids::hash_content(scope);
    let bytes = u64::from_str_radix(&hex[..16], 16).expect("hash_content returns hex");
    bytes as i64
}

/// Read a chain head (tip hash + length) within a transaction, or genesis.
pub(crate) fn read_head(
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


/// Split a `string_agg` CSV (or `None`) into a de-duped, sorted, non-empty list.
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

/// Read the member ids belonging to a group.
pub(crate) fn pg_group_member_ids(c: &mut impl GenericClient, group_id: &str) -> Result<Vec<String>> {
    let rows = c.query(
        "SELECT member_id FROM scim_group_member WHERE group_id = $1 ORDER BY member_id",
        &[&group_id],
    )?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// Replace a group's membership with `members` (each must belong to `org_id`),
/// then recompute each member's role. Returns the members that were set.
pub(crate) fn pg_set_group_members(
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
pub(crate) fn pg_recompute_member_role(c: &mut impl GenericClient, member_id: &str) -> Result<()> {
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
pub(crate) fn principal_row(client: &mut PooledClient, sql: &str, key: &str) -> Result<Option<Principal>> {
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

/// Count events grouped by an internal column, scoped to an org.
pub(crate) fn group_counts(
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

