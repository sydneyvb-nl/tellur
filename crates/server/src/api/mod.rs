//! HTTP API: authentication extractor + tenant-scoped endpoints.
//!
//! Handlers stay thin: authenticate, authorize on **object + tenant**, audit,
//! respond. Authorization is checked against the caller's own org, so a token
//! for one org cannot reach another org's resources (BOLA prevention).
//!
//! Shared infrastructure (the auth extractor, tenant/role guards, cookie and
//! response helpers) lives in [`common`]; the endpoints are grouped by domain.

mod analytics;
mod common;
mod device;
mod exports;
mod policies;
mod rbac;
mod repos;
mod sessions;
mod sso;

pub use analytics::*;
pub use device::*;
pub use exports::*;
pub use policies::*;
pub use rbac::*;
pub use repos::*;
pub use sessions::*;
pub use sso::*;
