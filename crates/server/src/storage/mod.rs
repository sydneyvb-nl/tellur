//! Storage abstraction.
//!
//! A single `Store` trait decouples handlers/services from the backend. The
//! default `SqliteStore` is zero-config for single-node self-hosting; a Postgres
//! backend (B5) will implement the same trait for horizontal scale.
//!
//! Tenant isolation is enforced here: identity/data rows carry an `org_id` and
//! lookups resolve a caller to a tenant-scoped [`Principal`], so handlers cannot
//! accidentally cross org boundaries.

use anyhow::Result;

use crate::auth::{GeneratedToken, Principal, Role};

pub mod sqlite;
pub use sqlite::SqliteStore;

/// An organization (tenant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Org {
    pub id: String,
    pub name: String,
    pub created_at: String,
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

    // ─── Audit log (append-only, hash-chained) ──────────────────────────────

    /// Append an entry to the tamper-evident audit log.
    fn append_audit(&self, entry: &AuditEntry) -> Result<()>;

    /// Number of audit entries (tests/ops).
    fn audit_len(&self) -> Result<u64>;

    /// Recompute the audit hash chain and report whether it is intact.
    fn verify_audit_chain(&self) -> Result<bool>;
}
