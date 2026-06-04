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
pub mod sqlite;
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

    /// Look up a repo by `(org_id, name)` without creating it.
    fn find_repo(&self, org_id: &str, name: &str) -> Result<Option<Repo>>;

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
}
