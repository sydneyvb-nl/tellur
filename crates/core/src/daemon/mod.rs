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
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};

use crate::schema::types::TraceEvent;
use crate::storage::TraceIndex;

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
    let state = DaemonState {
        repo_root: config.repo_root.clone(),
    };
    let app = build_router(state);

    println!("TraceGit daemon listening on {}", addr);
    println!("Repository: {}", config.repo_root.display());
    println!();
    println!("Endpoints:");
    println!("  POST /event    — Submit a single event");
    println!("  POST /events   — Submit multiple events");
    println!("  GET  /status   — Daemon status");
    println!("  GET  /sessions — List sessions");
    println!("  POST /export   — Generate export bundle");

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
    Json(req): Json<SubmitEventRequest>,
) -> (StatusCode, Json<ApiResponse>) {
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
    Json(req): Json<SubmitEventsRequest>,
) -> (StatusCode, Json<ApiResponse>) {
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

async fn export_bundle(State(state): State<Arc<DaemonState>>) -> Json<ApiResponse> {
    let traces_dir = state.repo_root.join(".tracegit").join("traces");
    match crate::storage::read_events(&traces_dir) {
        Ok(events) => Json(ApiResponse {
            status: "ok".to_string(),
            data: Some(serde_json::json!({
                "event_count": events.len(),
                "events": events,
            })),
        }),
        Err(e) => Json(ApiResponse {
            status: "error".to_string(),
            data: Some(serde_json::json!({ "error": e.to_string() })),
        }),
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn write_event_to_disk(repo_root: &Path, event: &TraceEvent) -> Result<String> {
    let traces_dir = repo_root.join(".tracegit").join("traces");
    std::fs::create_dir_all(&traces_dir)?;

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let log_path = traces_dir.join(format!("events-{}.jsonl", date));

    // Append event to JSONL
    let json = serde_json::to_string(event)?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&log_path)?;
    writeln!(file, "{}", json)?;

    // Index the event
    let index_path = repo_root.join(".tracegit").join("index").join("tracegit.db");
    let index = TraceIndex::open(&index_path)?;
    index.index_event(event)?;

    Ok(event.id.clone())
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
        };
        let _router = build_router(state);
    }
}
