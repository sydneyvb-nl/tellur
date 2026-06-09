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
