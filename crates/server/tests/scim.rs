//! SCIM 2.0 provisioning integration tests, driven through the router.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{
    Request, StatusCode,
    header::{AUTHORIZATION, CONTENT_TYPE},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tellur_server::ratelimit::RateLimiter;
use tellur_server::storage::{SqliteStore, Store};
use tellur_server::{AppState, Config, build_router};
use tower::ServiceExt;

struct Setup {
    state: AppState,
    store: Arc<SqliteStore>,
    scim_token: String,
}

fn setup() -> Setup {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    store.migrate().unwrap();
    let org = store.create_org("A").unwrap().id;
    let scim_token = store.create_scim_token(&org).unwrap().plaintext;
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
        oidc: None,
    };
    Setup {
        state,
        store,
        scim_token,
    }
}

async fn scim(
    state: &AppState,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(t) = token {
        b = b.header(AUTHORIZATION, format!("Bearer {t}"));
    }
    let body = match body {
        Some(v) => {
            b = b.header(CONTENT_TYPE, "application/scim+json");
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
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

fn user_body(email: &str, role: &str) -> Value {
    json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"],
        "userName": email,
        "displayName": "Test User",
        "active": true,
        "roles": [{ "value": role }],
        "externalId": "ext-1",
    })
}

#[tokio::test]
async fn provision_list_get_and_deactivate() {
    let s = setup();

    // Unauthenticated and bad-token requests are refused.
    let (status, _) = scim(&s.state, "GET", "/scim/v2/Users", None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = scim(
        &s.state,
        "GET",
        "/scim/v2/Users",
        Some("tlr_bad_token"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Create.
    let (status, body) = scim(
        &s.state,
        "POST",
        "/scim/v2/Users",
        Some(&s.scim_token),
        Some(user_body("alice@corp.test", "admin")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().unwrap().to_string();
    assert_eq!(body["userName"], "alice@corp.test");
    assert_eq!(body["active"], true);
    assert_eq!(body["roles"][0]["value"], "admin");

    // The provisioned member is resolvable (active) by email.
    assert!(
        s.store
            .find_member_by_email("alice@corp.test")
            .unwrap()
            .is_some()
    );

    // Duplicate userName → 409.
    let (status, _) = scim(
        &s.state,
        "POST",
        "/scim/v2/Users",
        Some(&s.scim_token),
        Some(user_body("alice@corp.test", "viewer")),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // List + filter.
    let (status, body) = scim(
        &s.state,
        "GET",
        "/scim/v2/Users?filter=userName%20eq%20%22alice@corp.test%22",
        Some(&s.scim_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["totalResults"], 1);

    // Get by id.
    let (status, body) = scim(
        &s.state,
        "GET",
        &format!("/scim/v2/Users/{id}"),
        Some(&s.scim_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], id);

    // Deprovision (DELETE → deactivate).
    let (status, _) = scim(
        &s.state,
        "DELETE",
        &format!("/scim/v2/Users/{id}"),
        Some(&s.scim_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Deactivated members no longer resolve for auth (SSO email lookup).
    assert!(
        s.store
            .find_member_by_email("alice@corp.test")
            .unwrap()
            .is_none()
    );
    // ...but they still appear in the SCIM directory as active=false.
    let (status, body) = scim(
        &s.state,
        "GET",
        &format!("/scim/v2/Users/{id}"),
        Some(&s.scim_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["active"], false);
}

#[tokio::test]
async fn patch_reactivates_and_changes_role() {
    let s = setup();
    let (_, body) = scim(
        &s.state,
        "POST",
        "/scim/v2/Users",
        Some(&s.scim_token),
        Some(user_body("bob@corp.test", "viewer")),
    )
    .await;
    let id = body["id"].as_str().unwrap().to_string();

    // PATCH: deactivate.
    let (status, body) = scim(
        &s.state,
        "PATCH",
        &format!("/scim/v2/Users/{id}"),
        Some(&s.scim_token),
        Some(json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [{ "op": "replace", "path": "active", "value": false }],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["active"], false);

    // PATCH: reactivate + elevate role.
    let (status, body) = scim(
        &s.state,
        "PATCH",
        &format!("/scim/v2/Users/{id}"),
        Some(&s.scim_token),
        Some(json!({
            "schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"],
            "Operations": [
                { "op": "replace", "path": "active", "value": true },
                { "op": "replace", "path": "roles", "value": [{ "value": "contributor" }] },
            ],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["active"], true);
    assert_eq!(body["roles"][0]["value"], "contributor");
    let p = s
        .store
        .find_member_by_email("bob@corp.test")
        .unwrap()
        .unwrap();
    assert_eq!(p.role, tellur_server::auth::Role::Contributor);
}

#[tokio::test]
async fn scim_token_is_tenant_scoped() {
    let s = setup();
    // A second org with its own token cannot see org A's users.
    let org_b = s.store.create_org("B").unwrap().id;
    let token_b = s.store.create_scim_token(&org_b).unwrap().plaintext;

    scim(
        &s.state,
        "POST",
        "/scim/v2/Users",
        Some(&s.scim_token),
        Some(user_body("carol@corp.test", "viewer")),
    )
    .await;

    let (status, body) = scim(&s.state, "GET", "/scim/v2/Users", Some(&token_b), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["totalResults"], 0);
}

#[tokio::test]
async fn case_insensitive_filter_and_patch_and_email_change() {
    let s = setup();
    let (_, body) = scim(
        &s.state,
        "POST",
        "/scim/v2/Users",
        Some(&s.scim_token),
        Some(user_body("dave@corp.test", "viewer")),
    )
    .await;
    let id = body["id"].as_str().unwrap().to_string();

    // Case-insensitive filter (attr/operator) still matches.
    let (status, body) = scim(
        &s.state,
        "GET",
        "/scim/v2/Users?filter=Username%20EQ%20%22dave@corp.test%22",
        Some(&s.scim_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["totalResults"], 1);

    // Case-variant PATCH path ("Active") still deactivates.
    let (status, body) = scim(
        &s.state,
        "PATCH",
        &format!("/scim/v2/Users/{id}"),
        Some(&s.scim_token),
        Some(json!({ "Operations": [{ "op": "Replace", "path": "Active", "value": false }] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["active"], false);
    assert!(
        s.store
            .find_member_by_email("dave@corp.test")
            .unwrap()
            .is_none()
    );

    // PUT renames the account's email; the new address is then resolvable.
    let (status, _) = scim(
        &s.state,
        "PUT",
        &format!("/scim/v2/Users/{id}"),
        Some(&s.scim_token),
        Some(json!({
            "userName": "dave2@corp.test",
            "displayName": "Dave Two",
            "active": true,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        s.store
            .find_member_by_email("dave2@corp.test")
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn scim_mutations_are_audited() {
    let s = setup();
    let org = s.store.create_org("Audited").unwrap().id;
    let token = s.store.create_scim_token(&org).unwrap().plaintext;
    let before = s.store.export_audit(&org).unwrap().len();
    scim(
        &s.state,
        "POST",
        "/scim/v2/Users",
        Some(&token),
        Some(user_body("ed@corp.test", "viewer")),
    )
    .await;
    let after = s.store.export_audit(&org).unwrap();
    assert!(after.len() > before);
    assert!(after.iter().any(|r| r.action == "scim.user.create"));
}

#[tokio::test]
async fn list_honors_pagination() {
    let s = setup();
    for i in 0..5 {
        scim(
            &s.state,
            "POST",
            "/scim/v2/Users",
            Some(&s.scim_token),
            Some(user_body(&format!("u{i}@corp.test"), "viewer")),
        )
        .await;
    }
    let (status, body) = scim(
        &s.state,
        "GET",
        "/scim/v2/Users?startIndex=2&count=2",
        Some(&s.scim_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["totalResults"], 5);
    assert_eq!(body["startIndex"], 2);
    assert_eq!(body["Resources"].as_array().unwrap().len(), 2);
}
