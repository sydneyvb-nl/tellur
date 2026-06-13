//! OIDC SSO flow integration tests, driven through the router with a mock IdP
//! client (no network). Exercises the full login → callback → session →
//! authenticated request → logout path plus the security rejections.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header::AUTHORIZATION};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http_body_util::BodyExt;
use serde_json::Value;
use tellur_server::auth::Role;
use tellur_server::oidc::{Discovery, OidcClient, OidcConfig, OidcRuntime};
use tellur_server::ratelimit::RateLimiter;
use tellur_server::storage::{SqliteStore, Store};
use tellur_server::{AppState, Config, build_router};
use tower::ServiceExt;

const ISSUER: &str = "https://idp.test";
const CLIENT_ID: &str = "client-1";

/// A mock IdP: canned discovery + a configurable next ID token to return from
/// the code exchange.
struct MockIdp {
    next_id_token: Mutex<Option<String>>,
}

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
        _token_endpoint: &str,
        _code: &str,
        _redirect_uri: &str,
        _client_id: &str,
        _client_secret: &str,
        _pkce_verifier: &str,
    ) -> anyhow::Result<String> {
        self.next_id_token
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| anyhow::anyhow!("no id token configured"))
    }
}

struct Setup {
    state: AppState,
    store: Arc<SqliteStore>,
    idp: Arc<MockIdp>,
    org: String,
}

fn setup() -> Setup {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    store.migrate().unwrap();
    let org = store.create_org("A").unwrap().id;

    let idp = Arc::new(MockIdp {
        next_id_token: Mutex::new(None),
    });
    let runtime = OidcRuntime::new(
        OidcConfig {
            issuer: ISSUER.to_string(),
            client_id: CLIENT_ID.to_string(),
            client_secret: "secret".to_string(),
            redirect_uri: "https://hub.test/auth/callback".to_string(),
            allow_insecure_http: false,
        },
        idp.clone(),
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
        github_app: None,
    };
    Setup {
        state,
        store,
        idp,
        org,
    }
}

/// A self-issued (unverified-signature) ID token, as the test IdP would mint.
fn id_token(sub: &str, email: &str, email_verified: bool, nonce: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let payload = serde_json::json!({
        "iss": ISSUER,
        "sub": sub,
        "aud": CLIENT_ID,
        "exp": chrono::Utc::now().timestamp() + 3600,
        "nonce": nonce,
        "email": email,
        "email_verified": email_verified,
    });
    let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
    let sig = URL_SAFE_NO_PAD.encode(b"sig");
    format!("{header}.{body}.{sig}")
}

async fn send(state: &AppState, req: Request<Body>) -> axum::response::Response {
    build_router(state.clone()).oneshot(req).await.unwrap()
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    for pair in q.split('&') {
        if let Some((k, v)) = pair.split_once('=')
            && k == key
        {
            return Some(v.to_string());
        }
    }
    None
}

/// Drive login → callback and return the session cookie value.
async fn login_get_session(
    s: &Setup,
    sub: &str,
    email: &str,
    verified: bool,
) -> (StatusCode, Option<String>) {
    // 1. /auth/login → 302 to the IdP authorize URL (carries state + nonce).
    let resp = send(
        &s.state,
        Request::builder()
            .uri("/auth/login")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let location = resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let state_tok = query_param(&location, "state").unwrap();
    let nonce = query_param(&location, "nonce").unwrap();
    // The login response sets a browser-binding cookie we must echo back.
    let login_cookie = resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|c| c.split(';').next())
        .unwrap()
        .to_string();

    // 2. The IdP would now mint an ID token bound to our nonce.
    *s.idp.next_id_token.lock().unwrap() = Some(id_token(sub, email, verified, &nonce));

    // 3. /auth/callback → consumes the code, sets the session cookie.
    let resp = send(
        &s.state,
        Request::builder()
            .uri(format!("/auth/callback?code=abc&state={state_tok}"))
            .header("cookie", login_cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let status = resp.status();
    let cookie = resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|c| c.split(';').next())
        .and_then(|kv| kv.strip_prefix("tellur_session="))
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    (status, cookie)
}

#[tokio::test]
async fn full_sso_login_creates_session_and_authenticates() {
    let s = setup();
    let member = s
        .store
        .provision_member(&s.org, "Alice", Role::Admin, "alice@corp.test")
        .unwrap();

    let (status, cookie) = login_get_session(&s, "idp-sub-1", "alice@corp.test", true).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let cookie = cookie.expect("session cookie set");

    // The session authenticates /v1/me.
    let resp = send(
        &s.state,
        Request::builder()
            .uri("/v1/me")
            .header("cookie", format!("tellur_session={cookie}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = {
        let b = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&b).unwrap()
    };
    assert_eq!(body["member_id"], member);
    assert_eq!(body["role"], "admin");

    // The OIDC subject was bound, so a second login matches by subject.
    let (status2, cookie2) = login_get_session(&s, "idp-sub-1", "changed@corp.test", false).await;
    assert_eq!(status2, StatusCode::SEE_OTHER);
    assert!(cookie2.is_some());

    // Logout clears the session.
    let resp = send(
        &s.state,
        Request::builder()
            .uri("/auth/logout")
            .header("cookie", format!("tellur_session={cookie}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let resp = send(
        &s.state,
        Request::builder()
            .uri("/v1/me")
            .header("cookie", format!("tellur_session={cookie}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unprovisioned_or_unverified_email_is_rejected() {
    let s = setup();
    // No member provisioned for this email → 403.
    let (status, cookie) = login_get_session(&s, "sub-x", "stranger@corp.test", true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(cookie.is_none());

    // Provisioned, but the IdP says the email is unverified → 403.
    s.store
        .provision_member(&s.org, "Bob", Role::Viewer, "bob@corp.test")
        .unwrap();
    let (status, _) = login_get_session(&s, "sub-y", "bob@corp.test", false).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn second_idp_subject_for_same_email_cannot_take_over() {
    let s = setup();
    s.store
        .provision_member(&s.org, "Alice", Role::Admin, "alice@corp.test")
        .unwrap();
    // First login binds subject A.
    let (status, _) = login_get_session(&s, "subject-A", "alice@corp.test", true).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    // A different IdP subject with the same verified email is refused (403).
    let (status, cookie) = login_get_session(&s, "subject-B", "alice@corp.test", true).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(cookie.is_none());
}

#[tokio::test]
async fn callback_without_browser_binding_cookie_is_rejected() {
    let s = setup();
    s.store
        .provision_member(&s.org, "Alice", Role::Admin, "alice@corp.test")
        .unwrap();
    // Initiate login, capture the state but NOT the binding cookie (as an
    // attacker who forwards only the callback URL to a victim would).
    let resp = send(
        &s.state,
        Request::builder()
            .uri("/auth/login")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let location = resp.headers().get("location").unwrap().to_str().unwrap();
    let state_tok = query_param(location, "state").unwrap();
    let nonce = query_param(location, "nonce").unwrap();
    *s.idp.next_id_token.lock().unwrap() = Some(id_token("sub", "alice@corp.test", true, &nonce));

    // Callback without the login cookie is refused (login-CSRF defense).
    let resp = send(
        &s.state,
        Request::builder()
            .uri(format!("/auth/callback?code=abc&state={state_tok}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn callback_with_unknown_state_is_rejected() {
    let s = setup();
    // No prior /auth/login → state is unknown (CSRF / replay).
    *s.idp.next_id_token.lock().unwrap() = Some(id_token("s", "a@b.test", true, "n"));
    let resp = send(
        &s.state,
        Request::builder()
            .uri("/auth/callback?code=abc&state=forged")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sso_routes_404_when_disabled() {
    let mut s = setup();
    // Disable SSO.
    s.state.oidc = None;
    let resp = send(
        &s.state,
        Request::builder()
            .uri("/auth/login")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn bearer_token_still_authenticates_alongside_sessions() {
    let s = setup();
    let m = s
        .store
        .create_member(&s.org, "svc", Role::Contributor)
        .unwrap();
    let token = s.store.create_token(&m).unwrap().plaintext;
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
}
