//! Dashboard SPA routing: `/app/*` serves the app shell (with client-side
//! routing fallback), real-asset misses 404, and `/v1/*` is unaffected.
//!
//! These assert behavior that holds whether or not `ui/dist` was built: when it
//! is empty the hub serves a placeholder HTML at `/app`; when built it serves
//! the SPA's `index.html`. Either way `/app` and client routes are `200 text/html`.

#![cfg(feature = "dashboard")]

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode, header::CONTENT_TYPE};
use tellur_server::ratelimit::RateLimiter;
use tellur_server::storage::{SqliteStore, Store};
use tellur_server::{AppState, Config, build_router};
use tower::ServiceExt;

fn state() -> AppState {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    store.migrate().unwrap();
    AppState {
        store,
        config: Arc::new(Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            db_path: ":memory:".into(),
            database_url: None,
            allow_non_loopback: false,
        }),
        rate_limiter: Arc::new(RateLimiter::new(1000, Duration::from_secs(60))),
        metrics: Arc::new(tellur_server::Metrics::new()),
        oidc: None,
    }
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, Option<String>) {
    let (status, ct, _csp) = get_full(state, uri).await;
    (status, ct)
}

async fn get_full(state: &AppState, uri: &str) -> (StatusCode, Option<String>, Option<String>) {
    let resp = build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let header = |name: axum::http::HeaderName| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let ct = header(CONTENT_TYPE);
    let csp = header(axum::http::header::CONTENT_SECURITY_POLICY);
    (status, ct, csp)
}

#[tokio::test]
async fn app_root_serves_html_with_csp() {
    let (status, ct, csp) = get_full(&state(), "/app").await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.unwrap_or_default().contains("text/html"));
    // The app shell must carry the strict same-origin CSP.
    let csp = csp.expect("CSP header present on the app shell");
    assert!(csp.contains("default-src 'self'"));
    assert!(csp.contains("frame-ancestors 'none'"));
}

#[tokio::test]
async fn client_route_falls_back_to_app_shell() {
    // A deep link with no file extension is a client route → app shell, not 404.
    let (status, ct) = get(&state(), "/app/orgs/org_123/overview").await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.unwrap_or_default().contains("text/html"));
}

#[tokio::test]
async fn missing_asset_is_404() {
    // A path that looks like a real asset (has an extension) must not fall back.
    let (status, _) = get(&state(), "/app/assets/does-not-exist.js").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_routes_are_unaffected() {
    // Unknown /v1 paths still 404 (JSON API), not the SPA shell.
    let (status, ct) = get(&state(), "/v1/nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_ne!(ct.unwrap_or_default(), "text/html; charset=utf-8");
    // Operational endpoints keep working.
    let (status, _) = get(&state(), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
}
