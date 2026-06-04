//! B4 integration tests: central policy distribution + export portal.

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
    admin_a: String,
    viewer_a: String,
    admin_b: String,
}

fn token(store: &SqliteStore, org: &str, name: &str, role: Role) -> String {
    let m = store.create_member(org, name, role).unwrap();
    store.create_token(&m).unwrap().plaintext
}

fn setup() -> Setup {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    store.migrate().unwrap();
    let org_a = store.create_org("A").unwrap().id;
    let admin_a = token(&store, &org_a, "alice", Role::Admin);
    let viewer_a = token(&store, &org_a, "vic", Role::Viewer);
    // Seed an event so audit/event export has content.
    let repo = store.ensure_repo(&org_a, "app").unwrap();
    store
        .append_events(
            &org_a,
            &repo.id,
            &[IngestEvent {
                session_id: "s".into(),
                timestamp: "2026-06-04T00:00:00Z".into(),
                event_type: "file.write".into(),
                actor: "agent".into(),
                payload: serde_json::json!({"file_path": "a.rs"}),
            }],
        )
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
        metrics: Arc::new(tellur_server::Metrics::new()),
    };
    Setup {
        state,
        org_a,
        admin_a,
        viewer_a,
        admin_b,
    }
}

async fn req(
    state: &AppState,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<&str>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = bearer {
        b = b.header(AUTHORIZATION, format!("Bearer {t}"));
    }
    let body = body
        .map(|s| Body::from(s.to_string()))
        .unwrap_or(Body::empty());
    let resp = build_router(state.clone())
        .oneshot(b.body(body).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

const VALID_POLICY: &str = "version: 1\nrules: []\n";

#[tokio::test]
async fn admin_can_put_get_and_version_policies() {
    let s = setup();
    let put_uri = format!("/v1/orgs/{}/policies/default", s.org_a);

    let (status, json) = req(
        &s.state,
        "PUT",
        &put_uri,
        Some(&s.admin_a),
        Some(VALID_POLICY),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["version"], 1);

    // Re-uploading bumps the version.
    let (_, json) = req(
        &s.state,
        "PUT",
        &put_uri,
        Some(&s.admin_a),
        Some(VALID_POLICY),
    )
    .await;
    assert_eq!(json["version"], 2);

    // A viewer can pull it.
    let (status, json) = req(&s.state, "GET", &put_uri, Some(&s.viewer_a), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["content"], VALID_POLICY);
    assert_eq!(json["version"], 2);

    // And list it.
    let (status, json) = req(
        &s.state,
        "GET",
        &format!("/v1/orgs/{}/policies", s.org_a),
        Some(&s.viewer_a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["policies"][0]["name"], "default");
}

#[tokio::test]
async fn invalid_policy_is_rejected() {
    let s = setup();
    let uri = format!("/v1/orgs/{}/policies/bad", s.org_a);
    let (status, _) = req(
        &s.state,
        "PUT",
        &uri,
        Some(&s.admin_a),
        Some("version: not_a_number"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn non_admin_and_cross_org_policy_writes_forbidden() {
    let s = setup();
    let uri = format!("/v1/orgs/{}/policies/default", s.org_a);
    // Viewer can't write.
    let (status, _) = req(&s.state, "PUT", &uri, Some(&s.viewer_a), Some(VALID_POLICY)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Org B admin can't write into org A.
    let (status, _) = req(&s.state, "PUT", &uri, Some(&s.admin_b), Some(VALID_POLICY)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = req(
        &s.state,
        "GET",
        &format!("/v1/orgs/{}/policies/missing", s.org_a),
        Some(&s.admin_a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_export_events_and_audit() {
    let s = setup();
    let (status, json) = req(
        &s.state,
        "GET",
        &format!("/v1/orgs/{}/export/events", s.org_a),
        Some(&s.admin_a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["count"], 1);
    assert_eq!(json["schema"], "tellur.server.export.events.v1");
    // Org-level export must carry repo identity per event.
    assert!(json["events"][0]["repo_id"].as_str().is_some());

    let (status, json) = req(
        &s.state,
        "GET",
        &format!("/v1/orgs/{}/export/audit", s.org_a),
        Some(&s.admin_a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["chain_intact"], true);
}

#[tokio::test]
async fn export_requires_admin_and_same_org() {
    let s = setup();
    let (status, _) = req(
        &s.state,
        "GET",
        &format!("/v1/orgs/{}/export/events", s.org_a),
        Some(&s.viewer_a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = req(
        &s.state,
        "GET",
        &format!("/v1/orgs/{}/export/audit", s.org_a),
        Some(&s.admin_b),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
