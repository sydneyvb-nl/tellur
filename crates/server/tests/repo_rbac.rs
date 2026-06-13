//! Fine-grained per-repo RBAC (B6a) integration tests.
//!
//! Per-repo grants are **additive**: a member's effective role on a repo is
//! `max(org_role, repo_grant)`. Grants elevate, never restrict.

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

struct Member {
    token: String,
    id: String,
}

struct Setup {
    state: AppState,
    org_a: String,
    admin_a: Member,
    contributor_a: Member,
    viewer_a: Member,
    viewer2_a: Member,
    member_b: Member,
}

fn member(store: &SqliteStore, org: &str, name: &str, role: Role) -> Member {
    let id = store.create_member(org, name, role).unwrap();
    let token = store.create_token(&id).unwrap().plaintext;
    Member { token, id }
}

fn setup() -> Setup {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    store.migrate().unwrap();
    let org_a = store.create_org("A").unwrap().id;
    let admin_a = member(&store, &org_a, "alice", Role::Admin);
    let contributor_a = member(&store, &org_a, "carl", Role::Contributor);
    let viewer_a = member(&store, &org_a, "vic", Role::Viewer);
    let viewer2_a = member(&store, &org_a, "val", Role::Viewer);
    let org_b = store.create_org("B").unwrap().id;
    let member_b = member(&store, &org_b, "bob", Role::Admin);

    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        db_path: ":memory:".into(),
        database_url: None,
        allow_non_loopback: false,
    };
    let state = AppState {
        store,
        config: Arc::new(config),
        rate_limiter: Arc::new(RateLimiter::new(10_000, Duration::from_secs(60))),
        metrics: Arc::new(tellur_server::Metrics::new()),
        oidc: None,
        github_app: None,
    };
    Setup {
        state,
        org_a,
        admin_a,
        contributor_a,
        viewer_a,
        viewer2_a,
        member_b,
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
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

fn events() -> Value {
    json!({ "events": [{ "session_id": "s1", "type": "edit", "actor": "claude" }] })
}

#[tokio::test]
async fn per_repo_contributor_grant_elevates_a_viewer() {
    let s = setup();
    // Contributor creates repo "app".
    let app_events = format!("/v1/orgs/{}/repos/app/events", s.org_a);
    let (status, _) = req(
        &s.state,
        "POST",
        &app_events,
        &s.contributor_a.token,
        Some(events()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // A plain org viewer cannot ingest.
    let (status, _) = req(
        &s.state,
        "POST",
        &app_events,
        &s.viewer_a.token,
        Some(events()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Admin grants the viewer a contributor role on "app".
    let grant_uri = format!("/v1/orgs/{}/repos/app/roles/{}", s.org_a, s.viewer_a.id);
    let (status, _) = req(
        &s.state,
        "PUT",
        &grant_uri,
        &s.admin_a.token,
        Some(json!({ "role": "contributor" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Now the viewer can ingest to "app".
    let (status, _) = req(
        &s.state,
        "POST",
        &app_events,
        &s.viewer_a.token,
        Some(events()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // ...but the grant is scoped to "app": the viewer still cannot create or
    // write a different repo.
    let other = format!("/v1/orgs/{}/repos/other/events", s.org_a);
    let (status, _) = req(&s.state, "POST", &other, &s.viewer_a.token, Some(events())).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Revoking the grant restores the viewer's baseline (denied again).
    let (status, body) = req(&s.state, "DELETE", &grant_uri, &s.admin_a.token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["removed"], true);
    let (status, _) = req(
        &s.state,
        "POST",
        &app_events,
        &s.viewer_a.token,
        Some(events()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn unauthorized_write_is_denied_before_batch_validation() {
    let s = setup();
    // Contributor creates repo "app".
    let app_events = format!("/v1/orgs/{}/repos/app/events", s.org_a);
    req(
        &s.state,
        "POST",
        &app_events,
        &s.contributor_a.token,
        Some(events()),
    )
    .await;

    // A viewer (no grant) sending an *empty* batch must be denied (403), not get
    // a 400 for the batch — authorization runs before request validation so the
    // attempt is recorded.
    let before = s.state.store.audit_len().unwrap();
    let (status, _) = req(
        &s.state,
        "POST",
        &app_events,
        &s.viewer_a.token,
        Some(json!({ "events": [] })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        s.state.store.audit_len().unwrap() > before,
        "denied write must be audited"
    );
}

#[tokio::test]
async fn per_repo_admin_grant_allows_export() {
    let s = setup();
    let base = format!("/v1/orgs/{}/repos/app", s.org_a);
    // Seed the repo with an event so it exists.
    let (status, _) = req(
        &s.state,
        "POST",
        &format!("{base}/events"),
        &s.contributor_a.token,
        Some(events()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Viewer cannot export (admin-level).
    let (status, _) = req(
        &s.state,
        "GET",
        &format!("{base}/export/slsa"),
        &s.viewer_a.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Grant the viewer admin on this repo.
    let (status, _) = req(
        &s.state,
        "PUT",
        &format!("/v1/orgs/{}/repos/app/roles/{}", s.org_a, s.viewer_a.id),
        &s.admin_a.token,
        Some(json!({ "role": "admin" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Now the viewer can export SLSA for this repo.
    let (status, body) = req(
        &s.state,
        "GET",
        &format!("{base}/export/slsa"),
        &s.viewer_a.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.get("predicateType").is_some() || body.get("_type").is_some());
}

#[tokio::test]
async fn role_management_is_admin_only_and_tenant_scoped() {
    let s = setup();
    // Create repo "app".
    req(
        &s.state,
        "POST",
        &format!("/v1/orgs/{}/repos/app/events", s.org_a),
        &s.contributor_a.token,
        Some(events()),
    )
    .await;

    let grant_uri = format!("/v1/orgs/{}/repos/app/roles/{}", s.org_a, s.viewer_a.id);

    // Contributor cannot grant roles.
    let (status, _) = req(
        &s.state,
        "PUT",
        &grant_uri,
        &s.contributor_a.token,
        Some(json!({ "role": "contributor" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Admin from another org cannot reach this org's repo (cross-tenant).
    let (status, _) = req(
        &s.state,
        "PUT",
        &grant_uri,
        &s.member_b.token,
        Some(json!({ "role": "contributor" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Granting to a member of a different org is refused (member not in org).
    let cross = format!("/v1/orgs/{}/repos/app/roles/{}", s.org_a, s.member_b.id);
    let (status, _) = req(
        &s.state,
        "PUT",
        &cross,
        &s.admin_a.token,
        Some(json!({ "role": "contributor" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Granting on a nonexistent repo is a 404.
    let (status, _) = req(
        &s.state,
        "PUT",
        &format!("/v1/orgs/{}/repos/ghost/roles/{}", s.org_a, s.viewer_a.id),
        &s.admin_a.token,
        Some(json!({ "role": "contributor" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // An unknown role string is a 400.
    let (status, _) = req(
        &s.state,
        "PUT",
        &grant_uri,
        &s.admin_a.token,
        Some(json!({ "role": "superuser" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Admin grants two roles, then lists them.
    req(
        &s.state,
        "PUT",
        &grant_uri,
        &s.admin_a.token,
        Some(json!({ "role": "contributor" })),
    )
    .await;
    req(
        &s.state,
        "PUT",
        &format!("/v1/orgs/{}/repos/app/roles/{}", s.org_a, s.viewer2_a.id),
        &s.admin_a.token,
        Some(json!({ "role": "admin" })),
    )
    .await;
    let (status, body) = req(
        &s.state,
        "GET",
        &format!("/v1/orgs/{}/repos/app/roles", s.org_a),
        &s.admin_a.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let grants = body["grants"].as_array().unwrap();
    assert_eq!(grants.len(), 2);
}
