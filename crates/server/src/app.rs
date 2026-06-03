//! Router assembly and shared application state.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;

use crate::config::Config;
use crate::storage::Store;

/// Shared, cheaply-cloneable application state.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<dyn Store>,
    pub config: Arc<Config>,
}

/// Build the HTTP router. B0 exposes only operational endpoints (no data, no
/// tenant access), so none require auth.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v1/me", get(crate::api::me))
        .route("/v1/orgs/{org_id}/me", get(crate::api::org_me))
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
