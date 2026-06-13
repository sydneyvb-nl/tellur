//! B2 integration tests: provenance ingest — authz, BOLA, caps, rate limit.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{
    Request, StatusCode,
    header::{AUTHORIZATION, CONTENT_TYPE},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tellur_server::auth::Role;
use tellur_server::ratelimit::RateLimiter;
use tellur_server::storage::{SqliteStore, Store};
use tellur_server::{AppState, Config, build_router};
use tower::ServiceExt;

struct Setup {
    state: AppState,
    store: Arc<SqliteStore>,
    org_a: String,
    admin_a: String,
    viewer_a: String,
    contributor_a: String,
    admin_b: String,
}

fn token_for(store: &SqliteStore, org: &str, name: &str, role: Role) -> String {
    let m = store.create_member(org, name, role).unwrap();
    store.create_token(&m).unwrap().plaintext
}

fn setup_with_limiter(max: u32) -> Setup {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    store.migrate().unwrap();

    let org_a = store.create_org("Org A").unwrap().id;
    let admin_a = token_for(&store, &org_a, "alice", Role::Admin);
    let viewer_a = token_for(&store, &org_a, "vic", Role::Viewer);
    let contributor_a = token_for(&store, &org_a, "carl", Role::Contributor);
    let org_b = store.create_org("Org B").unwrap().id;
    let admin_b = token_for(&store, &org_b, "bob", Role::Admin);

    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        db_path: ":memory:".into(),
        database_url: None,
        allow_non_loopback: false,
    };
    let state = AppState {
        store: store.clone(),
        config: Arc::new(config),
        rate_limiter: Arc::new(RateLimiter::new(max, Duration::from_secs(60))),
        metrics: Arc::new(tellur_server::Metrics::new()),
        oidc: None,
        github_app: None,
    };
    Setup {
        state,
        store,
        org_a,
        admin_a,
        viewer_a,
        contributor_a,
        admin_b,
    }
}

fn setup() -> Setup {
    setup_with_limiter(1000)
}

async fn post(
    state: &AppState,
    uri: &str,
    bearer: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(CONTENT_TYPE, "application/json");
    if let Some(b) = bearer {
        builder = builder.header(AUTHORIZATION, format!("Bearer {b}"));
    }
    let resp = build_router(state.clone())
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("expected JSON response for {uri} ({status}): {e}"));
    (status, json)
}

fn sample_body() -> Value {
    json!({"events": [
        {"session_id": "s1", "type": "file.write", "payload": {"file_path": "a.rs"}},
        {"session_id": "s1", "type": "command.exec", "payload": {"command": "cargo test"}}
    ]})
}

#[tokio::test]
async fn contributor_can_ingest_and_chain_verifies() {
    let s = setup();
    let uri = format!("/v1/orgs/{}/repos/app/events", s.org_a);
    let (status, json) = post(&s.state, &uri, Some(&s.contributor_a), sample_body()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["count"], 2);
    let repo_id = json["repo_id"].as_str().unwrap();
    assert_eq!(s.store.event_count(&s.org_a, repo_id).unwrap(), 2);
    assert!(s.store.verify_event_chain(&s.org_a, repo_id).unwrap());
}

#[tokio::test]
async fn viewer_cannot_ingest() {
    let s = setup();
    let uri = format!("/v1/orgs/{}/repos/app/events", s.org_a);
    let (status, json) = post(&s.state, &uri, Some(&s.viewer_a), sample_body()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(json["status"], 403);
    assert_eq!(json["title"], "forbidden");
}

#[tokio::test]
async fn cross_org_ingest_is_forbidden_bola() {
    let s = setup();
    // admin of org B tries to write into org A.
    let uri = format!("/v1/orgs/{}/repos/app/events", s.org_a);
    let (status, _) = post(&s.state, &uri, Some(&s.admin_b), sample_body()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn empty_and_oversized_batches_are_rejected() {
    let s = setup();
    let uri = format!("/v1/orgs/{}/repos/app/events", s.org_a);

    let (status, _) = post(&s.state, &uri, Some(&s.admin_a), json!({"events": []})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let many: Vec<Value> = (0..1001)
        .map(|_| json!({"session_id": "s", "type": "file.write", "payload": {}}))
        .collect();
    let (status, _) = post(&s.state, &uri, Some(&s.admin_a), json!({"events": many})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rate_limit_returns_429() {
    let s = setup_with_limiter(1);
    let uri = format!("/v1/orgs/{}/repos/app/events", s.org_a);
    let (status, _) = post(&s.state, &uri, Some(&s.admin_a), sample_body()).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post(&s.state, &uri, Some(&s.admin_a), sample_body()).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn unauthenticated_ingest_is_rejected() {
    let s = setup();
    let uri = format!("/v1/orgs/{}/repos/app/events", s.org_a);
    let (status, json) = post(&s.state, &uri, None, sample_body()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["status"], 401);
    assert_eq!(json["title"], "unauthorized");
}
