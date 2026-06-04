//! Router assembly and shared application state.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};

use crate::config::Config;
use crate::ratelimit::RateLimiter;
use crate::storage::Store;

/// Shared, cheaply-cloneable application state.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub config: Arc<Config>,
    /// Per-principal rate limiter for the ingest endpoint.
    pub rate_limiter: Arc<RateLimiter>,
}

/// Maximum accepted request body size (1 MiB).
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Build the HTTP router. Operational endpoints (`/healthz`, `/readyz`) need no
/// auth; `/v1/*` endpoints authenticate and scope to the caller's org.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v1/me", get(crate::api::me))
        .route("/v1/orgs/{org_id}/me", get(crate::api::org_me))
        .route("/v1/orgs/{org_id}/repos", get(crate::api::list_repos))
        .route("/v1/orgs/{org_id}/report", get(crate::api::org_report))
        .route(
            "/v1/orgs/{org_id}/repos/{repo}/events",
            post(crate::api::ingest_events).get(crate::api::list_events),
        )
        .route("/v1/orgs/{org_id}/policies", get(crate::api::list_policies))
        .route(
            "/v1/orgs/{org_id}/policies/{name}",
            put(crate::api::put_policy).get(crate::api::get_policy),
        )
        .route(
            "/v1/orgs/{org_id}/export/events",
            get(crate::api::export_events),
        )
        .route(
            "/v1/orgs/{org_id}/export/audit",
            get(crate::api::export_audit),
        )
        // Cap request bodies (defense against unrestricted resource consumption).
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

/// Liveness: the process is up. No state access.
async fn healthz() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "service": "tellur-server",
            "version": env!("CARGO_PKG_VERSION"),
        })),
    )
}

/// Readiness: dependencies (the store) are reachable.
async fn readyz(State(state): State<AppState>) -> Response {
    match state.store.health_check() {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "status": "ready" })),
        )
            .into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "status": "not_ready" })),
        )
            .into_response(),
    }
}
