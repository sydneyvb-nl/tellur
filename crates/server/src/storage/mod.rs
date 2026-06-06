//! Storage abstraction.
//!
//! A single `Store` trait decouples handlers/services from the backend. The
//! default `SqliteStore` is zero-config for single-node self-hosting; a Postgres
//! backend (B5) will implement the same trait for horizontal scale.
//!
//! Tenant isolation is enforced here: identity/data rows carry an `org_id` and
//! lookups resolve a caller to a tenant-scoped [`Principal`], so handlers cannot
//! accidentally cross org boundaries.

use std::collections::BTreeMap;

use anyhow::Result;

use crate::auth::{GeneratedToken, Principal, Role};

mod chain;
pub mod postgres;
pub mod sqlite;
pub use postgres::PostgresStore;
pub use sqlite::SqliteStore;

/// An organization (tenant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Org {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

/// A repository within an org.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub id: String,
    pub org_id: String,
    pub name: String,
}

/// A provenance event to ingest. The hub assigns the id and (re)computes the
/// hash chain, so client-supplied hashes are never trusted.
#[derive(Debug, Clone)]
pub struct IngestEvent {
    pub session_id: String,
    pub timestamp: String,
    pub event_type: String,
    pub actor: String,
    pub payload: serde_json::Value,
}

/// A repo plus its event count (for listings).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RepoSummary {
    pub id: String,
    pub name: String,
    pub event_count: u64,
}

/// A stored provenance event (read model).
#[derive(Debug, Clone, serde::Serialize)]
pub struct StoredEvent {
    pub seq: i64,
    pub id: String,
    pub repo_id: String,
    pub session_id: String,
    pub timestamp: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub actor: String,
    pub payload: serde_json::Value,
}

/// Org-level activity rollup aggregated across the org's repos.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OrgReport {
    pub org_id: String,
    pub total_events: u64,
    pub distinct_sessions: u64,
    pub by_type: BTreeMap<String, u64>,
    pub by_actor: BTreeMap<String, u64>,
    pub repos: Vec<RepoSummary>,
}

/// A stored org policy document.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PolicyDoc {
    pub name: String,
    pub content: String,
    pub version: i64,
    pub updated_at: String,
}

/// Policy metadata without the body (for listings).
#[derive(Debug, Clone, serde::Serialize)]
pub struct PolicySummary {
    pub name: String,
    pub version: i64,
    pub updated_at: String,
}

/// A pending OIDC login transaction (CSRF `state` → PKCE/nonce binding).
#[derive(Debug, Clone)]
pub struct LoginTx {
    pub pkce_verifier: String,
    pub nonce: String,
    /// Secret tied to the initiating browser (matched against a login cookie on
    /// callback to prevent login-CSRF / session fixation).
    pub browser_binding: String,
    pub created_at: String,
}

/// A durable background job (e.g. a large org export). Persisted so it survives
/// restarts; a worker claims `queued` jobs, runs them, and stores the result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Job {
    pub id: String,
    pub org_id: String,
    pub kind: String,
    /// `queued` | `running` | `completed` | `failed`.
    pub status: String,
    /// JSON result text (present when `completed`).
    #[serde(skip)]
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A SCIM-managed group. Membership drives org roles via [`role_from_group_name`].
#[derive(Debug, Clone)]
pub struct ScimGroup {
    pub id: String,
    pub org_id: String,
    pub display_name: String,
    pub external_id: Option<String>,
    /// Member ids belonging to the group.
    pub members: Vec<String>,
}

/// Map a SCIM group's `displayName` to an org role by convention:
/// `tellur-admin` / `tellur-contributor` / `tellur-viewer` (case-insensitive).
/// Any other name grants no role (the group is informational only).
pub fn role_from_group_name(display_name: &str) -> Option<Role> {
    match display_name.to_ascii_lowercase().as_str() {
        "tellur-admin" => Some(Role::Admin),
        "tellur-contributor" => Some(Role::Contributor),
        "tellur-viewer" => Some(Role::Viewer),
        _ => None,
    }
}

/// A SCIM-managed user (read model mapping a member + its SSO identity).
#[derive(Debug, Clone)]
pub struct ScimUser {
    pub member_id: String,
    pub email: String,
    pub display_name: String,
    pub role: Role,
    pub active: bool,
    pub external_id: Option<String>,
}

/// A per-repo role grant: a member's elevated role on a specific repo.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RepoRoleGrant {
    pub member_id: String,
    pub role: String,
    pub updated_at: String,
}

/// An audit-log record (read model for the export portal).
#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditRecord {
    pub seq: i64,
    pub ts: String,
    pub org_id: Option<String>,
    pub actor_member_id: Option<String>,
    pub action: String,
    pub detail: String,
    pub entry_hash: String,
}

/// A row to append to the tamper-evident audit log.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub org_id: Option<String>,
    pub actor_member_id: Option<String>,
    pub action: String,
    pub detail: String,
}

/// Backend-agnostic storage interface.
pub trait Store: Send + Sync {
    /// Apply schema migrations. Must be idempotent.
    fn migrate(&self) -> Result<()>;

    /// Cheap connectivity check used by the readiness probe.
    fn health_check(&self) -> Result<()>;

    // ─── Identity & tenancy ─────────────────────────────────────────────────

    /// Create a new organization (tenant).
    fn create_org(&self, name: &str) -> Result<Org>;

    /// Create a member in an org with a role; returns the member id.
    fn create_member(&self, org_id: &str, display_name: &str, role: Role) -> Result<String>;

    /// Mint an API token for a member. The plaintext is returned exactly once.
    fn create_token(&self, member_id: &str) -> Result<GeneratedToken>;

    /// Resolve a bearer token to a tenant-scoped principal, or `None` if the
    /// token is malformed/unknown/invalid.
    fn authenticate(&self, token: &str) -> Result<Option<Principal>>;

    // ─── Repos & provenance events (tenant-scoped) ──────────────────────────

    /// Get-or-create a repo by `(org_id, name)`; returns its id.
    fn ensure_repo(&self, org_id: &str, name: &str) -> Result<Repo>;

    /// Look up a repo by `(org_id, repo ref)` without creating it. The ref may
    /// be the stable repo id or the human-readable repo name.
    fn find_repo(&self, org_id: &str, repo: &str) -> Result<Option<Repo>>;

    // ─── Fine-grained per-repo RBAC (additive grants) ────────────────────────

    /// Grant (or update) a member's per-repo role. Both the repo and the member
    /// must belong to `org_id`. Grants are **additive**: a member's effective
    /// role on a repo is `max(org_role, repo_grant)`.
    fn set_repo_role(&self, org_id: &str, repo_id: &str, member_id: &str, role: Role)
    -> Result<()>;

    /// Remove a member's per-repo grant. Returns `true` if a grant existed.
    fn remove_repo_role(&self, org_id: &str, repo_id: &str, member_id: &str) -> Result<bool>;

    /// The member's per-repo grant for a repo, if any (tenant-scoped).
    fn get_repo_role(&self, org_id: &str, repo_id: &str, member_id: &str) -> Result<Option<Role>>;

    /// List all per-repo grants for a repo (tenant-scoped).
    fn list_repo_roles(&self, org_id: &str, repo_id: &str) -> Result<Vec<RepoRoleGrant>>;

    /// Append events to a repo's chain. The hub assigns ids and recomputes the
    /// per-repo hash chain (clients cannot forge provenance). Returns new ids.
    fn append_events(
        &self,
        org_id: &str,
        repo_id: &str,
        events: &[IngestEvent],
    ) -> Result<Vec<String>>;

    /// Count events in a repo (tenant-scoped).
    fn event_count(&self, org_id: &str, repo_id: &str) -> Result<u64>;

    /// Recompute a repo's event hash chain and report whether it is intact.
    fn verify_event_chain(&self, org_id: &str, repo_id: &str) -> Result<bool>;

    // ─── Attribution (line-level origin data; powers SLSA/SPDX export) ────────

    /// Upsert per-file attribution for a repo; returns the number of files
    /// written.
    fn put_attributions(
        &self,
        org_id: &str,
        repo_id: &str,
        files: &[tellur_core::schema::types::FileAttribution],
    ) -> Result<usize>;

    /// All stored file attributions for a repo (tenant-scoped).
    fn list_attributions(
        &self,
        org_id: &str,
        repo_id: &str,
    ) -> Result<Vec<tellur_core::schema::types::FileAttribution>>;

    /// List repos in an org with their event counts.
    fn list_repos(&self, org_id: &str) -> Result<Vec<RepoSummary>>;

    /// List events in a repo, newest first, with cursor pagination by `seq`.
    /// Returns at most `limit` rows with `seq < before_seq` (when given).
    fn list_events(
        &self,
        org_id: &str,
        repo_id: &str,
        limit: u32,
        before_seq: Option<i64>,
    ) -> Result<Vec<StoredEvent>>;

    /// Aggregate an org-level activity rollup across its repos.
    fn org_report(&self, org_id: &str) -> Result<OrgReport>;

    /// The most recent events across all of an org's repos (newest first), for
    /// the dashboard activity feed.
    fn recent_org_events(&self, org_id: &str, limit: u32) -> Result<Vec<StoredEvent>>;

    // ─── Central policy distribution ─────────────────────────────────────────

    /// Create or update a named org policy; returns the new version number.
    fn put_policy(&self, org_id: &str, name: &str, content: &str) -> Result<i64>;

    /// List an org's policies (metadata only).
    fn list_policies(&self, org_id: &str) -> Result<Vec<PolicySummary>>;

    /// Fetch a named org policy (for `tellur policy pull`).
    fn get_policy(&self, org_id: &str, name: &str) -> Result<Option<PolicyDoc>>;

    // ─── Export portal ───────────────────────────────────────────────────────

    /// All provenance events for an org, oldest first (full export bundle).
    fn export_events(&self, org_id: &str) -> Result<Vec<StoredEvent>>;

    /// All audit records scoped to an org, oldest first.
    fn export_audit(&self, org_id: &str) -> Result<Vec<AuditRecord>>;

    // ─── Audit log (append-only, hash-chained) ──────────────────────────────

    /// Append an entry to the tamper-evident audit log.
    fn append_audit(&self, entry: &AuditEntry) -> Result<()>;

    /// Number of audit entries (tests/ops).
    fn audit_len(&self) -> Result<u64>;

    /// Recompute the audit hash chain and report whether it is intact.
    fn verify_audit_chain(&self) -> Result<bool>;

    // ─── SSO: identity, login transactions, sessions ─────────────────────────

    /// Provision an SSO-capable member with a (globally unique) email and no API
    /// token. Returns the new member id. Used to pre-authorize who may sign in
    /// via the IdP (no open self-registration).
    fn provision_member(
        &self,
        org_id: &str,
        display_name: &str,
        role: Role,
        email: &str,
    ) -> Result<String>;

    /// Resolve a verified email to a principal, if a member is provisioned.
    fn find_member_by_email(&self, email: &str) -> Result<Option<Principal>>;

    /// Resolve a bound `(issuer, subject)` to a principal, if any. The subject is
    /// only unique within an issuer, so both are required.
    fn find_member_by_oidc_subject(&self, issuer: &str, subject: &str)
    -> Result<Option<Principal>>;

    /// Bind an `(issuer, subject)` to a member **only if none is bound yet**.
    /// Returns `true` if it bound, `false` if the member already has a (different)
    /// binding — preventing a second IdP account on the same email from taking
    /// over the member.
    fn bind_oidc_subject(&self, member_id: &str, issuer: &str, subject: &str) -> Result<bool>;

    /// Persist a pending login transaction keyed by its CSRF `state`, including
    /// the browser-binding secret.
    fn put_login(
        &self,
        state: &str,
        pkce_verifier: &str,
        nonce: &str,
        browser_binding: &str,
    ) -> Result<()>;

    /// Count outstanding login transactions (for a hard anti-flood cap).
    fn count_logins(&self) -> Result<u64>;

    /// Delete login transactions older than `ttl_secs` (bounds the table against
    /// anonymous `/auth/login` floods). Returns the number removed.
    fn prune_expired_logins(&self, ttl_secs: i64) -> Result<u64>;

    /// Atomically consume a login transaction by `state` (delete + return).
    fn take_login(&self, state: &str) -> Result<Option<LoginTx>>;

    /// Create a session for a member, expiring `ttl_secs` from now. Returns the
    /// opaque session id (used as the cookie value).
    fn create_session(&self, member_id: &str, ttl_secs: i64) -> Result<String>;

    /// Resolve a non-expired session id to a principal.
    fn session_principal(&self, session_id: &str) -> Result<Option<Principal>>;

    /// Delete a session (logout). Returns `true` if one existed.
    fn delete_session(&self, session_id: &str) -> Result<bool>;

    // ─── SCIM 2.0 provisioning ───────────────────────────────────────────────

    /// Mint an org-scoped SCIM provisioning token (plaintext returned once).
    fn create_scim_token(&self, org_id: &str) -> Result<GeneratedToken>;

    /// Resolve a SCIM bearer token to its org id, or `None`.
    fn authenticate_scim(&self, token: &str) -> Result<Option<String>>;

    /// Provision a SCIM user (creates an active member + SSO identity). Errors if
    /// the email is already in use (the caller maps that to 409 Conflict).
    fn scim_create_user(
        &self,
        org_id: &str,
        email: &str,
        display_name: &str,
        role: Role,
        external_id: Option<&str>,
    ) -> Result<ScimUser>;

    /// List SCIM users in an org, optionally filtered by exact `userName`/email.
    fn scim_list_users(&self, org_id: &str, email_filter: Option<&str>) -> Result<Vec<ScimUser>>;

    /// Fetch one SCIM user by member id (tenant-scoped).
    fn scim_get_user(&self, org_id: &str, member_id: &str) -> Result<Option<ScimUser>>;

    /// Update a SCIM user's mutable fields (any `Some` is applied). Returns the
    /// updated user, or `None` if it does not exist in the org.
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
    ) -> Result<Option<ScimUser>>;

    // ─── Durable job queue ───────────────────────────────────────────────────

    /// Enqueue a job for an org; returns the new job id.
    fn enqueue_job(&self, org_id: &str, kind: &str) -> Result<String>;

    /// Atomically claim the oldest `queued` job (marking it `running`), across
    /// all orgs. Returns `None` when the queue is empty.
    fn claim_next_job(&self) -> Result<Option<Job>>;

    /// Mark a job `completed` with its JSON result.
    fn complete_job(&self, job_id: &str, result_json: &str) -> Result<()>;

    /// Mark a job `failed` with an error message.
    fn fail_job(&self, job_id: &str, error: &str) -> Result<()>;

    /// Fetch a job by id, tenant-scoped to its org.
    fn get_job(&self, org_id: &str, job_id: &str) -> Result<Option<Job>>;

    // ─── SCIM Groups (group-based role sync) ─────────────────────────────────

    /// Create a SCIM group with the given members, then recompute each member's
    /// org role from their group memberships.
    fn scim_create_group(
        &self,
        org_id: &str,
        display_name: &str,
        external_id: Option<&str>,
        members: &[String],
    ) -> Result<ScimGroup>;

    /// List groups in an org, optionally filtered by exact `displayName`.
    fn scim_list_groups(&self, org_id: &str, name_filter: Option<&str>) -> Result<Vec<ScimGroup>>;

    /// Fetch one group by id (tenant-scoped).
    fn scim_get_group(&self, org_id: &str, group_id: &str) -> Result<Option<ScimGroup>>;

    /// Update a group's name/externalId and (if `Some`) replace its membership,
    /// recomputing affected members' roles. Returns the updated group, or `None`.
    fn scim_update_group(
        &self,
        org_id: &str,
        group_id: &str,
        display_name: Option<&str>,
        external_id: Option<&str>,
        members: Option<&[String]>,
    ) -> Result<Option<ScimGroup>>;

    /// Delete a group, recomputing the roles of its former members. Returns
    /// `true` if it existed.
    fn scim_delete_group(&self, org_id: &str, group_id: &str) -> Result<bool>;
}
