//! HTTP API: authentication extractor + tenant-scoped endpoints.
//!
//! Handlers stay thin: authenticate, authorize on **object + tenant**, audit,
//! respond. Authorization is checked against the caller's own org, so a token
//! for one org cannot reach another org's resources (BOLA prevention).

use axum::Json;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::auth::Principal;
use crate::error::ServerError;
use crate::storage::AuditEntry;

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
