//! B3 integration tests: read + report endpoints (tenant-scoped).

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header::AUTHORIZATION};
use http_body_util::BodyExt;
use serde_json::Value;
use tellur_server::auth::Role;
use tellur_server::ratelimit::RateLimiter;
use tellur_server::storage::{IngestEvent, SqliteStore, Store};
use tellur_server::{AppState, Config, build_router};
use tower::ServiceExt;

struct Setup {
    state: AppState,
    store: Arc<SqliteStore>,
    org_a: String,
    repo_id: String,
    viewer_a: String,
    admin_b: String,
}

fn token(store: &SqliteStore, org: &str, name: &str, role: Role) -> String {
    let m = store.create_member(org, name, role).unwrap();
    store.create_token(&m).unwrap().plaintext
}

fn ev(file: &str) -> IngestEvent {
    IngestEvent {
        session_id: "s1".to_string(),
        timestamp: "2026-06-03T00:00:00Z".to_string(),
        event_type: "file.write".to_string(),
        actor: "agent".to_string(),
        payload: serde_json::json!({ "file_path": file }),
    }
}

fn setup() -> Setup {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    store.migrate().unwrap();
    let org_a = store.create_org("A").unwrap().id;
    let viewer_a = token(&store, &org_a, "vic", Role::Viewer);
    let repo = store.ensure_repo(&org_a, "app").unwrap();
    store
        .append_events(&org_a, &repo.id, &[ev("a.rs"), ev("b.rs"), ev("c.rs")])
        .unwrap();
    let org_b = store.create_org("B").unwrap().id;
    let admin_b = token(&store, &org_b, "bob", Role::Admin);

    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        db_path: ":memory:".into(),
        database_url: None,
        allow_non_loopback: false,
    };
    let state = AppState {
        store: store.clone(),
        config: Arc::new(config),
        rate_limiter: Arc::new(RateLimiter::new(1000, Duration::from_secs(60))),
        metrics: Arc::new(tellur_server::Metrics::new()),
        oidc: None,
        github_app: None,
    };
    Setup {
        state,
        store,
        org_a,
        repo_id: repo.id,
        viewer_a,
        admin_b,
    }
}

async fn get(state: &AppState, uri: &str, bearer: Option<&str>) -> (StatusCode, Value) {
    let mut b = Request::builder().uri(uri);
    if let Some(t) = bearer {
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
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("expected JSON response for {uri} ({status}): {e}")),
    )
}

#[tokio::test]
async fn viewer_can_list_repos_events_and_report() {
    let s = setup();

    let (status, json) = get(
        &s.state,
        &format!("/v1/orgs/{}/repos", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["repos"][0]["name"], "app");
    assert_eq!(json["repos"][0]["event_count"], 3);

    let (status, json) = get(
        &s.state,
        &format!("/v1/orgs/{}/repos/{}/events?limit=2", s.org_a, s.repo_id),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first_page = json["events"].as_array().unwrap();
    assert_eq!(first_page.len(), 2);
    assert_eq!(first_page[0]["payload"]["file_path"], "c.rs");
    assert_eq!(first_page[1]["payload"]["file_path"], "b.rs");
    let next_before = json["next_before"].as_i64().unwrap(); // full page → cursor present

    let (status, json) = get(
        &s.state,
        &format!(
            "/v1/orgs/{}/repos/{}/events?limit=2&before={next_before}",
            s.org_a, s.repo_id
        ),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let second_page = json["events"].as_array().unwrap();
    assert_eq!(second_page.len(), 1);
    assert_eq!(second_page[0]["payload"]["file_path"], "a.rs");
    assert!(json["next_before"].is_null());

    let (status, json) = get(
        &s.state,
        &format!("/v1/orgs/{}/report", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["total_events"], 3);
    assert_eq!(json["distinct_sessions"], 1);
    assert_eq!(json["by_type"]["file.write"], 3);

    // The consolidated dashboard payload combines the rollup + a recent feed.
    let (status, json) = get(
        &s.state,
        &format!("/v1/orgs/{}/dashboard?limit=2", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["schema"], "tellur.server.dashboard.v1");
    assert_eq!(json["report"]["total_events"], 3);
    assert_eq!(json["recent_events"].as_array().unwrap().len(), 2);
    // Recent feed is newest-first and carries repo identity.
    assert!(json["recent_events"][0]["repo_id"].as_str().is_some());
}

#[tokio::test]
async fn reads_are_tenant_scoped_bola() {
    let s = setup();
    // Org B admin cannot read Org A.
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/repos", s.org_a),
        Some(&s.admin_b),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/report", s.org_a),
        Some(&s.admin_b),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/repos/{}/events", s.org_a, s.repo_id),
        Some(&s.admin_b),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn missing_repo_is_404_and_unauth_is_401() {
    let s = setup();
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/repos/nope/events", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = get(&s.state, &format!("/v1/orgs/{}/repos", s.org_a), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_runs_before_query_parsing() {
    let s = setup();
    // Bad ?limit on a protected endpoint with no token must be 401, not 400.
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/repos/{}/events?limit=abc", s.org_a, s.repo_id),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn successful_event_read_is_audited() {
    let s = setup();
    let before = s.store.audit_len().unwrap();
    let (status, _) = get(
        &s.state,
        &format!("/v1/orgs/{}/repos/{}/events", s.org_a, s.repo_id),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(s.store.audit_len().unwrap() > before);
    assert!(s.store.verify_audit_chain().unwrap());
}
