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
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use tellur_core::redaction::RedactionEngine;
use tellur_core::schema::types::FileAttribution;

use crate::app::AppState;
use crate::auth::{Principal, Role};
use crate::error::ServerError;
use crate::oidc::{self, Pkce};
use crate::review;
use crate::storage::{ActivityGroup, AuditEntry, DevicePoll, IngestEvent};

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

/// Name of the short-lived cookie that remembers where to send the browser after
/// an OIDC login (e.g. back to the device-approval page). Scoped to `/auth`.
const RETURN_COOKIE: &str = "tellur_return";

/// Lifetime of a device-authorization request (`tellur login`). The CLI must be
/// approved within this window or it must restart the flow.
const DEVICE_TTL_SECS: i64 = 15 * 60;

/// Suggested poll interval (seconds) handed to the `tellur login` client.
const DEVICE_POLL_INTERVAL_SECS: i64 = 5;

/// Hard cap on outstanding device-authorization requests (anti-flood).
const MAX_OUTSTANDING_DEVICE: u64 = 10_000;

/// Default and maximum page size for event listings.
const DEFAULT_PAGE: u32 = 50;
const MAX_PAGE: u32 = 200;

/// Cap on events returned for a single session replay. Higher than a list page
/// (a replay wants the whole session), but still bounded; the response notes
/// when it was hit so the UI can say the replay is truncated.
const SESSION_REPLAY_LIMIT: u32 = 5000;

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

/// `GET /v1/orgs/{org}/overview` — the landing screen in one round-trip (A9,
/// viewer+): org totals, org-wide AI share + review coverage, a 30-day activity
/// series, repos ranked by review gap (most unreviewed AI lines first), and a
/// recent-activity feed. Heavier than `/dashboard` (it folds in attribution), so
/// it is rate-limited and computed off the async runtime.
pub async fn overview(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "overview")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let since = (chrono::Utc::now() - chrono::Duration::days(30)).to_rfc3339();
    let (report, recent, activity, repo_stats, totals) = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        move || {
            let report = store.org_report(&org)?;
            let recent = store.recent_org_events(&org, DASHBOARD_FEED)?;
            let activity = store.activity_by_day(&org, &since, ActivityGroup::Type)?;
            // Per-repo review stats + an org-wide rollup.
            let mut repo_stats = Vec::with_capacity(report.repos.len());
            let mut total = review::ReviewStats::default();
            for repo in &report.repos {
                let attrs = store.list_attributions(&org, &repo.id)?;
                let s = review::review_stats(&attrs);
                total.total_attributed_lines += s.total_attributed_lines;
                total.ai_lines += s.ai_lines;
                total.reviewed_ai_lines += s.reviewed_ai_lines;
                repo_stats.push((repo.clone(), s));
            }
            Ok((report, recent, activity, repo_stats, total))
        }
    })
    .await?;

    // Rank repos by absolute review gap (unreviewed AI lines), then AI volume.
    let mut repos: Vec<Value> = repo_stats
        .iter()
        .map(|(repo, s)| {
            let gap = s.ai_lines.saturating_sub(s.reviewed_ai_lines);
            json!({
                "id": repo.id,
                "name": repo.name,
                "event_count": repo.event_count,
                "ai_lines": s.ai_lines,
                "reviewed_ai_lines": s.reviewed_ai_lines,
                "review_gap_lines": gap,
                "ai_share": s.ai_share(),
                "review_coverage": s.review_coverage(),
            })
        })
        .collect();
    repos.sort_by(|a, b| {
        let gap = |v: &Value| v["review_gap_lines"].as_u64().unwrap_or(0);
        let ai = |v: &Value| v["ai_lines"].as_u64().unwrap_or(0);
        gap(b).cmp(&gap(a)).then(ai(b).cmp(&ai(a)))
    });

    Ok(Json(json!({
        "schema": "tellur.server.overview.v1",
        "org_id": org_id,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "totals": {
            "events": report.total_events,
            "sessions": report.distinct_sessions,
            "repos": report.repos.len(),
            "ai_lines": totals.ai_lines,
            "reviewed_ai_lines": totals.reviewed_ai_lines,
            "total_attributed_lines": totals.total_attributed_lines,
        },
        "ai_share": totals.ai_share(),
        "review_coverage": totals.review_coverage(),
        "activity": activity,
        "repos": repos,
        "recent_events": recent,
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
    let (mut files, source) = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        let repo_id = repo.id.clone();
        move || {
            let files = store.list_attributions(&org, &repo_id)?;
            let source = store.get_repo_source(&org, &repo_id)?;
            Ok((files, source))
        }
    })
    .await?;
    if let Some(path) = q.path.as_deref() {
        files.retain(|f| f.file_path == path);
    }
    // Opt-in source URL templates (A12): `source_template` deep-links the
    // provider's web view; `source_raw_template` points at raw bytes the browser
    // fetches to render the inline source gutter. The hub stores/serves no source.
    serde_json::to_value(json!({
        "repo_id": repo.id,
        "files": files,
        "source_template": source.link,
        "source_raw_template": source.raw,
        // When a proxy token is configured the repo is private: the browser must
        // fetch raw bytes through the hub's blob endpoint, not direct.
        "source_proxy": source.token.is_some(),
    }))
    .map(Json)
    .map_err(|e| ServerError::Internal(e.into()))
}

/// Body for setting a repo's source connection (A12).
#[derive(Debug, Deserialize)]
pub struct SourceTemplateBody {
    /// Provider web-view template, `https://…` with `{path}`/`{start}`/`{end}`
    /// placeholders. `null`/absent clears it.
    #[serde(default)]
    pub template: Option<String>,
    /// Raw-bytes template (e.g. `raw.githubusercontent.com/...`), `https://…`
    /// with a `{path}` placeholder, for the inline source gutter. `null`/absent
    /// clears it. Only templates are stored — never source code.
    #[serde(default)]
    pub raw_template: Option<String>,
    /// Provider access token for the private-repo proxy. When non-empty it is
    /// stored; absent/empty preserves the existing token (so editing templates
    /// doesn't require re-entering the secret). Set `clear_token` to remove it.
    #[serde(default)]
    pub token: Option<String>,
    /// Remove any stored provider token.
    #[serde(default)]
    pub clear_token: bool,
}

/// `PUT /v1/orgs/{org}/repos/{repo}/source` — set or clear the opt-in source
/// connection (admin only, A12). Each template must be an `https://` URL so the
/// file view can safely render the link / fetch raw bytes. The optional token
/// (for the private-repo proxy) is stored but never returned.
pub async fn set_repo_source(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    principal: Principal,
    Json(body): Json<SourceTemplateBody>,
) -> Result<Json<Value>, ServerError> {
    ensure_org_role(&state, &principal, &org_id, Role::Admin, "repo.source.set")?;
    let repo = state
        .store
        .find_repo(&org_id, &repo)
        .map_err(ServerError::Internal)?
        .ok_or(ServerError::NotFound)?;
    // Only https templates — guards against javascript:/data: hrefs (XSS) and
    // plaintext fetches; length-bounded to a sane URL template.
    let validate = |raw: &Option<String>| -> Result<Option<String>, ServerError> {
        match raw.as_deref().map(str::trim) {
            Some("") | None => Ok(None),
            Some(t) => {
                if !t.starts_with("https://") || t.len() > 2048 {
                    return Err(ServerError::BadRequest(
                        "source templates must be https:// URLs under 2048 chars".into(),
                    ));
                }
                Ok(Some(t.to_string()))
            }
        }
    };
    let link = validate(&body.template)?;
    let raw = validate(&body.raw_template)?;
    // Token semantics: clear › set (non-empty) › preserve existing.
    let token_state;
    let token: Option<String> = if body.clear_token {
        token_state = "cleared";
        None
    } else if let Some(t) = body
        .token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        if t.len() > 4096 {
            return Err(ServerError::BadRequest("token too long".into()));
        }
        token_state = "set";
        Some(t.to_string())
    } else {
        token_state = "unchanged";
        state
            .store
            .get_repo_source(&org_id, &repo.id)
            .map_err(ServerError::Internal)?
            .token
    };
    state
        .store
        .set_repo_source(
            &org_id,
            &repo.id,
            link.as_deref(),
            raw.as_deref(),
            token.as_deref(),
        )
        .map_err(ServerError::Internal)?;
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id),
            actor_member_id: Some(principal.member_id.clone()),
            action: "repo.source.set".to_string(),
            // Never log the token value — only its state transition.
            detail: format!(
                "repo={} link={} raw={} token={token_state}",
                repo.id,
                if link.is_some() { "set" } else { "cleared" },
                if raw.is_some() { "set" } else { "cleared" }
            ),
        })
        .map_err(ServerError::Internal)?;
    Ok(Json(json!({
        "repo_id": repo.id,
        "source_template": link,
        "source_raw_template": raw,
        "token_configured": token.is_some(),
    })))
}

/// `GET /v1/orgs/{org}/repos/{repo}/source` — read a repo's source connection for
/// the admin settings form (admin only). Returns the templates and whether a
/// proxy token is configured — never the token itself.
pub async fn get_repo_source(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_org_role(&state, &principal, &org_id, Role::Admin, "repo.source.get")?;
    let repo = state
        .store
        .find_repo(&org_id, &repo)
        .map_err(ServerError::Internal)?
        .ok_or(ServerError::NotFound)?;
    let source = state
        .store
        .get_repo_source(&org_id, &repo.id)
        .map_err(ServerError::Internal)?;
    Ok(Json(json!({
        "repo_id": repo.id,
        "source_template": source.link,
        "source_raw_template": source.raw,
        "token_configured": source.token.is_some(),
    })))
}

/// Query for the source blob proxy.
#[derive(Debug, Deserialize)]
pub struct BlobQuery {
    pub path: String,
}

/// `GET /v1/orgs/{org}/repos/{repo}/blob?path=` — proxy raw file bytes from the
/// repo's configured provider (viewer+, A12). For **private** repos whose source
/// the browser can't fetch cross-origin: the hub fetches with the stored token,
/// SSRF-guarded against a host allowlist, size-capped. The token never leaves the
/// hub; the bytes are the org's own source returned to org members.
pub async fn source_blob(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    principal: Principal,
    Query(q): Query<BlobQuery>,
) -> Result<Json<Value>, ServerError> {
    ensure_same_org(&state, &principal, &org_id, "repo.source.blob")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let repo = state
        .store
        .find_repo(&org_id, &repo)
        .map_err(ServerError::Internal)?
        .ok_or(ServerError::NotFound)?;
    let source = state
        .store
        .get_repo_source(&org_id, &repo.id)
        .map_err(ServerError::Internal)?;
    let raw_template = source
        .raw
        .ok_or_else(|| ServerError::BadRequest("no source raw template configured".into()))?;
    // A configured GitHub Enterprise host is additionally allowlisted + recognised.
    let enterprise_host = state.github_app.as_ref().and_then(|a| a.config.api_host());
    // Build + allowlist-check the URL before any network call (SSRF guard).
    let url = crate::source::resolve_raw_url(&raw_template, &q.path, enterprise_host.as_deref())
        .map_err(|e| ServerError::BadRequest(e.to_string()))?;
    // Token minting signs a JWT + makes synchronous GitHub calls on the uncached
    // path, so resolve it inside the blocking closure alongside the fetch (never
    // block a Tokio worker on a slow GitHub response).
    let state = state.clone();
    let stored = source.token.clone();
    let content = run_blocking(move || {
        let token = resolve_source_token(&state, &raw_template, stored);
        crate::source::fetch_blob(&url, token.as_deref(), enterprise_host.as_deref())
    })
    .await?;
    Ok(Json(json!({ "path": q.path, "content": content })))
}

/// Choose the source-proxy auth token. For a GitHub repo with the App configured,
/// mint a short-lived installation token (replacing the stored PAT); for any other
/// provider, or if minting fails, fall back to the stored PAT (`stored`).
pub fn resolve_source_token(
    state: &AppState,
    raw_template: &str,
    stored: Option<String>,
) -> Option<String> {
    let Some(app) = &state.github_app else {
        return stored;
    };
    let enterprise_host = app.config.api_host();
    let Some((owner, repo)) =
        crate::github_app::github_owner_repo(raw_template, enterprise_host.as_deref())
    else {
        return stored; // non-GitHub provider — PAT fallback
    };
    match app.token_for(&owner, &repo) {
        Ok(token) => Some(token),
        Err(e) => {
            tracing::warn!(error = %e, "GitHub App token mint failed; falling back to stored token");
            stored
        }
    }
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
    let mut events = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        let sid = session_id.clone();
        // Fetch one past the cap so we can flag truncation honestly.
        move || store.session_events(&org, &sid, SESSION_REPLAY_LIMIT + 1)
    })
    .await?;
    if events.is_empty() {
        return Err(ServerError::NotFound);
    }
    let truncated = events.len() as u32 > SESSION_REPLAY_LIMIT;
    if truncated {
        events.truncate(SESSION_REPLAY_LIMIT as usize);
    }
    Ok(Json(json!({
        "schema": "tellur.server.session.v1",
        "org_id": org_id,
        "session_id": session_id,
        "truncated": truncated,
        "events": events,
    })))
}

// ─── Audit read + jobs list (dashboard D3) ───────────────────────────────────

/// Query for the audit read.
#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub range: Option<String>,
    /// Cursor: return rows with `seq` strictly below this (keyset pagination).
    #[serde(default)]
    pub before: Option<i64>,
    #[serde(default)]
    pub limit: Option<u32>,
}

/// `GET /v1/orgs/{org}/audit` — paginated, filterable read of the org's
/// tamper-evident audit log, newest first (admin only — audit detail can name
/// members and actions). Keyset paginate with `before=<seq>`.
///
/// On the first page (no `before` cursor) the response also carries
/// `chain_intact`: whether the global hash chain still verifies. That check is
/// O(n) over the whole audit log, so it is computed only when a cursor is absent.
pub async fn list_audit(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Value>, ServerError> {
    ensure_org_role(&state, &principal, &org_id, Role::Admin, "audit.read")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let since = q.range.as_deref().map(|r| {
        (chrono::Utc::now() - chrono::Duration::days(parse_range_days(Some(r)))).to_rfc3339()
    });
    let limit = q.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let verify = q.before.is_none();
    let (records, chain_intact) = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        let actor = q.actor.clone();
        let action = q.action.clone();
        move || {
            let records = store.list_audit(
                &org,
                actor.as_deref(),
                action.as_deref(),
                since.as_deref(),
                q.before,
                limit,
            )?;
            let chain_intact = if verify {
                Some(store.verify_audit_chain()?)
            } else {
                None
            };
            Ok((records, chain_intact))
        }
    })
    .await?;
    // Next cursor is the oldest seq in this page (rows are newest-first).
    let next = if records.len() as u32 == limit {
        records.last().map(|r| r.seq)
    } else {
        None
    };
    Ok(Json(json!({
        "schema": "tellur.server.audit.v1",
        "org_id": org_id,
        "chain_intact": chain_intact,
        "next_before": next,
        "records": records,
    })))
}

/// Generic `?limit=` page query.
#[derive(Debug, Deserialize)]
pub struct PageQuery {
    #[serde(default)]
    pub limit: Option<u32>,
}

/// `GET /v1/orgs/{org}/jobs` — list the org's durable jobs, newest first, for the
/// Exports history table (admin only — results carry org data). Results are not
/// inlined here; poll `GET /v1/orgs/{org}/jobs/{id}` for a completed job's output.
pub async fn list_jobs(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
    Query(params): Query<PageQuery>,
) -> Result<Json<Value>, ServerError> {
    ensure_org_role(&state, &principal, &org_id, Role::Admin, "jobs.list")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let limit = params.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let jobs = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        move || store.list_jobs(&org, limit)
    })
    .await?;
    Ok(Json(json!({
        "schema": "tellur.server.jobs.v1",
        "org_id": org_id,
        "jobs": jobs,
    })))
}

// ─── Policy compliance + People & Access (dashboard D4) ──────────────────────

/// `POST /v1/orgs/{org}/policies/compliance` — enqueue a durable job that
/// evaluates the org's `default` policy against every repo's attribution and
/// persists timestamped snapshots (A8, admin only). Returns `202` + job id.
pub async fn enqueue_compliance(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Response, ServerError> {
    ensure_org_role(
        &state,
        &principal,
        &org_id,
        Role::Admin,
        "compliance.enqueue",
    )?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let job_id = state
        .store
        .enqueue_job(&org_id, crate::jobs::KIND_COMPLIANCE, None)
        .map_err(ServerError::Internal)?;
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id.clone()),
            actor_member_id: Some(principal.member_id.clone()),
            action: "compliance.enqueue".to_string(),
            detail: format!("job={job_id}"),
        })
        .map_err(ServerError::Internal)?;
    let body = json!({
        "job_id": job_id,
        "status": "queued",
        "poll": format!("/v1/orgs/{org_id}/jobs/{job_id}"),
    });
    Ok((StatusCode::ACCEPTED, Json(body)).into_response())
}

/// `GET /v1/orgs/{org}/policies/compliance` — latest compliance snapshot per
/// repo (A8, admin only). `evaluated` is false until the first job has run.
pub async fn get_compliance(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_org_role(&state, &principal, &org_id, Role::Admin, "compliance.read")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let snapshots = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        move || store.latest_compliance(&org)
    })
    .await?;
    Ok(Json(json!({
        "schema": "tellur.server.compliance.v1",
        "org_id": org_id,
        "evaluated": !snapshots.is_empty(),
        "snapshots": snapshots,
    })))
}

/// `GET /v1/orgs/{org}/members` — org members with role, email, SSO-bound and
/// active flags (A2, admin only). Session-auth read for the People screen.
pub async fn list_members(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_org_role(&state, &principal, &org_id, Role::Admin, "members.list")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let members = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        move || store.list_members(&org)
    })
    .await?;
    Ok(Json(json!({
        "schema": "tellur.server.members.v1",
        "org_id": org_id,
        "members": members,
    })))
}

/// `GET /v1/orgs/{org}/groups` — SCIM groups with members and the org role each
/// `displayName` maps to (A11, admin only). This is the **session-auth** mirror
/// of `/scim/v2/Groups` so the browser SPA never needs a SCIM bearer token.
pub async fn list_groups(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_org_role(&state, &principal, &org_id, Role::Admin, "groups.list")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let groups = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        move || store.scim_list_groups(&org, None)
    })
    .await?;
    let groups: Vec<Value> = groups
        .into_iter()
        .map(|g| {
            let role = crate::storage::role_from_group_name(&g.display_name);
            json!({
                "id": g.id,
                "display_name": g.display_name,
                "external_id": g.external_id,
                "members": g.members,
                "maps_to_role": role.map(|r| r.as_str()),
            })
        })
        .collect();
    Ok(Json(json!({
        "schema": "tellur.server.groups.v1",
        "org_id": org_id,
        "groups": groups,
    })))
}

/// `GET /v1/orgs/{org}/sso-status` — read-only SSO/SCIM health for the People &
/// Access screen (A10, admin only). Reports configuration and freshness; **no
/// secrets** (no client secret, no token material).
pub async fn sso_status(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_org_role(&state, &principal, &org_id, Role::Admin, "sso.status")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let issuer = state.oidc.as_ref().map(|r| r.config.issuer.clone());
    let (members, scim_created_at, groups) = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        move || {
            let members = store.list_members(&org)?;
            let scim = store.scim_token_created_at(&org)?;
            let groups = store.scim_list_groups(&org, None)?;
            Ok((members, scim, groups))
        }
    })
    .await?;
    let sso_bound = members.iter().filter(|m| m.sso_bound).count();
    let active = members.iter().filter(|m| m.active).count();
    Ok(Json(json!({
        "schema": "tellur.server.sso_status.v1",
        "org_id": org_id,
        "oidc_enabled": issuer.is_some(),
        "oidc_issuer": issuer,
        "scim_configured": scim_created_at.is_some(),
        "scim_token_created_at": scim_created_at,
        "members_total": members.len(),
        "members_active": active,
        "members_sso_bound": sso_bound,
        "scim_groups": groups.len(),
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
        .enqueue_job(org_id, kind, None)
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

/// `POST /v1/orgs/{org}/export/evidence` — enqueue an org-wide evidence pack:
/// every repo's SLSA provenance + the latest compliance snapshot + the audit
/// chain's verification state, in one downloadable job result (admin only).
pub async fn export_evidence(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Response, ServerError> {
    enqueue_export(
        &state,
        &principal,
        &org_id,
        crate::jobs::KIND_EXPORT_EVIDENCE,
        "export.evidence",
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

/// `POST /v1/orgs/{org}/repos/{repo}/export/slsa` — enqueue the SLSA export as a
/// durable job (A13); for large repos the synchronous `GET` can be slow. Org
/// admin only — unlike the synchronous `GET` (which allows a per-repo admin),
/// the result is read back through `GET .../jobs/{id}`, which is org-admin-scoped,
/// so the enqueuer must be an org admin to retrieve it.
pub async fn export_slsa_job(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    principal: Principal,
    Query(ctx): Query<ExportContext>,
) -> Result<Response, ServerError> {
    enqueue_repo_export(
        &state,
        &principal,
        &org_id,
        &repo,
        &ctx,
        crate::jobs::KIND_EXPORT_SLSA,
        "export.slsa.job",
    )
    .await
}

/// `POST /v1/orgs/{org}/repos/{repo}/export/spdx` — enqueue the SPDX export as a
/// durable job (A13). See [`export_slsa_job`].
pub async fn export_spdx_job(
    State(state): State<AppState>,
    Path((org_id, repo)): Path<(String, String)>,
    principal: Principal,
    Query(ctx): Query<ExportContext>,
) -> Result<Response, ServerError> {
    enqueue_repo_export(
        &state,
        &principal,
        &org_id,
        &repo,
        &ctx,
        crate::jobs::KIND_EXPORT_SPDX,
        "export.spdx.job",
    )
    .await
}

/// Authorize (**org admin**) and enqueue a per-repo export job carrying the repo
/// id + optional build context as params. Org admin — not a per-repo grant —
/// because the result is polled via the org-admin-scoped `GET .../jobs/{id}`; a
/// per-repo admin could enqueue but never read it. A missing repo is disclosed
/// as 404 only after the admin check, so existence is not leaked to non-admins.
async fn enqueue_repo_export(
    state: &AppState,
    principal: &Principal,
    org_id: &str,
    repo_name: &str,
    ctx: &ExportContext,
    kind: &str,
    action: &str,
) -> Result<Response, ServerError> {
    ensure_org_role(state, principal, org_id, Role::Admin, action)?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let repo = state
        .store
        .find_repo(org_id, repo_name)
        .map_err(ServerError::Internal)?
        .ok_or(ServerError::NotFound)?;
    let params = json!({
        "repo_id": repo.id,
        "repo_url": ctx.repo_url,
        "commit": ctx.commit,
    })
    .to_string();
    let job_id = state
        .store
        .enqueue_job(org_id, kind, Some(&params))
        .map_err(ServerError::Internal)?;
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id.to_string()),
            actor_member_id: Some(principal.member_id.clone()),
            action: action.to_string(),
            detail: format!("repo={} job={job_id}", repo.id),
        })
        .map_err(ServerError::Internal)?;
    let body = json!({
        "job_id": job_id,
        "status": "queued",
        "poll": format!("/v1/orgs/{org_id}/jobs/{job_id}"),
    });
    Ok((StatusCode::ACCEPTED, Json(body)).into_response())
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

    // Honor a validated return path (e.g. the device-approval page that bounced
    // the user through login), else land on the app root. Only same-origin local
    // paths are allowed, so a forged cookie cannot redirect off-site.
    let dest = cookie_value(&headers, RETURN_COOKIE)
        .and_then(|p| safe_return_path(&p))
        .unwrap_or_else(|| "/".to_string());
    let mut resp = Redirect::to(&dest).into_response();
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
    // Clear the now-consumed return cookie.
    headers.append(
        SET_COOKIE,
        HeaderValue::from_str(&clear_return_cookie())
            .map_err(|e| ServerError::Internal(e.into()))?,
    );
    Ok(resp)
}

/// Validate a post-login return target: a same-origin absolute path only. Rejects
/// scheme-relative (`//host`) and absolute URLs so the redirect can't leave the
/// site, and rejects control characters (header/redirect splitting).
fn safe_return_path(raw: &str) -> Option<String> {
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
fn return_cookie(path: &str) -> String {
    format!(
        "{RETURN_COOKIE}={path}; HttpOnly; Secure; SameSite=Lax; Path=/auth; Max-Age={}",
        oidc::LOGIN_TTL_SECS
    )
}

/// Build the `Set-Cookie` value that clears the return cookie.
fn clear_return_cookie() -> String {
    format!("{RETURN_COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path=/auth; Max-Age=0")
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

// ─── Device authorization (CLI `tellur login`, RFC 8628-style) ───────────────

/// `POST /v1/device/authorize` — begin a CLI login. Issues a secret
/// `device_code` (polled by the CLI) and a short `user_code` (typed by the human
/// in the browser). No auth: the request is only a claim ticket; nothing is
/// granted until a signed-in member approves the `user_code`.
pub async fn device_authorize(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Value>, ServerError> {
    // Device login rides on the same SSO machinery; if SSO is off there is no
    // identity to approve with, so the flow is unavailable.
    if state.oidc.is_none() {
        return Err(ServerError::NotFound);
    }
    // Prune stale rows, then enforce a hard cap so anonymous traffic can't grow
    // the table without bound (mirrors the OIDC-login anti-flood).
    state
        .store
        .prune_expired_device_auths(DEVICE_TTL_SECS)
        .map_err(ServerError::Internal)?;
    if state
        .store
        .count_device_auths()
        .map_err(ServerError::Internal)?
        >= MAX_OUTSTANDING_DEVICE
    {
        return Err(ServerError::TooManyRequests);
    }
    let device_code = oidc::random_token(32);
    let user_code = oidc::random_user_code();
    state
        .store
        .create_device_auth(&device_code, &user_code)
        .map_err(ServerError::Internal)?;
    let base = request_base(&headers);
    Ok(Json(json!({
        "device_code": device_code,
        "user_code": user_code,
        "verification_uri": format!("{base}/auth/device"),
        "verification_uri_complete": format!("{base}/auth/device?user_code={user_code}"),
        "expires_in": DEVICE_TTL_SECS,
        "interval": DEVICE_POLL_INTERVAL_SECS,
    })))
}

/// Body of a device-token poll.
#[derive(Debug, Deserialize)]
pub struct DeviceTokenRequest {
    pub device_code: String,
}

/// `POST /v1/device/token` — the CLI polls here until the request is approved.
/// Mirrors RFC 8628 error codes: `authorization_pending`, `access_denied`,
/// `expired_token`. On approval the hub mints a member API token and returns it
/// once (the row is consumed), reflecting the member's role at approval time.
pub async fn device_token(
    State(state): State<AppState>,
    Json(req): Json<DeviceTokenRequest>,
) -> Result<Response, ServerError> {
    state
        .store
        .prune_expired_device_auths(DEVICE_TTL_SECS)
        .map_err(ServerError::Internal)?;
    match state
        .store
        .poll_device(&req.device_code, DEVICE_TTL_SECS)
        .map_err(ServerError::Internal)?
    {
        DevicePoll::Pending => Ok(device_pending("authorization_pending")),
        DevicePoll::Denied => Ok(device_pending("access_denied")),
        DevicePoll::NotFound => Ok(device_pending("expired_token")),
        DevicePoll::Approved(member_id) => {
            // Resolve the member fresh: a member deactivated between approval and
            // poll must not receive a working token.
            let principal = state
                .store
                .member_principal(&member_id)
                .map_err(ServerError::Internal)?
                .ok_or(ServerError::Forbidden)?;
            let token = state
                .store
                .create_token(&principal.member_id)
                .map_err(ServerError::Internal)?;
            state
                .store
                .append_audit(&AuditEntry {
                    org_id: Some(principal.org_id.clone()),
                    actor_member_id: Some(principal.member_id.clone()),
                    action: "auth.device_login".to_string(),
                    detail: "via=device_code".to_string(),
                })
                .map_err(ServerError::Internal)?;
            Ok(Json(json!({
                "access_token": token.plaintext,
                "token_type": "Bearer",
                "org_id": principal.org_id,
                "member_id": principal.member_id,
                "role": principal.role.as_str(),
            }))
            .into_response())
        }
    }
}

/// A still-pending / terminal-but-not-approved device poll: 400 + an `error`
/// code the CLI switches on (RFC 8628 grant-error shape).
fn device_pending(error: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response()
}

/// Query for the device-approval page.
#[derive(Debug, Deserialize)]
pub struct DeviceVerifyParams {
    #[serde(default)]
    pub user_code: Option<String>,
}

/// `GET /auth/device` — the human-facing approval page. Requires a signed-in
/// session; an unauthenticated visitor is sent through SSO and bounced back here
/// (return cookie). Renders a small confirmation form pre-filled with the code.
pub async fn device_page(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<DeviceVerifyParams>,
) -> Result<Response, ServerError> {
    let user_code = params.user_code.unwrap_or_default();
    // Require a session; otherwise start SSO and remember to return here.
    let principal = match current_session_principal(&state, &headers)? {
        Some(p) => p,
        None => {
            let return_to = format!("/auth/device?user_code={}", percent_encode(&user_code));
            let mut resp = Redirect::to("/auth/login").into_response();
            resp.headers_mut().insert(
                SET_COOKIE,
                HeaderValue::from_str(&return_cookie(&return_to))
                    .map_err(|e| ServerError::Internal(e.into()))?,
            );
            return Ok(resp);
        }
    };
    let known = state
        .store
        .find_device_by_user_code(&user_code)
        .map_err(ServerError::Internal)?
        .filter(|d| d.status == "pending");
    Ok(Html(render_device_page(&user_code, &principal, known.is_some())).into_response())
}

/// Form body of a device-approval decision.
#[derive(Debug, Deserialize)]
pub struct DeviceDecisionForm {
    pub user_code: String,
    /// `approve` or `deny`.
    pub decision: String,
}

/// `POST /auth/device/decision` — record the signed-in member's approve/deny for
/// a device request. Requires a session (a cross-site POST can't carry the
/// SameSite=Lax session cookie, so this is the CSRF defense).
pub async fn device_decision(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Form(form): axum::extract::Form<DeviceDecisionForm>,
) -> Result<Response, ServerError> {
    let principal =
        current_session_principal(&state, &headers)?.ok_or(ServerError::Unauthorized)?;
    let approve = form.decision == "approve";
    let updated = state
        .store
        .set_device_decision(&form.user_code, &principal.member_id, approve)
        .map_err(ServerError::Internal)?;
    if updated {
        state
            .store
            .append_audit(&AuditEntry {
                org_id: Some(principal.org_id.clone()),
                actor_member_id: Some(principal.member_id.clone()),
                action: "auth.device_decision".to_string(),
                detail: format!("decision={}", if approve { "approve" } else { "deny" }),
            })
            .map_err(ServerError::Internal)?;
    }
    let outcome = if !updated {
        DeviceOutcome::Unknown
    } else if approve {
        DeviceOutcome::Approved
    } else {
        DeviceOutcome::Denied
    };
    Ok(Html(render_device_result(outcome)).into_response())
}

/// Resolve the caller's session cookie to a principal, if any.
fn current_session_principal(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<Option<Principal>, ServerError> {
    let Some(sid) = session_cookie_value(headers) else {
        return Ok(None);
    };
    state
        .store
        .session_principal(&sid)
        .map_err(ServerError::Internal)
}

/// Best-effort public base URL (`scheme://host`) from request headers, for the
/// informational `verification_uri`. The CLI also builds this from its own hub
/// URL, so a spoofed `Host` only affects the displayed link, never authorization.
fn request_base(headers: &axum::http::HeaderMap) -> String {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let loopback = host.starts_with("localhost")
                || host.starts_with("127.0.0.1")
                || host.starts_with("[::1]");
            if loopback {
                "http".into()
            } else {
                "https".into()
            }
        });
    format!("{scheme}://{host}")
}

/// Minimal percent-encoding for a path query value (the user_code alphabet is
/// already URL-safe, but encode defensively).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Escape text for safe interpolation into HTML element content/attributes.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Result of a device-approval decision, for the confirmation page.
enum DeviceOutcome {
    Approved,
    Denied,
    Unknown,
}

/// Shared chrome for the small device-flow pages (self-contained, no SPA).
fn device_shell(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{title} · Tellur</title>\
<style>\
:root{{color-scheme:light dark}}\
*{{box-sizing:border-box}}\
body{{margin:0;min-height:100vh;display:grid;place-items:center;\
font:15px/1.5 system-ui,-apple-system,Segoe UI,Roboto,sans-serif;\
background:#f6f7f9;color:#1b1f24}}\
@media(prefers-color-scheme:dark){{body{{background:#0e1116;color:#e6e8eb}}\
.card{{background:#161b22;border-color:#30363d}}}}\
.card{{background:#fff;border:1px solid #e2e5e9;border-radius:14px;\
padding:32px;max-width:420px;width:calc(100% - 32px);\
box-shadow:0 8px 30px rgba(0,0,0,.08)}}\
h1{{font-size:18px;margin:0 0 6px}}\
p{{margin:8px 0;color:#5b6470}}\
@media(prefers-color-scheme:dark){{p{{color:#9aa4b2}}}}\
.code{{font:600 28px/1 ui-monospace,SFMono-Regular,Menlo,monospace;\
letter-spacing:3px;text-align:center;padding:16px;margin:16px 0;\
border-radius:10px;background:#eef1f4}}\
@media(prefers-color-scheme:dark){{.code{{background:#21262d}}}}\
.row{{display:flex;gap:10px;margin-top:20px}}\
button{{flex:1;padding:11px 14px;border-radius:9px;border:1px solid transparent;\
font:inherit;font-weight:600;cursor:pointer}}\
.approve{{background:#1f6feb;color:#fff}}\
.deny{{background:transparent;border-color:#d0d7de;color:inherit}}\
.muted{{font-size:13px;color:#8a929c}}\
.brand{{font-weight:700;letter-spacing:.5px;margin-bottom:18px;font-size:13px;\
text-transform:uppercase;color:#8a929c}}\
</style></head><body><div class=\"card\"><div class=\"brand\">Tellur</div>{body}</div></body></html>"
    )
}

/// Render the approval page for a (possibly unknown) user_code.
fn render_device_page(user_code: &str, principal: &Principal, known: bool) -> String {
    let safe_code = html_escape(user_code);
    if !known {
        let body = format!(
            "<h1>Code not found</h1>\
<p>The code <strong>{safe_code}</strong> is unknown or has expired. \
Start the login again from your terminal with <code>tellur login</code>.</p>"
        );
        return device_shell("Device login", &body);
    }
    let body = format!(
        "<h1>Authorize this device?</h1>\
<p>A terminal is requesting access to your Tellur account. Confirm the code \
matches the one shown in your terminal.</p>\
<div class=\"code\">{safe_code}</div>\
<p class=\"muted\">Signed in as <strong>{member}</strong> · role {role} · org {org}</p>\
<form method=\"post\" action=\"/auth/device/decision\">\
<input type=\"hidden\" name=\"user_code\" value=\"{safe_code}\">\
<div class=\"row\">\
<button class=\"approve\" name=\"decision\" value=\"approve\" type=\"submit\">Authorize</button>\
<button class=\"deny\" name=\"decision\" value=\"deny\" type=\"submit\">Cancel</button>\
</div></form>",
        member = html_escape(&principal.member_id),
        role = html_escape(principal.role.as_str()),
        org = html_escape(&principal.org_id),
    );
    device_shell("Authorize device", &body)
}

/// Render the post-decision confirmation page.
fn render_device_result(outcome: DeviceOutcome) -> String {
    let body = match outcome {
        DeviceOutcome::Approved => {
            "<h1>Device authorized ✓</h1>\
<p>You can return to your terminal — it will finish signing in automatically. \
You may close this tab.</p>"
        }
        DeviceOutcome::Denied => {
            "<h1>Request denied</h1>\
<p>The device was not authorized. You can close this tab.</p>"
        }
        DeviceOutcome::Unknown => {
            "<h1>Nothing to do</h1>\
<p>This code is unknown, already handled, or expired. \
Start again from your terminal with <code>tellur login</code>.</p>"
        }
    };
    device_shell("Device login", body)
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
