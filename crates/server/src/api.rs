//! HTTP API: authentication extractor + tenant-scoped endpoints.
//!
//! Handlers stay thin: authenticate, authorize on **object + tenant**, audit,
//! respond. Authorization is checked against the caller's own org, so a token
//! for one org cannot reach another org's resources (BOLA prevention).

use axum::Json;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use tellur_core::redaction::RedactionEngine;
use tellur_core::schema::types::FileAttribution;

use crate::app::AppState;
use crate::auth::{Principal, Role};
use crate::error::ServerError;
use crate::oidc::{self, Pkce};
use crate::review;
use crate::storage::{ActivityGroup, AuditEntry, IngestEvent};

/// Name of the session cookie set after a successful SSO login.
const SESSION_COOKIE: &str = "tellur_session";

/// Name of the short-lived cookie that binds an OIDC login flow to the browser
/// that initiated it (defends against login-CSRF / session fixation).
const LOGIN_COOKIE: &str = "tellur_login";

/// Hard cap on outstanding OIDC login transactions (anti-flood, in addition to
/// the TTL prune). New `/auth/login` requests are refused past this.
const MAX_OUTSTANDING_LOGINS: u64 = 10_000;

/// Maximum events accepted in a single ingest request.
const MAX_EVENTS_PER_REQUEST: usize = 1000;

/// Default and maximum page size for event listings.
const DEFAULT_PAGE: u32 = 50;
const MAX_PAGE: u32 = 200;

/// Run a blocking store operation off the async worker threads, flattening the
/// join + operation errors into a `ServerError`.
async fn run_blocking<T, F>(f: F) -> Result<T, ServerError>
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
fn ensure_same_org(
    state: &AppState,
    principal: &Principal,
    org_id: &str,
    action: &str,
) -> Result<(), ServerError> {
    ensure_org_role(state, principal, org_id, Role::Viewer, action)
}

/// Verify the caller belongs to the path org *and* meets the required role;
/// audit + reject (403) otherwise.
fn ensure_org_role(
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
fn deny(
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
fn effective_role(
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
fn cookie_value(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
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
fn session_cookie_value(headers: &axum::http::HeaderMap) -> Option<String> {
    cookie_value(headers, SESSION_COOKIE)
}

/// Build the `Set-Cookie` value for a new session.
fn session_cookie(sid: &str, max_age: i64) -> String {
    format!("{SESSION_COOKIE}={sid}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={max_age}")
}

/// Build the `Set-Cookie` value that clears the session (logout).
fn clear_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0")
}

/// Build the `Set-Cookie` value for the login-binding cookie. Scoped to `/auth`
/// so it is only sent to the callback.
fn login_cookie(binding: &str) -> String {
    format!(
        "{LOGIN_COOKIE}={binding}; HttpOnly; Secure; SameSite=Lax; Path=/auth; Max-Age={}",
        oidc::LOGIN_TTL_SECS
    )
}

/// Build the `Set-Cookie` value that clears the login-binding cookie.
fn clear_login_cookie() -> String {
    format!("{LOGIN_COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/auth; Max-Age=0")
}

/// Constant-time-ish equality for short secrets (avoids early-exit timing leak).
fn secret_eq(a: &str, b: &str) -> bool {
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
fn login_expired(created_at: &str) -> bool {
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
fn record_auth_denied(state: &AppState, token_id: Option<&str>) {
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
    // Tenant check first (object + tenant, not just role).
    if org_id != principal.org_id {
        return Err(deny(
            &state,
            &principal,
            &org_id,
            "ingest_denied",
            &format!("role={}", principal.role.as_str()),
        ));
    }

    // Per-member rate limit.
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }

    // Per-repo authorization runs *before* request validation, so an
    // unauthorized write is always denied + audited (a bad batch size must not
    // let an unauthorized caller skip the denial). For an existing repo an
    // additive per-repo grant can elevate an org viewer to contributor; creating
    // a *new* repo always requires the org-baseline contributor role (no grant
    // can exist yet). The repo is resolved without creating it here.
    let existing = state
        .store
        .find_repo(&org_id, &repo)
        .map_err(ServerError::Internal)?;
    match &existing {
        Some(r) => {
            if !effective_role(&state, &principal, &r.id)?.allows(Role::Contributor) {
                return Err(deny(
                    &state,
                    &principal,
                    &org_id,
                    "ingest_denied",
                    &format!("repo={} role={}", r.id, principal.role.as_str()),
                ));
            }
        }
        None => {
            if !principal.role.allows(Role::Contributor) {
                return Err(deny(
                    &state,
                    &principal,
                    &org_id,
                    "ingest_denied",
                    &format!("new_repo role={}", principal.role.as_str()),
                ));
            }
        }
    }

    // Validate batch size only after authorization, and before creating the repo
    // so an empty or oversized request never creates a repo as a side effect.
    if req.events.is_empty() {
        return Err(ServerError::BadRequest("no events provided".to_string()));
    }
    if req.events.len() > MAX_EVENTS_PER_REQUEST {
        return Err(ServerError::BadRequest(format!(
            "too many events: {} (max {MAX_EVENTS_PER_REQUEST})",
            req.events.len()
        )));
    }

    // Authorized + valid: resolve (creating the repo if it did not exist).
    let repo = match existing {
        Some(r) => r,
        None => state
            .store
            .ensure_repo(&org_id, &repo)
            .map_err(ServerError::Internal)?,
    };

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
    state.metrics.add_ingested(ids.len() as u64);

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
///
/// `principal` is extracted before `Query` so authentication/tenant checks run
/// before query-parameter parsing (a bad `?limit` must not turn a 401 into a 400
/// or skip the cross-org denial audit).
pub async fn list_events(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    principal: Principal,
    Query(params): Query<ListEventsParams>,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "list_events")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
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

    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id),
            actor_member_id: Some(principal.member_id.clone()),
            action: "events.read".to_string(),
            detail: format!("repo={} count={}", repo.id, events.len()),
        })
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
    // The org report runs full aggregates; rate-limit it to bound the cost.
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    // Run the heavy aggregate off the async worker so it can't stall the runtime.
    let report = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        move || store.org_report(&org)
    })
    .await?;
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

/// Default/maximum size of the dashboard recent-activity feed.
const DASHBOARD_FEED: u32 = 25;
const DASHBOARD_FEED_MAX: u32 = 100;

/// `GET /v1/orgs/{org}/dashboard` — a single consolidated payload for the web
/// dashboard: the org rollup plus a recent-activity feed. Viewer+ (session
/// cookie or API token); the heavy aggregate runs off the async worker.
pub async fn dashboard(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
    Query(params): Query<DashboardParams>,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "dashboard")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let limit = params
        .limit
        .unwrap_or(DASHBOARD_FEED)
        .clamp(1, DASHBOARD_FEED_MAX);
    let (report, recent) = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        move || {
            let report = store.org_report(&org)?;
            let recent = store.recent_org_events(&org, limit)?;
            Ok((report, recent))
        }
    })
    .await?;
    Ok(Json(json!({
        "schema": "tellur.server.dashboard.v1",
        "org_id": org_id,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "report": report,
        "recent_events": recent,
    })))
}

/// Query for the dashboard feed size.
#[derive(Debug, Deserialize)]
pub struct DashboardParams {
    #[serde(default)]
    pub limit: Option<u32>,
}

// ─── Activity time-series + repo summary (dashboard D1) ───────────────────────

/// Parse a range like `7d`/`30d`/`90d` or a bare day count into a day count,
/// clamped to 1..=365 (default 30).
fn parse_range_days(raw: Option<&str>) -> i64 {
    let parsed = raw
        .map(|s| s.trim().trim_end_matches('d'))
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(30);
    parsed.clamp(1, 365)
}

/// Query for the activity time-series.
#[derive(Debug, Deserialize)]
pub struct ActivityParams {
    #[serde(default)]
    pub range: Option<String>,
    #[serde(default, rename = "group_by")]
    pub group_by: Option<String>,
}

/// `GET /v1/orgs/{org}/activity?range=30d&group_by=type|actor` — daily event
/// counts for the dashboard trend chart (viewer+).
pub async fn activity(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
    Query(params): Query<ActivityParams>,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "activity")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let days = parse_range_days(params.range.as_deref());
    let group = match params.group_by.as_deref() {
        Some("actor") => ActivityGroup::Actor,
        _ => ActivityGroup::Type,
    };
    let group_label = match group {
        ActivityGroup::Actor => "actor",
        ActivityGroup::Type => "type",
    };
    let since = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    let buckets = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        move || store.activity_by_day(&org, &since, group)
    })
    .await?;
    Ok(Json(json!({
        "schema": "tellur.server.activity.v1",
        "org_id": org_id,
        "range_days": days,
        "group_by": group_label,
        "buckets": buckets,
    })))
}

/// `GET /v1/orgs/{org}/repos/{repo}` — single-repo summary: event-log facts plus
/// line-level AI share and review coverage from attribution (viewer+).
pub async fn repo_detail(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "repo.detail")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let repo = state
        .store
        .find_repo(&org_id, &repo)
        .map_err(ServerError::Internal)?
        .ok_or(ServerError::NotFound)?;
    let (facts, attrs) = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        let repo_id = repo.id.clone();
        move || {
            let facts = store.repo_facts(&org, &repo_id)?;
            let attrs = store.list_attributions(&org, &repo_id)?;
            Ok((facts, attrs))
        }
    })
    .await?;
    let stats = review::review_stats(&attrs);
    Ok(Json(json!({
        "schema": "tellur.server.repo.v1",
        "id": repo.id,
        "name": repo.name,
        "event_count": facts.event_count,
        "contributors": facts.contributors,
        "last_activity": facts.last_activity,
        "attributed_files": attrs.len(),
        "lines": {
            "total_attributed": stats.total_attributed_lines,
            "ai": stats.ai_lines,
            "reviewed_ai": stats.reviewed_ai_lines,
        },
        "ai_share": stats.ai_share(),
        "review_coverage": stats.review_coverage(),
    })))
}

// ─── Attribution read + sessions (dashboard D2) ───────────────────────────────

/// Query for the attribution read (optional exact-path filter).
#[derive(Debug, Deserialize)]
pub struct AttributionsQuery {
    #[serde(default)]
    pub path: Option<String>,
}

/// `GET /v1/orgs/{org}/repos/{repo}/attributions?path=` — read stored line-level
/// attribution for a repo (viewer+), powering the file provenance gutter. This
/// is metadata only (ranges + provenance); no source text is stored or served.
pub async fn list_attributions(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    principal: Principal,
    Query(q): Query<AttributionsQuery>,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "attributions.read")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let repo = state
        .store
        .find_repo(&org_id, &repo)
        .map_err(ServerError::Internal)?
        .ok_or(ServerError::NotFound)?;
    let mut files = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        let repo_id = repo.id.clone();
        move || store.list_attributions(&org, &repo_id)
    })
    .await?;
    if let Some(path) = q.path.as_deref() {
        files.retain(|f| f.file_path == path);
    }
    serde_json::to_value(json!({ "repo_id": repo.id, "files": files }))
        .map(Json)
        .map_err(|e| ServerError::Internal(e.into()))
}

/// Query for the sessions list.
#[derive(Debug, Deserialize)]
pub struct SessionsParams {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub range: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// `GET /v1/orgs/{org}/sessions` — sessions (events grouped by `session_id`),
/// newest first, filterable by repo/actor/range (viewer+).
pub async fn list_sessions(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
    Query(params): Query<SessionsParams>,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "sessions.list")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    // Resolve an optional repo filter to its id (unknown repo → 404).
    let repo_id = match params.repo.as_deref() {
        Some(r) => Some(
            state
                .store
                .find_repo(&org_id, r)
                .map_err(ServerError::Internal)?
                .ok_or(ServerError::NotFound)?
                .id,
        ),
        None => None,
    };
    let since = params.range.as_deref().map(|r| {
        (chrono::Utc::now() - chrono::Duration::days(parse_range_days(Some(r)))).to_rfc3339()
    });
    let limit = params.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let sessions = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        let actor = params.actor.clone();
        move || {
            store.list_sessions(
                &org,
                repo_id.as_deref(),
                actor.as_deref(),
                since.as_deref(),
                limit,
            )
        }
    })
    .await?;
    Ok(Json(json!({
        "schema": "tellur.server.sessions.v1",
        "org_id": org_id,
        "sessions": sessions,
    })))
}

/// `GET /v1/orgs/{org}/sessions/{id}` — a session's events, oldest first (for
/// replay). Viewer+, tenant-scoped.
pub async fn session_detail(
    State(state): State<AppState>,
    Path((org_id, session_id)): Path<(String, String)>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "session.detail")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let events = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        let sid = session_id.clone();
        move || store.session_events(&org, &sid, MAX_PAGE)
    })
    .await?;
    if events.is_empty() {
        return Err(ServerError::NotFound);
    }
    Ok(Json(json!({
        "schema": "tellur.server.session.v1",
        "org_id": org_id,
        "session_id": session_id,
        "events": events,
    })))
}

// ─── Central policy distribution ─────────────────────────────────────────────

/// `PUT /v1/orgs/{org}/policies/{name}` — upload a policy YAML doc (admin only).
/// The body is validated as Tellur policy YAML before storage.
pub async fn put_policy(
    State(state): State<AppState>,
    Path((org_id, name)): Path<(String, String)>,
    principal: Principal,
    body: String,
) -> Result<Json<Value>, ServerError> {
    ensure_org_role(&state, &principal, &org_id, Role::Admin, "policy.put")?;
    tellur_core::policy::PolicyEngine::from_yaml_str(&body)
        .map_err(|e| ServerError::BadRequest(format!("invalid policy YAML: {e}")))?;

    let version = state
        .store
        .put_policy(&org_id, &name, &body)
        .map_err(ServerError::Internal)?;
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id),
            actor_member_id: Some(principal.member_id.clone()),
            action: "policy.put".to_string(),
            detail: format!("name={name} version={version}"),
        })
        .map_err(ServerError::Internal)?;
    Ok(Json(json!({ "name": name, "version": version })))
}

/// `GET /v1/orgs/{org}/policies` — list policy metadata.
pub async fn list_policies(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "list_policies")?;
    let policies = state
        .store
        .list_policies(&org_id)
        .map_err(ServerError::Internal)?;
    Ok(Json(json!({ "policies": policies })))
}

/// `GET /v1/orgs/{org}/policies/{name}` — fetch a policy (for `policy pull`).
pub async fn get_policy(
    State(state): State<AppState>,
    Path((org_id, name)): Path<(String, String)>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "policy.pull")?;
    let doc = state
        .store
        .get_policy(&org_id, &name)
        .map_err(ServerError::Internal)?
        .ok_or(ServerError::NotFound)?;
    state.metrics.inc_policy_pull();
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id),
            actor_member_id: Some(principal.member_id.clone()),
            action: "policy.pull".to_string(),
            detail: format!("name={} version={}", doc.name, doc.version),
        })
        .map_err(ServerError::Internal)?;
    serde_json::to_value(&doc)
        .map(Json)
        .map_err(|e| ServerError::Internal(e.into()))
}

// ─── Export portal (durable jobs) ─────────────────────────────────────────────

/// Enqueue an export job (admin) and return `202 Accepted` with a poll URL. The
/// heavy work runs in the background worker so the request returns immediately
/// and large exports can't stall the runtime.
async fn enqueue_export(
    state: &AppState,
    principal: &Principal,
    org_id: &str,
    kind: &str,
    action: &str,
) -> Result<Response, ServerError> {
    ensure_org_role(state, principal, org_id, Role::Admin, action)?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let job_id = state
        .store
        .enqueue_job(org_id, kind)
        .map_err(ServerError::Internal)?;
    state.metrics.inc_export();
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id.to_string()),
            actor_member_id: Some(principal.member_id.clone()),
            action: action.to_string(),
            detail: format!("job={job_id} kind={kind}"),
        })
        .map_err(ServerError::Internal)?;
    let body = json!({
        "job_id": job_id,
        "status": "queued",
        "poll": format!("/v1/orgs/{org_id}/jobs/{job_id}"),
    });
    Ok((StatusCode::ACCEPTED, Json(body)).into_response())
}

/// `POST /v1/orgs/{org}/export/events` — enqueue a full event-bundle export.
pub async fn export_events(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Response, ServerError> {
    enqueue_export(
        &state,
        &principal,
        &org_id,
        crate::jobs::KIND_EXPORT_EVENTS,
        "export.events",
    )
    .await
}

/// `POST /v1/orgs/{org}/export/audit` — enqueue an audit-trail export.
pub async fn export_audit(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Response, ServerError> {
    enqueue_export(
        &state,
        &principal,
        &org_id,
        crate::jobs::KIND_EXPORT_AUDIT,
        "export.audit",
    )
    .await
}

/// `GET /v1/orgs/{org}/jobs/{id}` — poll a job's status; includes the JSON
/// result once `completed` (admin only, since export results carry org data).
pub async fn get_job(
    State(state): State<AppState>,
    Path((org_id, job_id)): Path<(String, String)>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_org_role(&state, &principal, &org_id, Role::Admin, "job.get")?;
    let job = state
        .store
        .get_job(&org_id, &job_id)
        .map_err(ServerError::Internal)?
        .ok_or(ServerError::NotFound)?;
    let mut body = serde_json::to_value(&job).map_err(|e| ServerError::Internal(e.into()))?;
    if job.status == "completed"
        && let Some(result) = job.result
    {
        let parsed: Value = serde_json::from_str(&result)
            .map_err(|e| ServerError::Internal(anyhow::anyhow!("corrupt job result: {e}")))?;
        body["result"] = parsed;
    }
    Ok(Json(body))
}

// ─── Attribution ingest + SLSA/SPDX export ───────────────────────────────────

/// Wire format for attribution ingest.
#[derive(Debug, Deserialize)]
pub struct IngestAttributionsRequest {
    pub attributions: Vec<FileAttribution>,
}

/// `POST /v1/orgs/{org}/repos/{repo}/attributions` — ingest line-level
/// attribution (contributor+). This is what powers SLSA/SPDX export.
pub async fn ingest_attributions(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    principal: Principal,
    Json(req): Json<IngestAttributionsRequest>,
) -> Result<Json<Value>, ServerError> {
    if org_id != principal.org_id {
        return Err(deny(
            &state,
            &principal,
            &org_id,
            "attributions_denied",
            &format!("role={}", principal.role.as_str()),
        ));
    }
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }

    // Per-repo authorization first (mirroring event ingest), so an unauthorized
    // write is always denied + audited regardless of request validity. An
    // existing repo can be written by a per-repo contributor; creating a new
    // repo requires the org-baseline contributor role. Resolved without creating.
    let existing = state
        .store
        .find_repo(&org_id, &repo)
        .map_err(ServerError::Internal)?;
    match &existing {
        Some(r) => {
            if !effective_role(&state, &principal, &r.id)?.allows(Role::Contributor) {
                return Err(deny(
                    &state,
                    &principal,
                    &org_id,
                    "attributions_denied",
                    &format!("repo={} role={}", r.id, principal.role.as_str()),
                ));
            }
        }
        None => {
            if !principal.role.allows(Role::Contributor) {
                return Err(deny(
                    &state,
                    &principal,
                    &org_id,
                    "attributions_denied",
                    &format!("new_repo role={}", principal.role.as_str()),
                ));
            }
        }
    }

    if req.attributions.is_empty() {
        return Err(ServerError::BadRequest(
            "no attributions provided".to_string(),
        ));
    }
    if req.attributions.len() > MAX_EVENTS_PER_REQUEST {
        return Err(ServerError::BadRequest(format!(
            "too many files: {} (max {MAX_EVENTS_PER_REQUEST})",
            req.attributions.len()
        )));
    }
    // Reject malformed line ranges up front: lines are 1-based and start must not
    // exceed end. Otherwise `end_line - start_line + 1` underflows in SPDX/SLSA
    // generation (panic in debug, huge count in release).
    for file in &req.attributions {
        for r in &file.ranges {
            if r.start_line == 0 || r.start_line > r.end_line {
                return Err(ServerError::BadRequest(format!(
                    "invalid range in {}: start_line={} end_line={}",
                    file.file_path, r.start_line, r.end_line
                )));
            }
        }
    }

    // Authorized + valid: resolve (creating the repo if it did not exist).
    let repo = match existing {
        Some(r) => r,
        None => state
            .store
            .ensure_repo(&org_id, &repo)
            .map_err(ServerError::Internal)?,
    };
    let n = state
        .store
        .put_attributions(&org_id, &repo.id, &req.attributions)
        .map_err(ServerError::Internal)?;
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id),
            actor_member_id: Some(principal.member_id.clone()),
            action: "attributions.ingest".to_string(),
            detail: format!("repo={} files={n}", repo.id),
        })
        .map_err(ServerError::Internal)?;
    Ok(Json(json!({ "repo_id": repo.id, "files": n })))
}

/// Query context for compliance exports (subject identity is caller-supplied
/// since the hub stores provenance events/attribution, not Git remotes/commits).
#[derive(Debug, Deserialize)]
pub struct ExportContext {
    #[serde(default)]
    pub repo_url: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
}

/// `GET /v1/orgs/{org}/repos/{repo}/export/slsa` — SLSA v1.0 provenance built
/// from the repo's ingested attribution (admin).
pub async fn export_slsa(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    principal: Principal,
    Query(ctx): Query<ExportContext>,
) -> Result<Json<Value>, ServerError> {
    let (repo, attrs) =
        export_attributions(&state, &principal, &org_id, &repo, "export.slsa").await?;
    let repo_url = ctx
        .repo_url
        .unwrap_or_else(|| format!("tellur:repo/{}", repo.id));
    let commit = ctx.commit.unwrap_or_else(|| "unknown".to_string());
    let slsa = tellur_core::export::generate_slsa_provenance(
        &repo_url,
        &commit,
        &attrs,
        "https://tellur.dev/hub",
    );
    serde_json::to_value(&slsa)
        .map(Json)
        .map_err(|e| ServerError::Internal(e.into()))
}

/// `GET /v1/orgs/{org}/repos/{repo}/export/spdx` — SPDX SBOM with AI attribution
/// built from the repo's ingested attribution (admin).
pub async fn export_spdx(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    principal: Principal,
    Query(ctx): Query<ExportContext>,
) -> Result<Json<Value>, ServerError> {
    let (repo, attrs) =
        export_attributions(&state, &principal, &org_id, &repo, "export.spdx").await?;
    let repo_url = ctx
        .repo_url
        .unwrap_or_else(|| format!("tellur:repo/{}", repo.id));
    let commit = ctx.commit.unwrap_or_else(|| "unknown".to_string());
    let spdx = tellur_core::export::generate_spdx_sbom(&repo.name, &repo_url, &commit, &attrs);
    serde_json::to_value(&spdx)
        .map(Json)
        .map_err(|e| ServerError::Internal(e.into()))
}

/// Shared admin-authz + tenant + rate-limit + fetch for the compliance exports.
async fn export_attributions(
    state: &AppState,
    principal: &Principal,
    org_id: &str,
    repo_name: &str,
    action: &str,
) -> Result<(crate::storage::Repo, Vec<FileAttribution>), ServerError> {
    // Same-org check up front.
    if org_id != principal.org_id {
        return Err(deny(
            state,
            principal,
            org_id,
            "access_denied",
            &format!("{action} role={}", principal.role.as_str()),
        ));
    }
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    // Admin-level authorization, allowing an additive per-repo admin grant. We
    // only disclose a missing repo (404) to callers who are already org admins;
    // anyone else gets a 403 so repo existence is not leaked by status code.
    let repo = match state
        .store
        .find_repo(org_id, repo_name)
        .map_err(ServerError::Internal)?
    {
        Some(r) => {
            if !effective_role(state, principal, &r.id)?.allows(Role::Admin) {
                return Err(deny(
                    state,
                    principal,
                    org_id,
                    "access_denied",
                    &format!("{action} repo={} role={}", r.id, principal.role.as_str()),
                ));
            }
            r
        }
        None => {
            if principal.role.allows(Role::Admin) {
                return Err(ServerError::NotFound);
            }
            return Err(deny(
                state,
                principal,
                org_id,
                "access_denied",
                &format!("{action} repo={repo_name} role={}", principal.role.as_str()),
            ));
        }
    };
    let attrs = run_blocking({
        let store = state.store.clone();
        let org = org_id.to_string();
        let repo_id = repo.id.clone();
        move || store.list_attributions(&org, &repo_id)
    })
    .await?;
    state.metrics.inc_export();
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id.to_string()),
            actor_member_id: Some(principal.member_id.clone()),
            action: action.to_string(),
            detail: format!("repo={} files={}", repo.id, attrs.len()),
        })
        .map_err(ServerError::Internal)?;
    Ok((repo, attrs))
}

// ─── Per-repo role administration (org admin) ────────────────────────────────

/// Wire format for granting a per-repo role.
#[derive(Debug, Deserialize)]
pub struct SetRepoRoleRequest {
    pub role: String,
}

/// Resolve a repo for an org-admin management action, or 403/404.
async fn admin_resolve_repo(
    state: &AppState,
    principal: &Principal,
    org_id: &str,
    repo_ref: &str,
    action: &str,
) -> Result<crate::storage::Repo, ServerError> {
    ensure_org_role(state, principal, org_id, Role::Admin, action)?;
    state
        .store
        .find_repo(org_id, repo_ref)
        .map_err(ServerError::Internal)?
        .ok_or(ServerError::NotFound)
}

/// `PUT /v1/orgs/{org}/repos/{repo}/roles/{member_id}` — grant a member an
/// additive per-repo role (org admin only).
pub async fn set_repo_role(
    State(state): State<AppState>,
    Path((org_id, repo, member_id)): Path<(String, String, String)>,
    principal: Principal,
    Json(req): Json<SetRepoRoleRequest>,
) -> Result<Json<Value>, ServerError> {
    // Authorize (org-admin + tenant) before validating the request body, so a
    // cross-tenant attempt is denied + audited rather than short-circuited by a
    // body 400.
    let repo = admin_resolve_repo(&state, &principal, &org_id, &repo, "repo_role.set").await?;
    let role = Role::parse(&req.role)
        .map_err(|_| ServerError::BadRequest(format!("unknown role: {}", req.role)))?;
    state
        .store
        .set_repo_role(&org_id, &repo.id, &member_id, role)
        .map_err(|e| ServerError::BadRequest(e.to_string()))?;
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id),
            actor_member_id: Some(principal.member_id.clone()),
            action: "repo_role.set".to_string(),
            detail: format!("repo={} member={member_id} role={}", repo.id, role.as_str()),
        })
        .map_err(ServerError::Internal)?;
    Ok(Json(json!({
        "repo_id": repo.id,
        "member_id": member_id,
        "role": role.as_str(),
    })))
}

/// `DELETE /v1/orgs/{org}/repos/{repo}/roles/{member_id}` — revoke a per-repo
/// grant (org admin only). The member keeps their org-baseline role.
pub async fn remove_repo_role(
    State(state): State<AppState>,
    Path((org_id, repo, member_id)): Path<(String, String, String)>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    let repo = admin_resolve_repo(&state, &principal, &org_id, &repo, "repo_role.remove").await?;
    let removed = state
        .store
        .remove_repo_role(&org_id, &repo.id, &member_id)
        .map_err(ServerError::Internal)?;
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id),
            actor_member_id: Some(principal.member_id.clone()),
            action: "repo_role.remove".to_string(),
            detail: format!("repo={} member={member_id} removed={removed}", repo.id),
        })
        .map_err(ServerError::Internal)?;
    Ok(Json(
        json!({ "repo_id": repo.id, "member_id": member_id, "removed": removed }),
    ))
}

/// `GET /v1/orgs/{org}/repos/{repo}/roles` — list per-repo grants (org admin).
pub async fn list_repo_roles(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    let repo = admin_resolve_repo(&state, &principal, &org_id, &repo, "repo_role.list").await?;
    let grants = state
        .store
        .list_repo_roles(&org_id, &repo.id)
        .map_err(ServerError::Internal)?;
    // Grants are the repo-scoped authorization state; enumerating them is itself
    // an auditable read (mirrors repo_role.set / repo_role.remove).
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id),
            actor_member_id: Some(principal.member_id.clone()),
            action: "repo_role.list".to_string(),
            detail: format!("repo={} count={}", repo.id, grants.len()),
        })
        .map_err(ServerError::Internal)?;
    Ok(Json(json!({ "repo_id": repo.id, "grants": grants })))
}

// ─── OIDC SSO (browser login) ────────────────────────────────────────────────

/// `GET /auth/login` — begin the OIDC Authorization Code + PKCE flow. Persists
/// a login transaction (CSRF state → PKCE/nonce) and redirects to the IdP.
pub async fn oidc_login(State(state): State<AppState>) -> Result<Response, ServerError> {
    let oidc = state.oidc.clone().ok_or(ServerError::NotFound)?;
    // Opportunistically prune stale login rows so anonymous /auth/login traffic
    // can't grow the table without bound, then enforce a hard cap.
    state
        .store
        .prune_expired_logins(oidc::LOGIN_TTL_SECS)
        .map_err(ServerError::Internal)?;
    if state.store.count_logins().map_err(ServerError::Internal)? >= MAX_OUTSTANDING_LOGINS {
        return Err(ServerError::TooManyRequests);
    }
    let pkce = Pkce::generate();
    let state_tok = oidc::random_token(24);
    let nonce = oidc::random_token(24);
    // Browser-binding secret: stored with the tx and set as a cookie; the
    // callback must present a matching cookie, so a state value leaked/forwarded
    // to a victim cannot complete the flow in their browser (login-CSRF).
    let binding = oidc::random_token(24);
    state
        .store
        .put_login(&state_tok, &pkce.verifier, &nonce, &binding)
        .map_err(ServerError::Internal)?;
    // Discovery may hit the network → run off the async worker.
    let url = run_blocking({
        let oidc = oidc.clone();
        let state_tok = state_tok.clone();
        let nonce = nonce.clone();
        let challenge = pkce.challenge.clone();
        move || {
            let disc = oidc.discovery()?;
            Ok(oidc::build_authorize_url(
                &disc,
                &oidc.config,
                &state_tok,
                &nonce,
                &challenge,
            ))
        }
    })
    .await?;
    let mut resp = Redirect::to(&url).into_response();
    resp.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&login_cookie(&binding))
            .map_err(|e| ServerError::Internal(e.into()))?,
    );
    Ok(resp)
}

/// Query parameters returned by the IdP to the redirect URI.
#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// `GET /auth/callback` — complete the flow: validate state, exchange the code,
/// validate the ID token, map to a provisioned member, and start a session.
pub async fn oidc_callback(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<CallbackParams>,
) -> Result<Response, ServerError> {
    let oidc = state.oidc.clone().ok_or(ServerError::NotFound)?;
    if let Some(err) = params.error {
        return Err(ServerError::BadRequest(format!(
            "IdP returned error: {err}"
        )));
    }
    let (Some(code), Some(state_tok)) = (params.code, params.state) else {
        return Err(ServerError::BadRequest("missing code or state".to_string()));
    };
    // Consume the login transaction. An unknown state is a CSRF / replay signal.
    let login = state
        .store
        .take_login(&state_tok)
        .map_err(ServerError::Internal)?
        .ok_or_else(|| ServerError::BadRequest("unknown or expired login state".to_string()))?;
    if login_expired(&login.created_at) {
        return Err(ServerError::BadRequest("login state expired".to_string()));
    }
    // The callback must come from the browser that initiated the flow: its
    // login cookie must match the stored binding (defends against login-CSRF /
    // session fixation where a leaked callback URL is opened in a victim's
    // browser).
    let presented = cookie_value(&headers, LOGIN_COOKIE).unwrap_or_default();
    if !secret_eq(&presented, &login.browser_binding) {
        return Err(ServerError::BadRequest(
            "login binding mismatch".to_string(),
        ));
    }
    // Exchange the code for an ID token (network, over TLS).
    let id_token = run_blocking({
        let oidc = oidc.clone();
        let verifier = login.pkce_verifier.clone();
        move || oidc.exchange_code(&code, &verifier)
    })
    .await?;
    // Validate ID-token claims (iss/aud/exp/nonce); signature integrity is
    // provided by the TLS-secured token-endpoint channel (see oidc module docs).
    let now = chrono::Utc::now().timestamp();
    let claims = oidc::parse_and_validate_id_token(
        &id_token,
        &oidc.config.issuer,
        &oidc.config.client_id,
        &login.nonce,
        now,
    )
    .map_err(|_| ServerError::Unauthorized)?;

    let principal = resolve_sso_member(&state, &claims, &oidc.config.issuer)?;
    let sid = state
        .store
        .create_session(&principal.member_id, oidc::SESSION_TTL_SECS)
        .map_err(ServerError::Internal)?;
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(principal.org_id.clone()),
            actor_member_id: Some(principal.member_id.clone()),
            action: "auth.sso_login".to_string(),
            detail: "via=oidc".to_string(),
        })
        .map_err(ServerError::Internal)?;

    let mut resp = Redirect::to("/").into_response();
    let headers = resp.headers_mut();
    headers.append(
        SET_COOKIE,
        HeaderValue::from_str(&session_cookie(&sid, oidc::SESSION_TTL_SECS))
            .map_err(|e| ServerError::Internal(e.into()))?,
    );
    // Clear the now-consumed login-binding cookie.
    headers.append(
        SET_COOKIE,
        HeaderValue::from_str(&clear_login_cookie())
            .map_err(|e| ServerError::Internal(e.into()))?,
    );
    Ok(resp)
}

/// Map validated ID-token claims to a provisioned member. No open
/// self-registration: an unknown identity is rejected (403). On first login the
/// OIDC subject is bound so later logins match by subject even if email changes.
fn resolve_sso_member(
    state: &AppState,
    claims: &oidc::IdClaims,
    issuer: &str,
) -> Result<Principal, ServerError> {
    // Subjects are only unique within an issuer, so the binding is keyed by
    // (issuer, subject).
    if let Some(p) = state
        .store
        .find_member_by_oidc_subject(issuer, &claims.subject)
        .map_err(ServerError::Internal)?
    {
        return Ok(p);
    }
    // Fall back to a *verified* email, then bind the subject for next time.
    let email = claims
        .email
        .as_deref()
        .filter(|_| claims.email_verified)
        .ok_or(ServerError::Forbidden)?;
    match state
        .store
        .find_member_by_email(email)
        .map_err(ServerError::Internal)?
    {
        Some(p) => {
            // Bind the (issuer, subject) on first login only. If the member
            // already has a (different) binding, refuse — a second IdP account
            // on the same email must not take over the member.
            let bound = state
                .store
                .bind_oidc_subject(&p.member_id, issuer, &claims.subject)
                .map_err(ServerError::Internal)?;
            if !bound {
                return Err(ServerError::Forbidden);
            }
            Ok(p)
        }
        None => Err(ServerError::Forbidden),
    }
}

/// `GET /auth/logout` — delete the current session and clear the cookie.
pub async fn oidc_logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Response, ServerError> {
    if let Some(sid) = session_cookie_value(&headers) {
        state
            .store
            .delete_session(&sid)
            .map_err(ServerError::Internal)?;
    }
    let mut resp = Redirect::to("/").into_response();
    resp.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&clear_session_cookie())
            .map_err(|e| ServerError::Internal(e.into()))?,
    );
    Ok(resp)
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
