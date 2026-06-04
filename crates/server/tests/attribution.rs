//! Attribution ingest + SLSA/SPDX export integration tests.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{
    Request, StatusCode,
    header::{AUTHORIZATION, CONTENT_TYPE},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tellur_core::schema::types::{
    AttributionRange, AttributionState, EvidenceStrength, FileAttribution, Origin,
};
use tellur_server::auth::Role;
use tellur_server::ratelimit::RateLimiter;
use tellur_server::storage::{SqliteStore, Store};
use tellur_server::{AppState, Config, build_router};
use tower::ServiceExt;

struct Setup {
    state: AppState,
    org_a: String,
    admin_a: String,
    contributor_a: String,
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
    let contributor_a = token(&store, &org_a, "carl", Role::Contributor);
    let viewer_a = token(&store, &org_a, "vic", Role::Viewer);
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
        contributor_a,
        viewer_a,
        admin_b,
    }
}

fn ai_attribution() -> FileAttribution {
    FileAttribution {
        schema: "tellur.attribution.v1".to_string(),
        file_path: "src/auth/session.rs".to_string(),
        git_blob_sha: "a91c".to_string(),
        ranges: vec![AttributionRange {
            range_id: "rng1".to_string(),
            start_line: 10,
            end_line: 40,
            origin: Origin::Ai,
            evidence_strength: EvidenceStrength::Recorded,
            confidence: 0.95,
            state: AttributionState::Exact,
            session_id: "sess1".to_string(),
            event_ids: vec![],
            agent_id: "claude-code".to_string(),
            model_id: Some("claude-opus-4.7".to_string()),
            prompt_hash: None,
            context_set_id: None,
            policy_tags: vec![],
            risk_tags: vec![],
            risk_level: None,
            tests_run: vec![],
            tests_passed: false,
            reviewer: None,
            reviewed_at: None,
        }],
        updated_at: "2026-06-04T00:00:00Z".to_string(),
    }
}

async fn req(
    state: &AppState,
    method: &str,
    uri: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = bearer {
        b = b.header(AUTHORIZATION, format!("Bearer {t}"));
    }
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

fn ingest_body() -> Value {
    json!({ "attributions": [ai_attribution()] })
}

#[tokio::test]
async fn contributor_ingests_attribution_then_admin_exports_slsa_and_spdx() {
    let s = setup();
    let base = format!("/v1/orgs/{}/repos/app", s.org_a);

    let (status, json) = req(
        &s.state,
        "POST",
        &format!("{base}/attributions"),
        Some(&s.contributor_a),
        Some(ingest_body()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["files"], 1);

    // SLSA export reflects the ingested AI attribution.
    let (status, slsa) = req(
        &s.state,
        "GET",
        &format!("{base}/export/slsa"),
        Some(&s.admin_a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(slsa["_type"], "https://in-toto.io/Statement/v1");
    let materials = slsa["predicate"]["materials"].as_array().unwrap();
    assert_eq!(materials.len(), 1);
    assert_eq!(materials[0]["ai_model"], "claude-opus-4.7");

    // SPDX export builds too.
    let (status, spdx) = req(
        &s.state,
        "GET",
        &format!("{base}/export/spdx"),
        Some(&s.admin_a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(spdx["spdx_version"].as_str().is_some());
}

#[tokio::test]
async fn attribution_ingest_respects_role_and_tenant() {
    let s = setup();
    let uri = format!("/v1/orgs/{}/repos/app/attributions", s.org_a);
    // Viewer cannot ingest.
    let (status, _) = req(
        &s.state,
        "POST",
        &uri,
        Some(&s.viewer_a),
        Some(ingest_body()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Org B admin cannot ingest into org A.
    let (status, _) = req(
        &s.state,
        "POST",
        &uri,
        Some(&s.admin_b),
        Some(ingest_body()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn slsa_export_requires_admin_and_existing_repo() {
    let s = setup();
    // Viewer can't export.
    let (status, _) = req(
        &s.state,
        "GET",
        &format!("/v1/orgs/{}/repos/app/export/slsa", s.org_a),
        Some(&s.viewer_a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Admin, but repo doesn't exist yet → 404.
    let (status, _) = req(
        &s.state,
        "GET",
        &format!("/v1/orgs/{}/repos/missing/export/slsa", s.org_a),
        Some(&s.admin_a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
