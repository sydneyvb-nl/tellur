//! Router assembly and shared application state.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};

use crate::config::Config;
use crate::metrics::Metrics;
use crate::ratelimit::RateLimiter;
use crate::storage::Store;

/// Shared, cheaply-cloneable application state.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub config: Arc<Config>,
    /// Per-principal rate limiter for the ingest endpoint.
    pub rate_limiter: Arc<RateLimiter>,
    /// In-process metrics, exposed at `/metrics`.
    pub metrics: Arc<Metrics>,
    /// OIDC SSO runtime, present only when SSO is configured.
    pub oidc: Option<Arc<crate::oidc::OidcRuntime>>,
}

/// Maximum accepted request body size (1 MiB).
pub const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Build the HTTP router. Operational endpoints (`/healthz`, `/readyz`) need no
/// auth; `/v1/*` endpoints authenticate and scope to the caller's org.
pub fn build_router(state: AppState) -> Router {
    let router = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        // OIDC SSO (browser): unauthenticated entry points; 404 when SSO is off.
        .route("/auth/login", get(crate::api::oidc_login))
        .route("/auth/callback", get(crate::api::oidc_callback))
        .route("/auth/logout", get(crate::api::oidc_logout))
        .route("/v1/me", get(crate::api::me))
        .route("/v1/orgs/{org_id}/me", get(crate::api::org_me))
        .route("/v1/orgs/{org_id}/repos", get(crate::api::list_repos))
        .route("/v1/orgs/{org_id}/report", get(crate::api::org_report))
        .route("/v1/orgs/{org_id}/dashboard", get(crate::api::dashboard))
        .route("/v1/orgs/{org_id}/overview", get(crate::api::overview))
        .route("/v1/orgs/{org_id}/activity", get(crate::api::activity))
        .route(
            "/v1/orgs/{org_id}/repos/{repo}",
            get(crate::api::repo_detail),
        )
        .route(
            "/v1/orgs/{org_id}/repos/{repo}/events",
            post(crate::api::ingest_events).get(crate::api::list_events),
        )
        .route(
            "/v1/orgs/{org_id}/repos/{repo}/attributions",
            post(crate::api::ingest_attributions).get(crate::api::list_attributions),
        )
        .route(
            "/v1/orgs/{org_id}/repos/{repo}/source",
            put(crate::api::set_repo_source),
        )
        .route("/v1/orgs/{org_id}/sessions", get(crate::api::list_sessions))
        .route(
            "/v1/orgs/{org_id}/sessions/{id}",
            get(crate::api::session_detail),
        )
        .route(
            "/v1/orgs/{org_id}/repos/{repo}/roles",
            get(crate::api::list_repo_roles),
        )
        .route(
            "/v1/orgs/{org_id}/repos/{repo}/roles/{member_id}",
            put(crate::api::set_repo_role).delete(crate::api::remove_repo_role),
        )
        .route(
            "/v1/orgs/{org_id}/repos/{repo}/export/slsa",
            get(crate::api::export_slsa).post(crate::api::export_slsa_job),
        )
        .route(
            "/v1/orgs/{org_id}/repos/{repo}/export/spdx",
            get(crate::api::export_spdx).post(crate::api::export_spdx_job),
        )
        .route("/v1/orgs/{org_id}/policies", get(crate::api::list_policies))
        .route(
            "/v1/orgs/{org_id}/policies/{name}",
            put(crate::api::put_policy).get(crate::api::get_policy),
        )
        .route(
            "/v1/orgs/{org_id}/export/events",
            post(crate::api::export_events),
        )
        .route(
            "/v1/orgs/{org_id}/export/audit",
            post(crate::api::export_audit),
        )
        .route(
            "/v1/orgs/{org_id}/export/evidence",
            post(crate::api::export_evidence),
        )
        .route("/v1/orgs/{org_id}/audit", get(crate::api::list_audit))
        .route("/v1/orgs/{org_id}/jobs", get(crate::api::list_jobs))
        .route("/v1/orgs/{org_id}/jobs/{id}", get(crate::api::get_job))
        .route(
            "/v1/orgs/{org_id}/policies/compliance",
            post(crate::api::enqueue_compliance).get(crate::api::get_compliance),
        )
        .route("/v1/orgs/{org_id}/members", get(crate::api::list_members))
        .route("/v1/orgs/{org_id}/groups", get(crate::api::list_groups))
        .route("/v1/orgs/{org_id}/sso-status", get(crate::api::sso_status))
        // SCIM 2.0 provisioning (org derived from the SCIM bearer token).
        .route(
            "/scim/v2/Users",
            get(crate::scim::list_users).post(crate::scim::create_user),
        )
        .route(
            "/scim/v2/Users/{id}",
            get(crate::scim::get_user)
                .put(crate::scim::replace_user)
                .patch(crate::scim::patch_user)
                .delete(crate::scim::delete_user),
        )
        .route(
            "/scim/v2/Groups",
            get(crate::scim::list_groups).post(crate::scim::create_group),
        )
        .route(
            "/scim/v2/Groups/{id}",
            get(crate::scim::get_group)
                .put(crate::scim::replace_group)
                .patch(crate::scim::patch_group)
                .delete(crate::scim::delete_group),
        );

    // Team dashboard SPA at /app (same-origin; embedded assets). Behind the
    // `dashboard` feature so a minimal API-only build can omit it.
    #[cfg(feature = "dashboard")]
    let router = router.merge(crate::dashboard::router());

    router
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

/// Prometheus metrics (no auth; no tenant data — only aggregate counters).
async fn metrics(State(state): State<AppState>) -> Response {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        state.metrics.render(),
    )
        .into_response()
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
