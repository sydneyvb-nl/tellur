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
use tellur_core::schema::types::FileAttribution;

use crate::app::AppState;
use crate::auth::{Principal, Role};
use crate::error::ServerError;
use crate::storage::{AuditEntry, IngestEvent};

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

    // Validate batch size before resolving/creating the repo, so an empty or
    // oversized request never creates a repo as a side effect.
    if req.events.is_empty() {
        return Err(ServerError::BadRequest("no events provided".to_string()));
    }
    if req.events.len() > MAX_EVENTS_PER_REQUEST {
        return Err(ServerError::BadRequest(format!(
            "too many events: {} (max {MAX_EVENTS_PER_REQUEST})",
            req.events.len()
        )));
    }

    // Per-repo authorization. For an existing repo, an additive per-repo grant
    // can elevate an org viewer to contributor. Creating a *new* repo always
    // requires the org-baseline contributor role (no grant can exist yet).
    let repo = match state
        .store
        .find_repo(&org_id, &repo)
        .map_err(ServerError::Internal)?
    {
        Some(existing) => {
            if !effective_role(&state, &principal, &existing.id)?.allows(Role::Contributor) {
                return Err(deny(
                    &state,
                    &principal,
                    &org_id,
                    "ingest_denied",
                    &format!("repo={} role={}", existing.id, principal.role.as_str()),
                ));
            }
            existing
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
            state
                .store
                .ensure_repo(&org_id, &repo)
                .map_err(ServerError::Internal)?
        }
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

// ─── Export portal ───────────────────────────────────────────────────────────

/// `GET /v1/orgs/{org}/export/events` — full provenance event bundle (admin).
pub async fn export_events(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_org_role(&state, &principal, &org_id, Role::Admin, "export.events")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let events = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        move || store.export_events(&org)
    })
    .await?;
    state.metrics.inc_export();
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id.clone()),
            actor_member_id: Some(principal.member_id.clone()),
            action: "export.events".to_string(),
            detail: format!("count={}", events.len()),
        })
        .map_err(ServerError::Internal)?;
    Ok(Json(json!({
        "schema": "tellur.server.export.events.v1",
        "org_id": org_id,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "count": events.len(),
        "events": events,
    })))
}

/// `GET /v1/orgs/{org}/export/audit` — org audit trail + chain integrity (admin).
pub async fn export_audit(
    State(state): State<AppState>,
    Path(org_id): Path<String>,
    principal: Principal,
) -> Result<Json<Value>, ServerError> {
    ensure_org_role(&state, &principal, &org_id, Role::Admin, "export.audit")?;
    if !state.rate_limiter.check(&principal.member_id) {
        return Err(ServerError::TooManyRequests);
    }
    let (entries, chain_intact) = run_blocking({
        let store = state.store.clone();
        let org = org_id.clone();
        move || {
            let entries = store.export_audit(&org)?;
            let chain_intact = store.verify_audit_chain()?;
            Ok((entries, chain_intact))
        }
    })
    .await?;
    state.metrics.inc_export();
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id.clone()),
            actor_member_id: Some(principal.member_id.clone()),
            action: "export.audit".to_string(),
            detail: format!("count={} chain_intact={chain_intact}", entries.len()),
        })
        .map_err(ServerError::Internal)?;
    Ok(Json(json!({
        "schema": "tellur.server.export.audit.v1",
        "org_id": org_id,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "chain_intact": chain_intact,
        "count": entries.len(),
        "entries": entries,
    })))
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

    // Per-repo authorization (additive grants), mirroring event ingest: an
    // existing repo can be written by a per-repo contributor; creating a new
    // repo requires the org-baseline contributor role.
    let repo = match state
        .store
        .find_repo(&org_id, &repo)
        .map_err(ServerError::Internal)?
    {
        Some(existing) => {
            if !effective_role(&state, &principal, &existing.id)?.allows(Role::Contributor) {
                return Err(deny(
                    &state,
                    &principal,
                    &org_id,
                    "attributions_denied",
                    &format!("repo={} role={}", existing.id, principal.role.as_str()),
                ));
            }
            existing
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
            state
                .store
                .ensure_repo(&org_id, &repo)
                .map_err(ServerError::Internal)?
        }
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
    let role = Role::parse(&req.role)
        .map_err(|_| ServerError::BadRequest(format!("unknown role: {}", req.role)))?;
    let repo = admin_resolve_repo(&state, &principal, &org_id, &repo, "repo_role.set").await?;
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
    Ok(Json(json!({ "repo_id": repo.id, "grants": grants })))
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
