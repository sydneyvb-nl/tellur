//! Local HTTP daemon — event ingestion API
//!
//! Lightweight HTTP server for receiving events from AI tools,
//! editor extensions, and CI systems.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use axum::{
    Router,
    extract::{Json, Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    response::Html,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::schema::types::TraceEvent;
use crate::storage::{EventWriter, TraceIndex};

mod webhook;

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
    /// first run and stored at `.tellur/daemon.token`.
    pub token: String,
}

/// Load the daemon token from `.tellur/daemon.token`, creating a random one
/// (UUID v4-style via v7 simple form) on first use.
pub fn load_or_create_token(repo_root: &Path) -> Result<String> {
    let path = repo_root.join(".tellur").join("daemon.token");
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
    match headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
    {
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
        .route("/webhook/{source}", post(ingest_webhook_route))
        .route("/sessions", get(list_sessions))
        .route("/sessions/{session_id}/events", get(get_session_events))
        .route("/export", post(export_bundle))
        .with_state(Arc::new(state))
}

/// Run the daemon
pub async fn run_daemon(config: DaemonConfig) -> Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    if !addr.ip().is_loopback() {
        eprintln!(
            "⚠ Refusing to bind non-loopback address {}. Tellur's daemon is local-only.",
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

    println!("Tellur daemon listening on http://{}", addr);
    println!("Repository: {}", config.repo_root.display());
    println!();
    println!("Auth token (.tellur/daemon.token):");
    println!("  {}", token);
    println!("  Send as: Authorization: Bearer <token>");
    println!();
    println!("Endpoints:");
    println!("  POST /event    — Submit a single event   (auth required)");
    println!("  POST /events   — Submit multiple events   (auth required)");
    println!("  POST /webhook/{{source}} — Ingest a tool's native webhook (auth required)");
    println!("  GET  /status   — Daemon status");
    println!("  GET  /sessions — List sessions");
    println!("  GET  /sessions/{{id}}/events — Session event timeline");
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

/// Live-capture webhook for cloud agents (Devin and similar) that have no local
/// lifecycle hook or editor extension. The tool POSTs its native payload to
/// `POST /webhook/{source}`; Tellur normalizes it into canonical events and
/// **recomputes the hash chain** so provenance cannot be forged.
async fn ingest_webhook_route(
    State(state): State<Arc<DaemonState>>,
    AxumPath(source): AxumPath<String>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> (StatusCode, Json<ApiResponse>) {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    match webhook::ingest_webhook(&state.repo_root, &source, &body) {
        Ok(result) => (
            StatusCode::CREATED,
            Json(ApiResponse {
                status: "created".to_string(),
                data: Some(serde_json::json!({
                    "session_id": result.session_id,
                    "event_ids": result.event_ids,
                    "count": result.event_ids.len(),
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

async fn list_sessions(State(state): State<Arc<DaemonState>>) -> Json<ApiResponse> {
    let index_path = state
        .repo_root
        .join(".tellur")
        .join("index")
        .join("tellur.db");
    match TraceIndex::open(&index_path) {
        Ok(index) => {
            let events = index.event_count().unwrap_or(0);
            let sessions = index.list_dashboard_sessions(100).unwrap_or_default();
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

async fn get_session_events(
    State(state): State<Arc<DaemonState>>,
    AxumPath(session_id): AxumPath<String>,
) -> Json<ApiResponse> {
    let index_path = state
        .repo_root
        .join(".tellur")
        .join("index")
        .join("tellur.db");
    match TraceIndex::open(&index_path) {
        Ok(index) => match index.get_session_events(&session_id) {
            Ok(events) => {
                let events: Vec<_> = events
                    .into_iter()
                    .map(|event| {
                        serde_json::json!({
                            "id": event.id,
                            "session_id": event.session_id,
                            "timestamp": event.timestamp,
                            "event_type": event.event_type.as_wire(),
                            "actor": event.actor,
                            "body": format_event_body(&event.event_type, &event.payload),
                            "payload": event.payload,
                        })
                    })
                    .collect();
                Json(ApiResponse {
                    status: "ok".to_string(),
                    data: Some(serde_json::json!({ "events": events })),
                })
            }
            Err(e) => Json(ApiResponse {
                status: "error".to_string(),
                data: Some(serde_json::json!({ "error": e.to_string() })),
            }),
        },
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
    let traces_dir = state.repo_root.join(".tellur").join("traces");
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
                "error": "missing or invalid bearer token (see .tellur/daemon.token)"
            })),
        }),
    )
}

/// Persist a client-submitted event. The server **recomputes the hash chain**
/// via `EventWriter`, ignoring any client-supplied `event_hash`/`prev_hash`, so
/// that provenance cannot be forged through the HTTP API. UTC is used for the
/// log file partition to match `EventWriter`.
fn write_event_to_disk(repo_root: &Path, event: &TraceEvent) -> Result<String> {
    let traces_dir = repo_root.join(".tellur").join("traces");

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
    let index_path = repo_root.join(".tellur").join("index").join("tellur.db");
    let index = TraceIndex::open(&index_path)?;
    index.index_event(&stored)?;

    Ok(stored.id)
}

fn format_event_body(
    event_type: &crate::schema::types::EventType,
    payload: &serde_json::Value,
) -> String {
    let file_path = payload
        .get("file_path")
        .or_else(|| payload.get("file"))
        .and_then(|v| v.as_str());
    match event_type {
        crate::schema::types::EventType::FileWrite | crate::schema::types::EventType::FilePatch => {
            format!("Modified {}", file_path.unwrap_or("unknown file"))
        }
        crate::schema::types::EventType::FileRead => {
            format!("Read {}", file_path.unwrap_or("unknown file"))
        }
        crate::schema::types::EventType::FileDelete => {
            format!("Deleted {}", file_path.unwrap_or("unknown file"))
        }
        crate::schema::types::EventType::CommandPreExecute
        | crate::schema::types::EventType::CommandPostExecute
        | crate::schema::types::EventType::CommandExecution => payload
            .get("command")
            .and_then(|v| v.as_str())
            .map(|cmd| format!("Executed {}", cmd))
            .unwrap_or_else(|| "Command event".to_string()),
        crate::schema::types::EventType::UserPrompt
        | crate::schema::types::EventType::PromptSubmitted => payload
            .get("prompt_redacted")
            .or_else(|| payload.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap_or("Prompt submitted")
            .to_string(),
        other => other.as_wire(),
    }
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

    #[test]
    fn dashboard_avoids_html_string_injection_sinks() {
        let dashboard = include_str!("../../../../web/index.html");
        for sink in [".innerHTML", ".outerHTML", "document.write"] {
            assert!(
                !dashboard.contains(sink),
                "dashboard reintroduced unsafe DOM sink {sink}"
            );
        }
        assert!(dashboard.contains("textContent"));
        assert!(dashboard.contains("replaceChildren"));
    }

    #[tokio::test]
    async fn test_list_sessions_returns_dashboard_rows() {
        use crate::schema::types::*;

        let temp = tempfile::tempdir().unwrap();
        let repo_root = temp.path();
        let index_path = repo_root.join(".tellur").join("index").join("tellur.db");
        let index = TraceIndex::open(&index_path).unwrap();
        let mut session = Session::new(
            "repo".to_string(),
            Actor {
                name: "dev".to_string(),
                email: None,
                email_hash: None,
                actor_type: EventActor::Human,
            },
            AgentInfo {
                id: "claude-code".to_string(),
                name: "Claude Code".to_string(),
                version: None,
            },
        );
        session.id = "sess_daemon".to_string();
        index.index_session(&session).unwrap();

        let state = Arc::new(DaemonState {
            repo_root: repo_root.to_path_buf(),
            token: "test-token".to_string(),
        });
        let Json(body) = list_sessions(State(state)).await;
        let sessions = body.data.unwrap()["sessions"].as_array().unwrap().clone();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["id"], "sess_daemon");
        assert_eq!(sessions[0]["agent_id"], "claude-code");
    }

    #[tokio::test]
    async fn test_get_session_events_returns_timeline_events() {
        use crate::schema::types::{EventActor, EventType, TraceEvent};
        use axum::extract::Path;

        let temp = tempfile::tempdir().unwrap();
        let repo_root = temp.path();
        let index_path = repo_root.join(".tellur").join("index").join("tellur.db");
        let index = TraceIndex::open(&index_path).unwrap();
        index
            .index_event(&TraceEvent {
                schema: "tellur.event.v1".to_string(),
                id: "evt_daemon".to_string(),
                session_id: "sess_daemon".to_string(),
                timestamp: "2026-05-31T10:01:00Z".to_string(),
                event_type: EventType::FileWrite,
                actor: EventActor::Agent,
                payload: serde_json::json!({"file_path": "src/lib.rs"}),
                redaction: None,
                prev_hash: None,
                event_hash: Some("hash".to_string()),
            })
            .unwrap();

        let state = Arc::new(DaemonState {
            repo_root: repo_root.to_path_buf(),
            token: "test-token".to_string(),
        });
        let Json(body) = get_session_events(State(state), Path("sess_daemon".to_string())).await;
        let events = body.data.unwrap()["events"].as_array().unwrap().clone();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["id"], "evt_daemon");
        assert_eq!(events[0]["event_type"], "file.write");
        assert_eq!(events[0]["body"], "Modified src/lib.rs");
    }

    #[tokio::test]
    async fn test_webhook_route_requires_auth_and_ingests() {
        use axum::extract::Path;
        use axum::http::header::AUTHORIZATION;

        let temp = tempfile::tempdir().unwrap();
        let repo_root = temp.path();
        std::fs::create_dir_all(repo_root.join(".tellur").join("traces")).unwrap();
        std::fs::create_dir_all(repo_root.join(".tellur").join("index")).unwrap();
        let state = Arc::new(DaemonState {
            repo_root: repo_root.to_path_buf(),
            token: "test-token".to_string(),
        });
        let body = serde_json::json!({
            "session_id": "run-7",
            "messages": [{"type": "edit", "file_path": "src/main.rs"}]
        });

        // Unauthenticated request is rejected and writes nothing.
        let (status, _) = ingest_webhook_route(
            State(state.clone()),
            Path("devin".to_string()),
            HeaderMap::new(),
            Json(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        // Authenticated request ingests the normalized event.
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, "Bearer test-token".parse().unwrap());
        let (status, Json(resp)) = ingest_webhook_route(
            State(state.clone()),
            Path("devin".to_string()),
            headers,
            Json(body),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(resp.data.as_ref().unwrap()["count"], 1);
        assert_eq!(resp.data.as_ref().unwrap()["session_id"], "run-7");

        let index =
            TraceIndex::open(&repo_root.join(".tellur").join("index").join("tellur.db")).unwrap();
        let events = index.get_session_events("run-7").unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type.as_wire(), "file.write");
    }
}
