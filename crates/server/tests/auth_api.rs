//! B1 integration tests: authentication, tenant scoping, BOLA prevention, audit.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode, header::AUTHORIZATION};
use http_body_util::BodyExt;
use serde_json::Value;
use tellur_server::auth::Role;
use tellur_server::storage::{SqliteStore, Store};
use tellur_server::{AppState, Config, build_router};
use tower::ServiceExt;

struct Setup {
    state: AppState,
    store: Arc<SqliteStore>,
    org_a: String,
    token_a: String,
    org_b: String,
    token_b: String,
}

fn setup() -> Setup {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    store.migrate().unwrap();

    let org_a = store.create_org("Org A").unwrap().id;
    let m_a = store.create_member(&org_a, "alice", Role::Admin).unwrap();
    let token_a = store.create_token(&m_a).unwrap().plaintext;

    let org_b = store.create_org("Org B").unwrap().id;
    let m_b = store.create_member(&org_b, "bob", Role::Admin).unwrap();
    let token_b = store.create_token(&m_b).unwrap().plaintext;

    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        db_path: ":memory:".into(),
        database_url: None,
        allow_non_loopback: false,
    };
    let state = AppState {
        store: store.clone(),
        config: Arc::new(config),
        rate_limiter: Arc::new(tellur_server::ratelimit::RateLimiter::new(
            1000,
            std::time::Duration::from_secs(60),
        )),
        metrics: Arc::new(tellur_server::Metrics::new()),
        oidc: None,
        github_app: None,
    };
    Setup {
        state,
        store,
        org_a,
        token_a,
        org_b,
        token_b,
    }
}

async fn get(state: &AppState, uri: &str, bearer: Option<&str>) -> (StatusCode, Value) {
    let mut builder = Request::builder().uri(uri);
    if let Some(b) = bearer {
        builder = builder.header(AUTHORIZATION, format!("Bearer {b}"));
    }
    let resp = build_router(state.clone())
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("expected JSON response for {uri} ({status}): {e}"));
    (status, json)
}

#[tokio::test]
async fn unauthenticated_request_is_rejected() {
    let s = setup();
    let (status, json) = get(&s.state, "/v1/me", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["status"], 401);
    assert_eq!(json["title"], "unauthorized");

    let (status, json) = get(&s.state, "/v1/me", Some("tlr_bogus_token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json["status"], 401);
    assert_eq!(json["title"], "unauthorized");
}

#[tokio::test]
async fn invalid_token_is_audited_but_missing_header_is_not() {
    let s = setup();
    let before = s.store.audit_len().unwrap();

    // No Authorization header → rejected, NOT audited (avoids anonymous flood).
    let (status, _) = get(&s.state, "/v1/me", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(s.store.audit_len().unwrap(), before);

    // Presented-but-invalid token → rejected AND audited as a probing signal.
    let (status, _) = get(&s.state, "/v1/me", Some("tlr_deadbeef_wrong")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(s.store.audit_len().unwrap(), before + 1);
    assert!(s.store.verify_audit_chain().unwrap());
}

#[tokio::test]
async fn valid_token_returns_identity() {
    let s = setup();
    let (status, json) = get(&s.state, "/v1/me", Some(&s.token_a)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["org_id"], s.org_a);
    assert_eq!(json["role"], "admin");
}

#[tokio::test]
async fn same_org_access_is_allowed() {
    let s = setup();
    let uri = format!("/v1/orgs/{}/me", s.org_a);
    let (status, json) = get(&s.state, &uri, Some(&s.token_a)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["org_id"], s.org_a);
}

#[tokio::test]
async fn cross_org_access_is_forbidden_bola() {
    let s = setup();
    // Token A must not reach Org B's resource, and vice versa.
    let uri_b = format!("/v1/orgs/{}/me", s.org_b);
    let (status, _) = get(&s.state, &uri_b, Some(&s.token_a)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let uri_a = format!("/v1/orgs/{}/me", s.org_a);
    let (status, _) = get(&s.state, &uri_a, Some(&s.token_b)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn access_is_audited_and_chain_stays_intact() {
    let s = setup();
    let before = s.store.audit_len().unwrap();

    let _ = get(&s.state, "/v1/me", Some(&s.token_a)).await;
    let uri_b = format!("/v1/orgs/{}/me", s.org_b);
    let _ = get(&s.state, &uri_b, Some(&s.token_a)).await; // denied → also audited

    let after = s.store.audit_len().unwrap();
    assert_eq!(after, before + 2, "expected one audit entry per request");
    assert!(s.store.verify_audit_chain().unwrap());
}
