//! Shared API infrastructure: the `Authorization`/session authentication
//! extractor, tenant/role guards, cookie helpers, and small response utilities
//! used by every endpoint module.

pub(crate) use axum::Json;
pub(crate) use axum::extract::{FromRequestParts, Path, Query, State};
pub(crate) use axum::http::HeaderValue;
pub(crate) use axum::http::StatusCode;
pub(crate) use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
pub(crate) use axum::http::request::Parts;
pub(crate) use axum::response::{Html, IntoResponse, Redirect, Response};
pub(crate) use serde::Deserialize;
pub(crate) use serde_json::{Value, json};
pub(crate) use tellur_core::redaction::RedactionEngine;
pub(crate) use tellur_core::schema::types::FileAttribution;

pub(crate) use crate::app::AppState;
pub(crate) use crate::auth::{Principal, Role};
pub(crate) use crate::error::ServerError;
pub(crate) use crate::oidc::{self, Pkce};
pub(crate) use crate::review;
pub(crate) use crate::storage::{ActivityGroup, AuditEntry, DevicePoll, IngestEvent};

/// Name of the session cookie set after a successful SSO login.
pub(crate) const SESSION_COOKIE: &str = "tellur_session";

/// Name of the short-lived cookie that binds an OIDC login flow to the browser
/// that initiated it (defends against login-CSRF / session fixation).
pub(crate) const LOGIN_COOKIE: &str = "tellur_login";

/// Hard cap on outstanding OIDC login transactions (anti-flood, in addition to
/// the TTL prune). New `/auth/login` requests are refused past this.
pub(crate) const MAX_OUTSTANDING_LOGINS: u64 = 10_000;

/// Maximum events accepted in a single ingest request.
pub(crate) const MAX_EVENTS_PER_REQUEST: usize = 1000;

/// Name of the short-lived cookie that remembers where to send the browser after
/// an OIDC login (e.g. back to the device-approval page). Scoped to `/auth`.
pub(crate) const RETURN_COOKIE: &str = "tellur_return";

/// Lifetime of a device-authorization request (`tellur login`). The CLI must be
/// approved within this window or it must restart the flow.
pub(crate) const DEVICE_TTL_SECS: i64 = 15 * 60;

/// Suggested poll interval (seconds) handed to the `tellur login` client.
pub(crate) const DEVICE_POLL_INTERVAL_SECS: i64 = 5;

/// Hard cap on outstanding device-authorization requests (anti-flood).
pub(crate) const MAX_OUTSTANDING_DEVICE: u64 = 10_000;

/// Default and maximum page size for event listings.
pub(crate) const DEFAULT_PAGE: u32 = 50;
pub(crate) const MAX_PAGE: u32 = 200;

/// Cap on events returned for a single session replay. Higher than a list page
/// (a replay wants the whole session), but still bounded; the response notes
/// when it was hit so the UI can say the replay is truncated.
pub(crate) const SESSION_REPLAY_LIMIT: u32 = 5000;

/// Run a blocking store operation off the async worker threads, flattening the
/// join + operation errors into a `ServerError`.
pub(crate) async fn run_blocking<T, F>(f: F) -> Result<T, ServerError>
where
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ServerError::Internal(anyhow::anyhow!("blocking task failed: {e}")))?
        .map_err(ServerError::Internal)
}

/// Verify the caller's org matches the path org; audit + reject otherwise.
pub(crate) fn ensure_same_org(
    state: &AppState,
    principal: &Principal,
    org_id: &str,
    action: &str,
) -> Result<(), ServerError> {
    ensure_org_role(state, principal, org_id, Role::Viewer, action)
}

/// Verify the caller belongs to the path org *and* meets the required role;
/// audit + reject (403) otherwise.
pub(crate) fn ensure_org_role(
    state: &AppState,
    principal: &Principal,
    org_id: &str,
    required: Role,
    action: &str,
) -> Result<(), ServerError> {
    if org_id != principal.org_id || !principal.role.allows(required) {
        state
            .store
            .append_audit(&AuditEntry {
                org_id: Some(principal.org_id.clone()),
                actor_member_id: Some(principal.member_id.clone()),
                action: "access_denied".to_string(),
                detail: format!(
                    "{action} attempted_org={org_id} role={} required={}",
                    principal.role.as_str(),
                    required.as_str()
                ),
            })
            .map_err(ServerError::Internal)?;
        return Err(ServerError::Forbidden);
    }
    Ok(())
}

/// Audit a denied access attempt and return the `403` to surface. If writing
/// the audit entry fails, that becomes the (500) error instead — a denial must
/// always be recorded.
pub(crate) fn deny(
    state: &AppState,
    principal: &Principal,
    attempted_org: &str,
    action: &str,
    detail: &str,
) -> ServerError {
    if let Err(e) = state.store.append_audit(&AuditEntry {
        org_id: Some(principal.org_id.clone()),
        actor_member_id: Some(principal.member_id.clone()),
        action: action.to_string(),
        detail: format!("attempted_org={attempted_org} {detail}"),
    }) {
        return ServerError::Internal(e);
    }
    ServerError::Forbidden
}

/// The caller's **effective** role on a repo: their org-baseline role combined
/// with any additive per-repo grant (`max(org_role, grant)`). Grants only
/// elevate; they never reduce a member below their org role.
pub(crate) fn effective_role(
    state: &AppState,
    principal: &Principal,
    repo_id: &str,
) -> Result<Role, ServerError> {
    let grant = state
        .store
        .get_repo_role(&principal.org_id, repo_id, &principal.member_id)
        .map_err(ServerError::Internal)?;
    Ok(grant.map_or(principal.role, |g| principal.role.max(g)))
}

/// Authenticate the caller from a `Authorization: Bearer <token>` header.
/// Deny by default: any missing/invalid token is rejected.
impl FromRequestParts<AppState> for Principal {
    type Rejection = ServerError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Credential 1: an API bearer token (machine/CLI clients).
        if let Some(token) = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(str::to_string)
        {
            // The token id is a public lookup key (not the secret), safe to audit.
            let token_id = crate::auth::parse_token(&token).map(|(id, _)| id);
            // Run the (deliberately expensive) Argon2 verification off the async
            // worker thread so it cannot stall the runtime.
            let store = state.store.clone();
            let auth = tokio::task::spawn_blocking(move || store.authenticate(&token)).await;
            return match auth {
                Ok(Ok(Some(principal))) => Ok(principal),
                Ok(Ok(None)) => {
                    record_auth_denied(state, token_id.as_deref());
                    Err(ServerError::Unauthorized)
                }
                Ok(Err(e)) => Err(ServerError::Internal(e)),
                Err(join) => Err(ServerError::Internal(anyhow::anyhow!(
                    "auth task failed: {join}"
                ))),
            };
        }

        // Credential 2: a browser SSO session cookie.
        if let Some(sid) = session_cookie_value(&parts.headers) {
            let store = state.store.clone();
            let lookup = tokio::task::spawn_blocking(move || store.session_principal(&sid)).await;
            return match lookup {
                Ok(Ok(Some(principal))) => Ok(principal),
                Ok(Ok(None)) => Err(ServerError::Unauthorized),
                Ok(Err(e)) => Err(ServerError::Internal(e)),
                Err(join) => Err(ServerError::Internal(anyhow::anyhow!(
                    "session task failed: {join}"
                ))),
            };
        }

        // No credential presented: deny without auditing (anonymous traffic must
        // not be able to flood the audit log).
        Err(ServerError::Unauthorized)
    }
}

/// Extract a named cookie value from the `Cookie` header, if present.
pub(crate) fn cookie_value(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let header = headers.get(COOKIE)?.to_str().ok()?;
    for pair in header.split(';') {
        let pair = pair.trim();
        if let Some(v) = pair.strip_prefix(&format!("{name}="))
            && !v.is_empty()
        {
            return Some(v.to_string());
        }
    }
    None
}

/// Extract the session cookie value, if present.
pub(crate) fn session_cookie_value(headers: &axum::http::HeaderMap) -> Option<String> {
    cookie_value(headers, SESSION_COOKIE)
}

/// Build the `Set-Cookie` value for a new session.
pub(crate) fn session_cookie(sid: &str, max_age: i64) -> String {
    format!("{SESSION_COOKIE}={sid}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={max_age}")
}

/// Build the `Set-Cookie` value that clears the session (logout).
pub(crate) fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0")
}

/// Build the `Set-Cookie` value for the login-binding cookie. Scoped to `/auth`
/// so it is only sent to the callback.
pub(crate) fn login_cookie(binding: &str) -> String {
    format!(
        "{LOGIN_COOKIE}={binding}; HttpOnly; Secure; SameSite=Lax; Path=/auth; Max-Age={}",
        oidc::LOGIN_TTL_SECS
    )
}

/// Build the `Set-Cookie` value that clears the login-binding cookie.
pub(crate) fn clear_login_cookie() -> String {
    format!("{LOGIN_COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/auth; Max-Age=0")
}

/// Constant-time-ish equality for short secrets (avoids early-exit timing leak).
pub(crate) fn secret_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Whether a login transaction is older than its allowed lifetime.
pub(crate) fn login_expired(created_at: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(created_at) {
        Ok(t) => {
            chrono::Utc::now()
                .signed_duration_since(t.with_timezone(&chrono::Utc))
                .num_seconds()
                > oidc::LOGIN_TTL_SECS
        }
        Err(_) => true,
    }
}

/// Best-effort audit of a rejected authentication attempt. A failure to write
/// the entry must not turn the 401 into a 500, so it is logged, not propagated.
pub(crate) fn record_auth_denied(state: &AppState, token_id: Option<&str>) {
    state.metrics.inc_auth_denied();
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

pub(crate) fn principal_json(principal: &Principal) -> Value {
    json!({
        "org_id": principal.org_id,
        "member_id": principal.member_id,
        "role": principal.role.as_str(),
    })
}

/// Parse a range like `7d`/`30d`/`90d` or a bare day count into a day count,
/// clamped to 1..=365 (default 30).
pub(crate) fn parse_range_days(raw: Option<&str>) -> i64 {
    let parsed = raw
        .map(|s| s.trim().trim_end_matches('d'))
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(30);
    parsed.clamp(1, 365)
}

/// Validate a post-login return target: a same-origin absolute path only. Rejects
/// scheme-relative (`//host`) and absolute URLs so the redirect can't leave the
/// site, and rejects control characters (header/redirect splitting).
pub(crate) fn safe_return_path(raw: &str) -> Option<String> {
    if !raw.starts_with('/') || raw.starts_with("//") {
        return None;
    }
    if raw.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(raw.to_string())
}

/// Build the `Set-Cookie` value for the post-login return cookie (scoped to
/// `/auth` so it only rides along the OIDC callback).
pub(crate) fn return_cookie(path: &str) -> String {
    format!(
        "{RETURN_COOKIE}={path}; HttpOnly; Secure; SameSite=Lax; Path=/auth; Max-Age={}",
        oidc::LOGIN_TTL_SECS
    )
}

/// Build the `Set-Cookie` value that clears the return cookie.
pub(crate) fn clear_return_cookie() -> String {
    format!("{RETURN_COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/auth; Max-Age=0")
}

/// Recursively redact secret-looking strings anywhere in a JSON value.
pub(crate) fn redact_value(engine: &RedactionEngine, value: Value) -> Value {
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
