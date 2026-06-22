//! Session listing and replay endpoints.

use super::common::*;

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

