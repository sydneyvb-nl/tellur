//! HTTP API: authentication extractor + tenant-scoped endpoints.
//!
//! Handlers stay thin: authenticate, authorize on **object + tenant**, audit,
//! respond. Authorization is checked against the caller's own org, so a token
//! for one org cannot reach another org's resources (BOLA prevention).

use axum::Json;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use serde::Deserialize;
use serde_json::{Value, json};
use tellur_core::redaction::RedactionEngine;

use crate::app::AppState;
use crate::auth::{Principal, Role};
use crate::error::ServerError;
use crate::storage::{AuditEntry, IngestEvent};

/// Maximum events accepted in a single ingest request.
const MAX_EVENTS_PER_REQUEST: usize = 1000;

/// Default and maximum page size for event listings.
const DEFAULT_PAGE: u32 = 50;
const MAX_PAGE: u32 = 200;

/// Verify the caller's org matches the path org; audit + reject otherwise.
fn ensure_same_org(
    state: &AppState,
    principal: &Principal,
    org_id: &str,
    action: &str,
) -> Result<(), ServerError> {
    if org_id != principal.org_id {
        state
            .store
            .append_audit(&AuditEntry {
                org_id: Some(principal.org_id.clone()),
                actor_member_id: Some(principal.member_id.clone()),
                action: "access_denied".to_string(),
                detail: format!("{action} attempted_org={org_id}"),
            })
            .map_err(ServerError::Internal)?;
        return Err(ServerError::Forbidden);
    }
    Ok(())
}

/// Authenticate the caller from a `Authorization: Bearer <token>` header.
/// Deny by default: any missing/invalid token is rejected.
impl FromRequestParts<AppState> for Principal {
    type Rejection = ServerError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // A presented-but-invalid token is a credential-probing signal worth
        // auditing; a request with no Authorization header is not (and auditing
        // it would let anonymous traffic flood the audit log).
        let Some(token) = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(str::to_string)
        else {
            return Err(ServerError::Unauthorized);
        };

        // The token id is a public lookup key (not the secret), safe to audit.
        let token_id = crate::auth::parse_token(&token).map(|(id, _)| id);

        // Run the (deliberately expensive) Argon2 verification off the async
        // worker thread so it cannot stall the runtime.
        let store = state.store.clone();
        let auth = tokio::task::spawn_blocking(move || store.authenticate(&token)).await;

        match auth {
            Ok(Ok(Some(principal))) => Ok(principal),
            Ok(Ok(None)) => {
                record_auth_denied(state, token_id.as_deref());
                Err(ServerError::Unauthorized)
            }
            Ok(Err(e)) => Err(ServerError::Internal(e)),
            Err(join) => Err(ServerError::Internal(anyhow::anyhow!(
                "auth task failed: {join}"
            ))),
        }
    }
}

/// Best-effort audit of a rejected authentication attempt. A failure to write
/// the entry must not turn the 401 into a 500, so it is logged, not propagated.
fn record_auth_denied(state: &AppState, token_id: Option<&str>) {
    let detail = match token_id {
        Some(id) => format!("token_id={id}"),
        None => "malformed_token".to_string(),
    };
    if let Err(e) = state.store.append_audit(&AuditEntry {
        org_id: None,
        actor_member_id: None,
        action: "auth_denied".to_string(),
        detail,
    }) {
        tracing::error!(error = %e, "failed to write auth_denied audit entry");
    }
}

fn principal_json(principal: &Principal) -> Value {
    json!({
        "org_id": principal.org_id,
        "member_id": principal.member_id,
        "role": principal.role.as_str(),
    })
}

/// `GET /v1/me` — the authenticated caller's identity.
pub async fn me(
    State(state): State<AppState>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(principal.org_id.clone()),
            actor_member_id: Some(principal.member_id.clone()),
            action: "me".to_string(),
            detail: String::new(),
        })
        .map_err(ServerError::Internal)?;
    Ok(Json(principal_json(&principal)))
}

/// `GET /v1/orgs/{org_id}/me` — same, but tenant-scoped. The path org must match
/// the caller's org; otherwise it is a forbidden cross-tenant access.
pub async fn org_me(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    if org_id != principal.org_id {
        state
            .store
            .append_audit(&AuditEntry {
                org_id: Some(principal.org_id.clone()),
                actor_member_id: Some(principal.member_id.clone()),
                action: "access_denied".to_string(),
                detail: format!("attempted_org={org_id}"),
            })
            .map_err(ServerError::Internal)?;
        return Err(ServerError::Forbidden);
    }

    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id),
            actor_member_id: Some(principal.member_id.clone()),
            action: "org_me".to_string(),
            detail: String::new(),
        })
        .map_err(ServerError::Internal)?;
    Ok(Json(principal_json(&principal)))
}

/// Wire format for an ingest request.
#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    pub events: Vec<IngestEventWire>,
}

/// Wire format for a single event. The hub assigns the id and (re)computes the
/// hash chain, so any client-supplied hashes are ignored.
#[derive(Debug, Deserialize)]
pub struct IngestEventWire {
    pub session_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

/// `POST /v1/orgs/{org}/repos/{repo}/events` — ingest provenance events.
///
/// Requires a contributor+ token for the path org (cross-tenant → forbidden).
/// Inbound payloads are secret-redacted; the hub recomputes the per-repo hash
/// chain so provenance cannot be forged. Rate-limited per member; body size is
/// capped by the router layer and event count by `MAX_EVENTS_PER_REQUEST`.
pub async fn ingest_events(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    principal: Principal,
    Json(req): Json<IngestRequest>,
) -> Result<Json<Value>, ServerError> {
    // Tenant + role authorization (object + tenant, not just role).
    if org_id != principal.org_id || !principal.role.allows(Role::Contributor) {
        state
            .store
            .append_audit(&AuditEntry {
                org_id: Some(principal.org_id.clone()),
                actor_member_id: Some(principal.member_id.clone()),
                action: "ingest_denied".to_string(),
                detail: format!("attempted_org={org_id} role={}", principal.role.as_str()),
            })
            .map_err(ServerError::Internal)?;
        return Err(ServerError::Forbidden);
    }

    // Per-member rate limit.
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }

    // Validate batch size.
    if req.events.is_empty() {
        return Err(ServerError::BadRequest("no events provided".to_string()));
    }
    if req.events.len() > MAX_EVENTS_PER_REQUEST {
        return Err(ServerError::BadRequest(format!(
            "too many events: {} (max {MAX_EVENTS_PER_REQUEST})",
            req.events.len()
        )));
    }

    let repo = state
        .store
        .ensure_repo(&org_id, &repo)
        .map_err(ServerError::Internal)?;

    // Redact secrets from inbound payloads before storage.
    let engine = RedactionEngine::default_engine();
    let events: Vec<IngestEvent> = req
        .events
        .into_iter()
        .map(|e| IngestEvent {
            session_id: e.session_id,
            timestamp: e
                .timestamp
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
            event_type: e.event_type,
            actor: e.actor.unwrap_or_else(|| "agent".to_string()),
            payload: redact_value(&engine, e.payload),
        })
        .collect();

    let ids = state
        .store
        .append_events(&org_id, &repo.id, &events)
        .map_err(ServerError::Internal)?;

    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id),
            actor_member_id: Some(principal.member_id.clone()),
            action: "events.ingest".to_string(),
            detail: format!("repo={} count={}", repo.id, ids.len()),
        })
        .map_err(ServerError::Internal)?;

    Ok(Json(json!({
        "repo_id": repo.id,
        "count": ids.len(),
        "event_ids": ids,
    })))
}

/// `GET /v1/orgs/{org}/repos` — list the org's repos with event counts.
pub async fn list_repos(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "list_repos")?;
    let repos = state
        .store
        .list_repos(&org_id)
        .map_err(ServerError::Internal)?;
    Ok(Json(json!({ "repos": repos })))
}

/// Query parameters for event listing (cursor pagination by `seq`).
#[derive(Debug, Deserialize)]
pub struct ListEventsParams {
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub before: Option<i64>,
}

/// `GET /v1/orgs/{org}/repos/{repo}/events` — newest-first, paginated.
pub async fn list_events(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    Query(params): Query<ListEventsParams>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "list_events")?;
    let repo = state
        .store
        .find_repo(&org_id, &repo)
        .map_err(ServerError::Internal)?
        .ok_or(ServerError::NotFound)?;

    let limit = params.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let events = state
        .store
        .list_events(&org_id, &repo.id, limit, params.before)
        .map_err(ServerError::Internal)?;

    // Cursor for the next page: the seq of the last row, only if the page is full.
    let next_before = if events.len() as u32 == limit {
        events.last().map(|e| e.seq)
    } else {
        None
    };
    Ok(Json(json!({
        "repo_id": repo.id,
        "events": events,
        "next_before": next_before,
    })))
}

/// `GET /v1/orgs/{org}/report` — org-level activity rollup across repos.
pub async fn org_report(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "report")?;
    let report = state
        .store
        .org_report(&org_id)
        .map_err(ServerError::Internal)?;
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id),
            actor_member_id: Some(principal.member_id.clone()),
            action: "report".to_string(),
            detail: format!("total_events={}", report.total_events),
        })
        .map_err(ServerError::Internal)?;
    serde_json::to_value(&report)
        .map(Json)
        .map_err(|e| ServerError::Internal(e.into()))
}

/// Recursively redact secret-looking strings anywhere in a JSON value.
fn redact_value(engine: &RedactionEngine, value: Value) -> Value {
    match value {
        Value::String(s) => Value::String(engine.scan_and_redact(&s).redacted_content.unwrap_or(s)),
        Value::Array(items) => {
            Value::Array(items.into_iter().map(|v| redact_value(engine, v)).collect())
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, redact_value(engine, v)))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_value_strips_secrets_recursively() {
        let engine = RedactionEngine::default_engine();
        let input = json!({
            "command": "deploy --key AKIAIOSFODNN7EXAMPLE",
            "nested": { "list": ["plain", "password=hunter2supersecretvalue"] },
            "count": 3
        });
        let out = redact_value(&engine, input);
        let s = out.to_string();
        assert!(!s.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!s.contains("hunter2supersecretvalue"));
        assert!(s.contains("[REDACTED]"));
        // Non-string values are preserved.
        assert_eq!(out["count"], 3);
    }
}
