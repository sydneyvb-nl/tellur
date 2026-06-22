//! Org policy CRUD endpoints.

use super::common::*;

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

