//! D1 dashboard API: activity time-series (A1) + repo summary (A3).

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header::AUTHORIZATION};
use http_body_util::BodyExt;
use serde_json::Value;
use tellur_core::schema::types::{
    AttributionRange, AttributionState, EvidenceStrength, FileAttribution, Origin,
};
use tellur_server::auth::Role;
use tellur_server::ratelimit::RateLimiter;
use tellur_server::storage::{IngestEvent, SqliteStore, Store};
use tellur_server::{AppState, Config, build_router};
use tower::ServiceExt;

struct Setup {
    state: AppState,
    org_a: String,
    repo_id: String,
    viewer_a: String,
    admin_b: String,
}

fn token(store: &SqliteStore, org: &str, name: &str, role: Role) -> String {
    let m = store.create_member(org, name, role).unwrap();
    store.create_token(&m).unwrap().plaintext
}

fn ev(session: &str, kind: &str, actor: &str) -> IngestEvent {
    IngestEvent {
        session_id: session.into(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        event_type: kind.into(),
        actor: actor.into(),
        payload: serde_json::json!({ "file": "src/a.rs" }),
    }
}

fn ai_range(
    start: u32,
    end: u32,
    reviewer: Option<&str>,
    reviewed_at: Option<&str>,
) -> AttributionRange {
    AttributionRange {
        range_id: format!("r{start}"),
        start_line: start,
        end_line: end,
        origin: Origin::Ai,
        evidence_strength: EvidenceStrength::Recorded,
        confidence: 0.9,
        state: AttributionState::Exact,
        session_id: "s".into(),
        event_ids: vec![],
        agent_id: "claude".into(),
        model_id: None,
        prompt_hash: None,
        context_set_id: None,
        policy_tags: vec![],
        risk_tags: vec![],
        risk_level: None,
        tests_run: vec![],
        tests_passed: false,
        reviewer: reviewer.map(str::to_string),
        reviewed_at: reviewed_at.map(str::to_string),
    }
}

fn setup() -> Setup {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    store.migrate().unwrap();
    let org_a = store.create_org("A").unwrap().id;
    let viewer_a = token(&store, &org_a, "vic", Role::Viewer);
    let repo = store.ensure_repo(&org_a, "app").unwrap();
    store
        .append_events(
            &org_a,
            &repo.id,
            &[
                ev("s1", "file.write", "claude"),
                ev("s1", "file.write", "human"),
                ev("s2", "file.read", "claude"),
            ],
        )
        .unwrap();
    // 20 AI lines: 8 reviewed (distinct human + timestamp), 12 not.
    store
        .put_attributions(
            &org_a,
            &repo.id,
            &[FileAttribution {
                schema: "tellur.attribution.v1".into(),
                file_path: "src/a.rs".into(),
                git_blob_sha: "sha".into(),
                ranges: vec![
                    ai_range(1, 8, Some("alice"), Some("2026-06-08T01:00:00Z")),
                    ai_range(9, 20, None, None),
                ],
                updated_at: chrono::Utc::now().to_rfc3339(),
            }],
        )
        .unwrap();

    let org_b = store.create_org("B").unwrap().id;
    let admin_b = token(&store, &org_b, "bob", Role::Admin);

    let state = AppState {
        store,
        config: Arc::new(Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            db_path: ":memory:".into(),
            database_url: None,
            allow_non_loopback: false,
        }),
        rate_limiter: Arc::new(RateLimiter::new(10_000, Duration::from_secs(60))),
        metrics: Arc::new(tellur_server::Metrics::new()),
        oidc: None,
    };
    Setup {
        state,
        org_a,
        repo_id: repo.id,
        viewer_a,
        admin_b,
    }
}

async fn get(state: &AppState, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut b = Request::builder().method("GET").uri(uri);
    if let Some(t) = token {
        b = b.header(AUTHORIZATION, format!("Bearer {t}"));
    }
    let resp = build_router(state.clone())
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn activity_groups_by_type_and_actor() {
    let s = setup();
    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/activity?range=30d&group_by=type", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["group_by"], "type");
    assert_eq!(body["range_days"], 30);
    let buckets = body["buckets"].as_array().unwrap();
    let total: i64 = buckets.iter().map(|b| b["count"].as_i64().unwrap()).sum();
    assert_eq!(total, 3);
    assert!(
        buckets
            .iter()
            .any(|b| b["key"] == "file.write" && b["count"] == 2)
    );

    let (_, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/activity?group_by=actor", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    let buckets = body["buckets"].as_array().unwrap();
    assert!(
        buckets
            .iter()
            .any(|b| b["key"] == "claude" && b["count"] == 2)
    );
    assert!(
        buckets
            .iter()
            .any(|b| b["key"] == "human" && b["count"] == 1)
    );
}

#[tokio::test]
async fn activity_range_is_clamped() {
    let s = setup();
    let (_, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/activity?range=9999d", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(body["range_days"], 365);
}

#[tokio::test]
async fn repo_detail_reports_ai_share_and_review_coverage() {
    let s = setup();
    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/repos/{}", s.org_a, s.repo_id),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "app");
    assert_eq!(body["event_count"], 3);
    assert_eq!(body["lines"]["ai"], 20);
    assert_eq!(body["lines"]["reviewed_ai"], 8);
    // 20 AI lines, all attributed → ai_share 1.0; review coverage 8/20 = 0.4.
    assert_eq!(body["ai_share"], 1.0);
    assert!((body["review_coverage"].as_f64().unwrap() - 0.4).abs() < 1e-9);
    // Contributors come from the event log.
    let contributors = body["contributors"].as_array().unwrap();
    assert!(contributors.iter().any(|c| c == "claude"));
    assert!(contributors.iter().any(|c| c == "human"));
}

#[tokio::test]
async fn repo_detail_tenant_scoped_and_404() {
    let s = setup();
    // Cross-org caller is forbidden.
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/repos/{}", s.org_a, s.repo_id),
        Some(&s.admin_b),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Missing repo in own org is 404.
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/repos/ghost", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Unauthenticated is 401.
    let (status, _) = get(&s.state, &format!("/v1/orgs/{}/activity", s.org_a), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn attribution_read_returns_ranges_and_filters_by_path() {
    let s = setup();
    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/repos/{}/attributions", s.org_a, s.repo_id),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let files = body["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["file_path"], "src/a.rs");
    assert_eq!(files[0]["ranges"].as_array().unwrap().len(), 2);

    // Exact path filter that matches nothing → empty.
    let (_, body) = get(
        &s.state,
        &format!(
            "/v1/orgs/{}/repos/{}/attributions?path=nope.rs",
            s.org_a, s.repo_id
        ),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(body["files"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn sessions_list_and_detail() {
    let s = setup();
    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/sessions", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let sessions = body["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 2);
    let s1 = sessions.iter().find(|x| x["session_id"] == "s1").unwrap();
    assert_eq!(s1["event_count"], 2);
    let actors = s1["actors"].as_array().unwrap();
    assert!(actors.iter().any(|a| a == "claude"));
    assert!(actors.iter().any(|a| a == "human"));

    // Detail returns the session's events, oldest first.
    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/sessions/s1", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["truncated"], false);
    let events = body["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert!(events[0]["seq"].as_i64().unwrap() < events[1]["seq"].as_i64().unwrap());

    // Unknown session → 404.
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/sessions/ghost", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn sessions_repo_filter_and_tenant_scope() {
    let s = setup();
    // Filter by the repo id → still finds both sessions (both in that repo).
    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/sessions?repo={}", s.org_a, s.repo_id),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["sessions"].as_array().unwrap().len(), 2);
    // Unknown repo filter → 404.
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/sessions?repo=ghost", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Cross-org caller forbidden.
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/sessions", s.org_a),
        Some(&s.admin_b),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ─── A9: composed overview ───────────────────────────────────────────────────

#[tokio::test]
async fn overview_rolls_up_totals_and_ranks_repos_by_review_gap() {
    let s = setup();
    // Second repo with more unreviewed AI lines, so it must rank first.
    let repo2 = s.state.store.ensure_repo(&s.org_a, "infra").unwrap();
    s.state
        .store
        .append_events(&s.org_a, &repo2.id, &[ev("s3", "file.write", "claude")])
        .unwrap();
    s.state
        .store
        .put_attributions(
            &s.org_a,
            &repo2.id,
            &[FileAttribution {
                schema: "tellur.attribution.v1".into(),
                file_path: "infra/main.tf".into(),
                git_blob_sha: "sha2".into(),
                ranges: vec![ai_range(1, 50, None, None)], // 50 unreviewed AI lines
                updated_at: chrono::Utc::now().to_rfc3339(),
            }],
        )
        .unwrap();

    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/overview", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["schema"], "tellur.server.overview.v1");
    // Totals fold both repos: app has 20 AI lines (8 reviewed), infra 50 (0).
    assert_eq!(body["totals"]["ai_lines"], 70);
    assert_eq!(body["totals"]["reviewed_ai_lines"], 8);
    assert_eq!(body["totals"]["repos"], 2);
    // Risk ranking: infra (gap 50) ahead of app (gap 12).
    let repos = body["repos"].as_array().unwrap();
    assert_eq!(repos[0]["name"], "infra");
    assert_eq!(repos[0]["review_gap_lines"], 50);
    assert_eq!(repos[1]["name"], "app");
    assert_eq!(repos[1]["review_gap_lines"], 12);
    // One round-trip carries activity + recent feed too.
    assert!(body["activity"].is_array());
    assert!(body["recent_events"].is_array());
}

#[tokio::test]
async fn overview_is_tenant_scoped() {
    let s = setup();
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/overview", s.org_a),
        Some(&s.admin_b),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ─── Evidence exports (A13: per-repo SLSA/SPDX jobs + org evidence pack) ──────

#[tokio::test]
async fn evidence_pack_bundles_provenance_and_audit() {
    let s = setup();
    let admin_a = admin_token(&s.state, &s.org_a);
    let (status, enq) = post(
        &s.state,
        &format!("/v1/orgs/{}/export/evidence", s.org_a),
        Some(&admin_a),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let job_id = enq["job_id"].as_str().unwrap().to_string();
    assert!(tellur_server::jobs::process_one(&s.state.store).unwrap());

    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/jobs/{job_id}", s.org_a),
        Some(&admin_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "completed");
    let r = &body["result"];
    assert_eq!(r["schema"], "tellur.server.evidence.v1");
    assert_eq!(r["repos_evaluated"], 1);
    assert_eq!(r["provenance"][0]["repo_name"], "app");
    assert!(r["provenance"][0]["slsa"].is_object());
    assert_eq!(r["audit"]["chain_intact"], true);
}

#[tokio::test]
async fn per_repo_slsa_export_runs_as_a_job() {
    let s = setup();
    let admin_a = admin_token(&s.state, &s.org_a);
    let (status, enq) = post(
        &s.state,
        &format!("/v1/orgs/{}/repos/app/export/slsa", s.org_a),
        Some(&admin_a),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let job_id = enq["job_id"].as_str().unwrap().to_string();
    assert!(tellur_server::jobs::process_one(&s.state.store).unwrap());

    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/jobs/{job_id}", s.org_a),
        Some(&admin_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "completed");
    assert_eq!(body["result"]["schema"], "tellur.server.export.slsa.v1");
    assert!(body["result"]["document"].is_object());

    // The async POST requires org admin (the result is polled via the
    // org-admin-scoped jobs read), so a viewer is refused.
    let (status, _) = post(
        &s.state,
        &format!("/v1/orgs/{}/repos/app/export/slsa", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn evidence_export_requires_admin() {
    let s = setup();
    let (status, _) = post(
        &s.state,
        &format!("/v1/orgs/{}/export/evidence", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ─── D3: audit read (A7) + jobs list ─────────────────────────────────────────

/// Mint an admin token in an org via the Store trait (setup only gives a viewer).
fn admin_token(state: &AppState, org: &str) -> String {
    let m = state.store.create_member(org, "adm", Role::Admin).unwrap();
    state.store.create_token(&m).unwrap().plaintext
}

#[tokio::test]
async fn audit_read_filters_paginates_and_verifies_chain() {
    let s = setup();
    let admin_a = admin_token(&s.state, &s.org_a);
    // Seed a handful of audit entries for org A (plus one for another org that
    // must never leak in).
    for i in 0..5 {
        s.state
            .store
            .append_audit(&tellur_server::storage::AuditEntry {
                org_id: Some(s.org_a.clone()),
                actor_member_id: Some(if i % 2 == 0 { "alice" } else { "bob" }.into()),
                action: "policy.update".into(),
                detail: format!("n{i}"),
            })
            .unwrap();
    }
    let org_b = s.state.store.create_org("Bdiff").unwrap().id;
    s.state
        .store
        .append_audit(&tellur_server::storage::AuditEntry {
            org_id: Some(org_b.clone()),
            actor_member_id: Some("eve".into()),
            action: "policy.update".into(),
            detail: "other-org".into(),
        })
        .unwrap();

    // First page (no cursor): newest-first, chain verified, tenant-scoped.
    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/audit?limit=2", s.org_a),
        Some(&admin_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["chain_intact"], true);
    let recs = body["records"].as_array().unwrap();
    assert_eq!(recs.len(), 2);
    assert_eq!(recs[0]["detail"], "n4");
    assert_eq!(recs[1]["detail"], "n3");
    // No org-B row leaked.
    assert!(recs.iter().all(|r| r["detail"] != "other-org"));

    // Keyset paginate with the returned cursor; chain check omitted on later pages.
    let cursor = body["next_before"].as_i64().unwrap();
    let (status, page2) = get(
        &s.state,
        &format!("/v1/orgs/{}/audit?limit=2&before={cursor}", s.org_a),
        Some(&admin_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(page2["chain_intact"].is_null());
    assert_eq!(page2["records"][0]["detail"], "n2");

    // Actor filter.
    let (_, filtered) = get(
        &s.state,
        &format!("/v1/orgs/{}/audit?actor=alice", s.org_a),
        Some(&admin_a),
    )
    .await;
    let fr = filtered["records"].as_array().unwrap();
    assert!(!fr.is_empty());
    assert!(fr.iter().all(|r| r["actor_member_id"] == "alice"));
}

#[tokio::test]
async fn audit_read_requires_admin() {
    let s = setup();
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/audit", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn jobs_list_admin_only_and_tenant_scoped() {
    let s = setup();
    let admin_a = admin_token(&s.state, &s.org_a);
    s.state
        .store
        .enqueue_job(&s.org_a, tellur_server::jobs::KIND_EXPORT_AUDIT, None)
        .unwrap();
    s.state
        .store
        .enqueue_job(&s.org_a, tellur_server::jobs::KIND_EXPORT_EVENTS, None)
        .unwrap();

    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/jobs", s.org_a),
        Some(&admin_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["jobs"].as_array().unwrap().len(), 2);

    // Viewer forbidden.
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/jobs", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Cross-org caller forbidden.
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/jobs", s.org_a),
        Some(&s.admin_b),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ─── D4: compliance (A8) + People & Access (A2/A10/A11) ──────────────────────

async fn post(state: &AppState, uri: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut b = Request::builder().method("POST").uri(uri);
    if let Some(t) = token {
        b = b.header(AUTHORIZATION, format!("Bearer {t}"));
    }
    let resp = build_router(state.clone())
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn compliance_evaluates_policy_and_reads_latest() {
    let s = setup();
    let admin_a = admin_token(&s.state, &s.org_a);
    // A `default` policy that requires human review for src/** — the seeded
    // attribution has one reviewed AI range and one unreviewed → 1 violation.
    s.state
        .store
        .put_policy(
            &s.org_a,
            "default",
            "version: 1\nsensitive_paths:\n  - path: \"src/**\"\n    tags: [\"core\"]\n    require_human_review: true\n",
        )
        .unwrap();

    // Before any run: not evaluated.
    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/policies/compliance", s.org_a),
        Some(&admin_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["evaluated"], false);

    // Enqueue via the HTTP endpoint (admin), then run the worker deterministically.
    let (status, enq) = post(
        &s.state,
        &format!("/v1/orgs/{}/policies/compliance", s.org_a),
        Some(&admin_a),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(enq["job_id"].is_string());
    assert!(tellur_server::jobs::process_one(&s.state.store).unwrap());

    // Latest snapshot now reflects the evaluation.
    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/policies/compliance", s.org_a),
        Some(&admin_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["evaluated"], true);
    let snaps = body["snapshots"].as_array().unwrap();
    assert_eq!(snaps.len(), 1);
    assert_eq!(snaps[0]["ai_ranges"], 2);
    assert_eq!(snaps[0]["violations"], 1);
    assert_eq!(snaps[0]["high"], 1);
    assert_eq!(snaps[0]["policy_version"], 1);
}

#[tokio::test]
async fn compliance_and_people_require_admin() {
    let s = setup();
    for path in ["policies/compliance", "members", "groups", "sso-status"] {
        let (status, _) = get(
            &s.state,
            &format!("/v1/orgs/{}/{path}", s.org_a),
            Some(&s.viewer_a),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path} must be admin-only");
    }
    // Cross-org admin is also forbidden.
    let (status, _) = post(
        &s.state,
        &format!("/v1/orgs/{}/policies/compliance", s.org_a),
        Some(&s.admin_b),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn members_groups_and_sso_status() {
    let s = setup();
    let admin_a = admin_token(&s.state, &s.org_a);

    // Members: includes the seeded viewer + the admin we just minted.
    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/members", s.org_a),
        Some(&admin_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let members = body["members"].as_array().unwrap();
    assert!(members.len() >= 2);
    assert!(members.iter().all(|m| m["active"] == true));

    // Groups: a SCIM admin group maps to the admin role.
    s.state
        .store
        .scim_create_group(&s.org_a, "tellur-admin", None, &[])
        .unwrap();
    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/groups", s.org_a),
        Some(&admin_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let groups = body["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["maps_to_role"], "admin");

    // SSO status: no OIDC configured in this harness; counts present, no secrets.
    let (status, body) = get(
        &s.state,
        &format!("/v1/orgs/{}/sso-status", s.org_a),
        Some(&admin_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["oidc_enabled"], false);
    assert_eq!(body["scim_configured"], false);
    assert_eq!(body["scim_groups"], 1);
    assert!(body["members_total"].as_u64().unwrap() >= 2);
    // Never leak secrets.
    assert!(body.get("client_secret").is_none());
}
