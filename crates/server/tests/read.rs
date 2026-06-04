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
        allow_non_loopback: false,
    };
    let state = AppState {
        store,
        config: Arc::new(config),
        rate_limiter: Arc::new(RateLimiter::new(1000, Duration::from_secs(60))),
    };
    Setup {
        state,
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
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
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
        &format!("/v1/orgs/{}/repos/app/events?limit=2", s.org_a),
        Some(&s.viewer_a),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["events"].as_array().unwrap().len(), 2);
    assert!(json["next_before"].is_i64()); // full page → cursor present

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

    let _ = s.repo_id; // silence unused if cfg changes
}
