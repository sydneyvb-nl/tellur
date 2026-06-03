//! Storage abstraction.
//!
//! A single `Store` trait decouples handlers/services from the backend. The
//! default `SqliteStore` is zero-config for single-node self-hosting; a Postgres
//! backend (B5) will implement the same trait for horizontal scale.

use anyhow::Result;

pub mod sqlite;
pub use sqlite::SqliteStore;

/// Backend-agnostic storage interface. Tenant-scoped data methods are added in
/// later phases; B0 only needs schema setup and a health probe.
pub trait Store: Send + Sync {
    /// Apply schema migrations. Must be idempotent.
    fn migrate(&self) -> Result<()>;

    /// Cheap connectivity check used by the readiness probe.
    fn health_check(&self) -> Result<()>;
}
