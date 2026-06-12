//! Source-connection (A12) integration tests: the admin settings endpoints
//! (`PUT`/`GET .../source`) and the private-repo blob proxy (`GET .../blob`).
//! The successful proxied fetch is a network call covered by unit tests in the
//! `source` module; here we exercise auth, the token-never-leaks contract, the
//! preserve/clear token semantics, and the SSRF allowlist rejection.

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
    org: String,
    admin: String,
    viewer: String,
}

fn setup() -> Setup {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    store.migrate().unwrap();
    let org = store.create_org("A").unwrap().id;
    let admin_id = store.create_member(&org, "alice", Role::Admin).unwrap();
    let admin = store.create_token(&admin_id).unwrap().plaintext;
    let viewer_id = store.create_member(&org, "vic", Role::Viewer).unwrap();
    let viewer = store.create_token(&viewer_id).unwrap().plaintext;
    // Create the repo "app" by ingesting one event as admin.
    store.ensure_repo(&org, "app").unwrap();
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
        org,
        admin,
        viewer,
    }
}

async fn req(
    state: &AppState,
    method: &str,
    uri: &str,
    bearer: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(AUTHORIZATION, format!("Bearer {bearer}"));
    let body = match body {
        Some(v) => {
            b = b.header(CONTENT_TYPE, "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
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

#[tokio::test]
async fn set_get_preserve_and_clear_token_never_leaks_it() {
    let s = setup();
    let url = format!("/v1/orgs/{}/repos/app/source", s.org);

    // Admin connects a private GitHub repo with a token.
    let (st, body) = req(
        &s.state,
        "PUT",
        &url,
        &s.admin,
        Some(json!({
            "template": "https://github.com/acme/app/blob/main/{path}#L{start}-L{end}",
            "raw_template": "https://api.github.com/repos/acme/app/contents/{path}?ref=main",
            "token": "ghp_secret123",
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["token_configured"], true);

    // GET returns the templates + token_configured, never the token itself.
    let (st, body) = req(&s.state, "GET", &url, &s.admin, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["token_configured"], true);
    assert!(body.get("token").is_none() && body.get("source_token").is_none());
    let serialized = body.to_string();
    assert!(
        !serialized.contains("ghp_secret123"),
        "token must never leak"
    );

    // Editing the template without resending the token preserves it.
    let (st, body) = req(
        &s.state,
        "PUT",
        &url,
        &s.admin,
        Some(json!({
            "template": "https://github.com/acme/app/blob/dev/{path}#L{start}-L{end}",
            "raw_template": "https://api.github.com/repos/acme/app/contents/{path}?ref=dev",
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["token_configured"], true, "token preserved on edit");

    // clear_token removes it.
    let (_st, body) = req(
        &s.state,
        "PUT",
        &url,
        &s.admin,
        Some(json!({
            "raw_template": "https://api.github.com/repos/acme/app/contents/{path}?ref=dev",
            "clear_token": true,
        })),
    )
    .await;
    assert_eq!(body["token_configured"], false);
}

#[tokio::test]
async fn source_settings_are_admin_only() {
    let s = setup();
    let url = format!("/v1/orgs/{}/repos/app/source", s.org);
    let (st, _) = req(&s.state, "GET", &url, &s.viewer, None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = req(
        &s.state,
        "PUT",
        &url,
        &s.viewer,
        Some(json!({ "template": "https://github.com/x/y/blob/main/{path}" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn blob_requires_a_raw_template() {
    let s = setup();
    // Only a link template, no raw → blob has nothing to fetch.
    req(
        &s.state,
        "PUT",
        &format!("/v1/orgs/{}/repos/app/source", s.org),
        &s.admin,
        Some(json!({ "template": "https://github.com/acme/app/blob/main/{path}" })),
    )
    .await;
    let (st, _) = req(
        &s.state,
        "GET",
        &format!("/v1/orgs/{}/repos/app/blob?path=src/a.rs", s.org),
        &s.viewer,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn blob_rejects_a_non_allowlisted_host() {
    let s = setup();
    // A raw template pointing off the provider allowlist must be refused before
    // any network call (SSRF guard), even though an admin set it.
    req(
        &s.state,
        "PUT",
        &format!("/v1/orgs/{}/repos/app/source", s.org),
        &s.admin,
        Some(json!({
            "raw_template": "https://internal.evil.example/{path}",
            "token": "t",
        })),
    )
    .await;
    let (st, body) = req(
        &s.state,
        "GET",
        &format!("/v1/orgs/{}/repos/app/blob?path=src/a.rs", s.org),
        &s.viewer,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    assert!(body["detail"].as_str().unwrap_or("").contains("allowed"));
}

#[tokio::test]
async fn attributions_reports_source_proxy_when_token_set() {
    let s = setup();
    let attr = format!("/v1/orgs/{}/repos/app/attributions", s.org);
    // No token → not a proxy repo.
    let (_st, body) = req(&s.state, "GET", &attr, &s.viewer, None).await;
    assert_eq!(body["source_proxy"], false);
    // Configure a token → proxy flag flips on.
    req(
        &s.state,
        "PUT",
        &format!("/v1/orgs/{}/repos/app/source", s.org),
        &s.admin,
        Some(json!({
            "raw_template": "https://raw.githubusercontent.com/acme/app/main/{path}",
            "token": "ghp_x",
        })),
    )
    .await;
    let (_st, body) = req(&s.state, "GET", &attr, &s.viewer, None).await;
    assert_eq!(body["source_proxy"], true);
}
