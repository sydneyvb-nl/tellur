//! Members, groups, SSO status, and per-repo role endpoints.

use super::common::*;

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

