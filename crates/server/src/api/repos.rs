//! Repository, event, attribution, and source endpoints.

use super::common::*;

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
