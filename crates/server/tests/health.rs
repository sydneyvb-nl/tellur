//! Integration tests for the B0 operational endpoints.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tellur_server::storage::{SqliteStore, Store};
use tellur_server::{AppState, Config, build_router};
use tower::ServiceExt; // for `oneshot`

fn test_state() -> AppState {
    let store = SqliteStore::open_in_memory().unwrap();
    store.migrate().unwrap();
    state_with_store(store)
}

fn state_with_store(store: SqliteStore) -> AppState {
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        db_path: ":memory:".into(),
        database_url: None,
        allow_non_loopback: false,
    };
    AppState {
        store: Arc::new(store),
        config: Arc::new(config),
        rate_limiter: Arc::new(tellur_server::ratelimit::RateLimiter::new(
            1000,
            std::time::Duration::from_secs(60),
        )),
        metrics: Arc::new(tellur_server::Metrics::new()),
    }
}

#[tokio::test]
async fn healthz_returns_ok() {
    let app = build_router(test_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["service"], "tellur-server");
}

#[tokio::test]
async fn readyz_returns_ready_when_store_healthy() {
    let app = build_router(test_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["status"], "ready");
}

#[tokio::test]
async fn readyz_returns_unavailable_when_store_is_not_migrated() {
    let app = build_router(state_with_store(SqliteStore::open_in_memory().unwrap()));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/readyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["status"], "not_ready");
}

#[tokio::test]
async fn metrics_endpoint_exposes_counters() {
    let app = build_router(test_state());
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("tellur_ingest_events_total"));
    assert!(text.contains("# TYPE tellur_exports_total counter"));
}

#[tokio::test]
async fn unknown_route_is_404() {
    let app = build_router(test_state());
    let resp = app
        .oneshot(Request::builder().uri("/nope").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
