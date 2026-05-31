//! Local HTTP daemon — event ingestion API
//!
//! Lightweight HTTP server for receiving events from AI tools,
//! editor extensions, and CI systems.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use axum::{
    extract::{Json, State},
    http::{HeaderMap, StatusCode},
    response::Html,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};

use crate::schema::types::TraceEvent;
use crate::storage::{EventWriter, TraceIndex};

/// Daemon configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub host: String,
    pub port: u16,
    pub repo_root: PathBuf,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 4917,
            repo_root: PathBuf::from("."),
        }
    }
}

/// Shared state for the daemon
#[derive(Clone)]
pub struct DaemonState {
    pub repo_root: PathBuf,
    /// Bearer token required for mutating/exporting endpoints. Generated on
    /// first run and stored at `.tracegit/daemon.token`.
    pub token: String,
}

/// Load the daemon token from `.tracegit/daemon.token`, creating a random one
/// (UUID v4-style via v7 simple form) on first use.
pub fn load_or_create_token(repo_root: &Path) -> Result<String> {
    let path = repo_root.join(".tracegit").join("daemon.token");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let token = format!(
        "{}{}",
        uuid::Uuid::now_v7().simple(),
        uuid::Uuid::now_v7().simple()
    );
    std::fs::write(&path, &token)?;
    Ok(token)
}

/// Reject requests whose Host header is not loopback (defends against
/// DNS-rebinding from a browser). Returns true if the request is acceptable.
fn host_is_local(headers: &HeaderMap) -> bool {
    match headers.get(axum::http::header::HOST).and_then(|v| v.to_str().ok()) {
        // No Host header (e.g. HTTP/2 authority handled elsewhere) — allow.
        None => true,
        Some(host) => {
            let h = host.split(':').next().unwrap_or("");
            h == "localhost" || h == "127.0.0.1" || h == "[::1]" || h == "::1"
        }
    }
}

/// Check the bearer token on a protected request.
fn authorized(state: &DaemonState, headers: &HeaderMap) -> bool {
    if !host_is_local(headers) {
        return false;
    }
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t == state.token)
        .unwrap_or(false)
}

/// Generic JSON response
#[derive(Serialize)]
struct ApiResponse {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
}

/// Event submission request
#[derive(Debug, Deserialize)]
pub struct SubmitEventRequest {
    pub event: TraceEvent,
}

/// Batch event submission request
#[derive(Debug, Deserialize)]
pub struct SubmitEventsRequest {
    pub events: Vec<TraceEvent>,
}

/// Build the axum router
pub fn build_router(state: DaemonState) -> Router {
    Router::new()
        .route("/", get(get_ui))
        .route("/status", get(get_status))
        .route("/event", post(submit_event))
        .route("/events", post(submit_events))
        .route("/sessions", get(list_sessions))
        .route("/export", post(export_bundle))
        .with_state(Arc::new(state))
}

/// Run the daemon
pub async fn run_daemon(config: DaemonConfig) -> Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    if !addr.ip().is_loopback() {
        eprintln!(
            "⚠ Refusing to bind non-loopback address {}. TraceGit's daemon is local-only.",
            addr.ip()
        );
        anyhow::bail!("daemon must bind a loopback address (127.0.0.1 or ::1)");
    }

    let token = load_or_create_token(&config.repo_root)?;
    let state = DaemonState {
        repo_root: config.repo_root.clone(),
        token: token.clone(),
    };
    let app = build_router(state);

    println!("TraceGit daemon listening on http://{}", addr);
    println!("Repository: {}", config.repo_root.display());
    println!();
    println!("Auth token (.tracegit/daemon.token):");
    println!("  {}", token);
    println!("  Send as: Authorization: Bearer <token>");
    println!();
    println!("Endpoints:");
    println!("  POST /event    — Submit a single event   (auth required)");
    println!("  POST /events   — Submit multiple events   (auth required)");
    println!("  GET  /status   — Daemon status");
    println!("  GET  /sessions — List sessions");
    println!("  POST /export   — Generate export bundle   (auth required)");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ─── Handlers ──────────────────────────────────────────────────────────────

async fn get_ui() -> Html<&'static str> {
    Html(std::include_str!("../../../../web/index.html"))
}

async fn get_status() -> Json<ApiResponse> {
    Json(ApiResponse {
        status: "ok".to_string(),
        data: Some(serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "status": "running",
        })),
    })
}

async fn submit_event(
    State(state): State<Arc<DaemonState>>,
    headers: HeaderMap,
    Json(req): Json<SubmitEventRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    match write_event_to_disk(&state.repo_root, &req.event) {
        Ok(id) => (
            StatusCode::CREATED,
            Json(ApiResponse {
                status: "created".to_string(),
                data: Some(serde_json::json!({ "event_id": id })),
            }),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                status: "error".to_string(),
                data: Some(serde_json::json!({ "error": e.to_string() })),
            }),
        ),
    }
}

async fn submit_events(
    State(state): State<Arc<DaemonState>>,
    headers: HeaderMap,
    Json(req): Json<SubmitEventsRequest>,
) -> (StatusCode, Json<ApiResponse>) {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    let mut ids = Vec::new();
    for event in &req.events {
        match write_event_to_disk(&state.repo_root, event) {
            Ok(id) => ids.push(id),
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiResponse {
                        status: "error".to_string(),
                        data: Some(serde_json::json!({
                            "error": format!("Failed after {} events: {}", ids.len(), e),
                            "written": ids.len(),
                        })),
                    }),
                );
            }
        }
    }
    (
        StatusCode::CREATED,
        Json(ApiResponse {
            status: "created".to_string(),
            data: Some(serde_json::json!({ "event_ids": ids, "count": ids.len() })),
        }),
    )
}

async fn list_sessions(State(state): State<Arc<DaemonState>>) -> Json<ApiResponse> {
    let index_path = state.repo_root.join(".tracegit").join("index").join("tracegit.db");
    match TraceIndex::open(&index_path) {
        Ok(index) => {
            let sessions = index.session_count().unwrap_or(0);
            let events = index.event_count().unwrap_or(0);
            Json(ApiResponse {
                status: "ok".to_string(),
                data: Some(serde_json::json!({
                    "sessions": sessions,
                    "total_events": events,
                })),
            })
        }
        Err(e) => Json(ApiResponse {
            status: "error".to_string(),
            data: Some(serde_json::json!({ "error": e.to_string() })),
        }),
    }
}

async fn export_bundle(
    State(state): State<Arc<DaemonState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse>) {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    let traces_dir = state.repo_root.join(".tracegit").join("traces");
    match crate::storage::read_events(&traces_dir) {
        Ok(events) => (
            StatusCode::OK,
            Json(ApiResponse {
                status: "ok".to_string(),
                data: Some(serde_json::json!({
                    "event_count": events.len(),
                    "events": events,
                })),
            }),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                status: "error".to_string(),
                data: Some(serde_json::json!({ "error": e.to_string() })),
            }),
        ),
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn unauthorized() -> (StatusCode, Json<ApiResponse>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiResponse {
            status: "unauthorized".to_string(),
            data: Some(serde_json::json!({
                "error": "missing or invalid bearer token (see .tracegit/daemon.token)"
            })),
        }),
    )
}

/// Persist a client-submitted event. The server **recomputes the hash chain**
/// via `EventWriter`, ignoring any client-supplied `event_hash`/`prev_hash`, so
/// that provenance cannot be forged through the HTTP API. UTC is used for the
/// log file partition to match `EventWriter`.
fn write_event_to_disk(repo_root: &Path, event: &TraceEvent) -> Result<String> {
    let traces_dir = repo_root.join(".tracegit").join("traces");

    let mut writer = EventWriter::new(&traces_dir);
    writer.open()?;
    let stored = writer.write_event(
        &event.session_id,
        &event.event_type.as_wire(),
        &serde_json::to_value(&event.actor)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string()),
        event.payload.clone(),
        event.redaction.clone(),
    )?;
    writer.close();

    // Index the re-hashed event.
    let index_path = repo_root.join(".tracegit").join("index").join("tracegit.db");
    let index = TraceIndex::open(&index_path)?;
    index.index_event(&stored)?;

    Ok(stored.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DaemonConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 4917);
    }

    #[test]
    fn test_build_router() {
        let state = DaemonState {
            repo_root: PathBuf::from("/tmp"),
            token: "test-token".to_string(),
        };
        let _router = build_router(state);
    }
}
