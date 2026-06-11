//! Device-authorization flow integration tests (`tellur login` server side),
//! driven through the router. Covers the happy path (authorize → approve in a
//! signed-in session → poll returns a working token), the deny/expiry/pending
//! branches, the unauthenticated approval-page redirect, and the SSO-off gate.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header::AUTHORIZATION};
use http_body_util::BodyExt;
use serde_json::Value;
use tellur_server::auth::Role;
use tellur_server::oidc::{Discovery, OidcClient, OidcConfig, OidcRuntime};
use tellur_server::ratelimit::RateLimiter;
use tellur_server::storage::{SqliteStore, Store};
use tellur_server::{AppState, Config, build_router};
use tower::ServiceExt;

const ISSUER: &str = "https://idp.test";

struct MockIdp;
impl OidcClient for MockIdp {
    fn discover(&self, _issuer: &str) -> anyhow::Result<Discovery> {
        Ok(Discovery {
            authorization_endpoint: format!("{ISSUER}/authorize"),
            token_endpoint: format!("{ISSUER}/token"),
            issuer: ISSUER.to_string(),
        })
    }
    fn exchange_code(
        &self,
        _t: &str,
        _c: &str,
        _r: &str,
        _ci: &str,
        _cs: &str,
        _v: &str,
    ) -> anyhow::Result<String> {
        anyhow::bail!("unused")
    }
}

struct Setup {
    state: AppState,
    store: Arc<SqliteStore>,
    org: String,
}

fn setup() -> Setup {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    store.migrate().unwrap();
    let org = store.create_org("A").unwrap().id;
    let runtime = OidcRuntime::new(
        OidcConfig {
            issuer: ISSUER.to_string(),
            client_id: "client-1".to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "https://hub.test/auth/callback".to_string(),
        },
        Arc::new(MockIdp),
    );
    let state = AppState {
        store: store.clone(),
        config: Arc::new(Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            db_path: ":memory:".into(),
            database_url: None,
            allow_non_loopback: false,
        }),
        rate_limiter: Arc::new(RateLimiter::new(10_000, Duration::from_secs(60))),
        metrics: Arc::new(tellur_server::Metrics::new()),
        oidc: Some(Arc::new(runtime)),
    };
    Setup { state, store, org }
}

async fn send(state: &AppState, req: Request<Body>) -> axum::response::Response {
    build_router(state.clone()).oneshot(req).await.unwrap()
}

async fn body_json(resp: axum::response::Response) -> Value {
    let b = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&b).unwrap()
}

async fn body_text(resp: axum::response::Response) -> String {
    let b = resp.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(b.to_vec()).unwrap()
}

/// Start a device flow; returns (device_code, user_code).
async fn authorize(s: &Setup) -> (String, String) {
    let resp = send(
        &s.state,
        Request::builder()
            .method("POST")
            .uri("/v1/device/authorize")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    (
        v["device_code"].as_str().unwrap().to_string(),
        v["user_code"].as_str().unwrap().to_string(),
    )
}

/// Poll the token endpoint once, returning (status, json).
async fn poll(s: &Setup, device_code: &str) -> (StatusCode, Value) {
    let resp = send(
        &s.state,
        Request::builder()
            .method("POST")
            .uri("/v1/device/token")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "device_code": device_code }).to_string(),
            ))
            .unwrap(),
    )
    .await;
    let status = resp.status();
    (status, body_json(resp).await)
}

/// Submit an approve/deny decision with a signed-in session cookie.
async fn decide(s: &Setup, session: &str, user_code: &str, decision: &str) -> StatusCode {
    let body = format!("user_code={user_code}&decision={decision}");
    send(
        &s.state,
        Request::builder()
            .method("POST")
            .uri("/auth/device/decision")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("cookie", format!("tellur_session={session}"))
            .body(Body::from(body))
            .unwrap(),
    )
    .await
    .status()
}

#[tokio::test]
async fn device_login_happy_path_mints_working_token() {
    let s = setup();
    let member = s
        .store
        .provision_member(&s.org, "Alice", Role::Admin, "alice@corp.test")
        .unwrap();
    let session = s.store.create_session(&member, 3600).unwrap();

    let (device_code, user_code) = authorize(&s).await;

    // Before approval the CLI gets a pending signal (400 + error code).
    let (status, body) = poll(&s, &device_code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "authorization_pending");

    // The approval page renders the code for a signed-in member.
    let resp = send(
        &s.state,
        Request::builder()
            .uri(format!("/auth/device?user_code={user_code}"))
            .header("cookie", format!("tellur_session={session}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_text(resp).await;
    assert!(html.contains(&user_code));
    assert!(html.contains("Authorize"));

    // Approve, then the next poll returns a real token bound to the member.
    assert_eq!(
        decide(&s, &session, &user_code, "approve").await,
        StatusCode::OK
    );
    let (status, body) = poll(&s, &device_code).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["org_id"], s.org);
    assert_eq!(body["role"], "admin");
    let token = body["access_token"].as_str().unwrap().to_string();
    assert!(!token.is_empty());

    // The minted token authenticates the API as the approving member.
    let resp = send(
        &s.state,
        Request::builder()
            .uri("/v1/me")
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["member_id"], member);

    // The row is consumed: a second poll no longer finds the request.
    let (status, body) = poll(&s, &device_code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "expired_token");
}

#[tokio::test]
async fn denied_request_reports_access_denied() {
    let s = setup();
    let member = s
        .store
        .provision_member(&s.org, "Bob", Role::Viewer, "bob@corp.test")
        .unwrap();
    let session = s.store.create_session(&member, 3600).unwrap();
    let (device_code, user_code) = authorize(&s).await;

    assert_eq!(
        decide(&s, &session, &user_code, "deny").await,
        StatusCode::OK
    );
    let (status, body) = poll(&s, &device_code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "access_denied");
}

#[tokio::test]
async fn approval_page_redirects_to_login_when_signed_out() {
    let s = setup();
    let (_device, user_code) = authorize(&s).await;
    let resp = send(
        &s.state,
        Request::builder()
            .uri(format!("/auth/device?user_code={user_code}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers().get("location").unwrap(), "/auth/login");
    // A return cookie remembers the device page so login bounces back here.
    let set = resp.headers().get("set-cookie").unwrap().to_str().unwrap();
    assert!(set.contains("tellur_return="));
    assert!(set.contains("%2Fauth%2Fdevice") || set.contains("/auth/device"));
}

#[tokio::test]
async fn decision_without_session_is_unauthorized() {
    let s = setup();
    let (_d, user_code) = authorize(&s).await;
    // No session cookie → the SameSite-Lax CSRF defense rejects the POST.
    let body = format!("user_code={user_code}&decision=approve");
    let resp = send(
        &s.state,
        Request::builder()
            .method("POST")
            .uri("/auth/device/decision")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unknown_device_code_is_expired() {
    let s = setup();
    let (status, body) = poll(&s, "nope").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "expired_token");
}

#[tokio::test]
async fn device_endpoints_404_when_sso_disabled() {
    let mut s = setup();
    s.state.oidc = None;
    let resp = send(
        &s.state,
        Request::builder()
            .method("POST")
            .uri("/v1/device/authorize")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
