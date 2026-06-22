//! Identity + analytics endpoints: `me`, org report, dashboard, overview, activity.

use super::common::*;

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
