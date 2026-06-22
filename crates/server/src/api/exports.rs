//! Jobs, compliance, audit listing, and export endpoints.

use super::common::*;

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

