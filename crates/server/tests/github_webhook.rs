//! GitHub App P3 integration tests: signed webhook delivery, repo auto-provision,
//! and idempotent notes harvesting.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use axum::body::Body;
use axum::http::{Request, StatusCode, header::CONTENT_TYPE};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tellur_core::notes::{IndexedAttribution, render_git_ai_note};
use tellur_core::schema::types::{AttributionRange, AttributionState, EvidenceStrength, Origin};
use tellur_server::github_app::{
    GitCommitObject, GitObjectRef, GitTreeEntry, GitTreeObject, GithubAppApi, GithubRepository,
    InstallationToken,
};
use tellur_server::ratelimit::RateLimiter;
use tellur_server::storage::{SqliteStore, Store};
use tellur_server::{AppState, Config, build_router};
use tower::ServiceExt;

const TEST_KEY_B64: &str = include_str!("data/github_app_test_key.pem.b64");
const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn test_key() -> String {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(TEST_KEY_B64.trim())
        .unwrap();
    String::from_utf8(bytes).unwrap()
}

struct MockGithubApi;

impl GithubAppApi for MockGithubApi {
    fn installation_id(&self, _: &str, _: &str, _: &str, _: &str) -> Result<u64> {
        Ok(99)
    }

    fn installation_token(
        &self,
        _: &str,
        _: &str,
        _: u64,
        repo: &str,
    ) -> Result<InstallationToken> {
        Ok(InstallationToken {
            token: format!("ghs_{repo}"),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        })
    }

    fn installation_repositories(&self, _: &str, _: &str) -> Result<Vec<GithubRepository>> {
        Ok(vec![GithubRepository {
            full_name: "acme/app".into(),
            name: "app".into(),
            owner_login: "acme".into(),
            default_branch: "main".into(),
            private: true,
        }])
    }

    fn ref_object(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        git_ref: &str,
    ) -> Result<Option<GitObjectRef>> {
        assert_eq!(git_ref, "notes/ai");
        Ok(Some(GitObjectRef {
            sha: "notes_commit".into(),
        }))
    }

    fn commit_object(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        sha: &str,
    ) -> Result<GitCommitObject> {
        assert_eq!(sha, "notes_commit");
        Ok(GitCommitObject {
            tree_sha: "notes_tree".into(),
        })
    }

    fn tree(&self, _: &str, _: &str, _: &str, _: &str, sha: &str) -> Result<GitTreeObject> {
        assert_eq!(sha, "notes_tree");
        Ok(GitTreeObject {
            entries: vec![GitTreeEntry {
                path: "aa/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                kind: "blob".into(),
                sha: "note_blob".into(),
            }],
            truncated: false,
        })
    }

    fn blob_text(&self, _: &str, _: &str, _: &str, _: &str, sha: &str) -> Result<String> {
        assert_eq!(sha, "note_blob");
        render_git_ai_note(&[sample_attr()], COMMIT, "0.1.0")
    }
}

fn sample_attr() -> IndexedAttribution {
    IndexedAttribution {
        file_path: "src/main.rs".into(),
        git_blob_sha: "blob123".into(),
        range: AttributionRange {
            range_id: "rng_1".into(),
            start_line: 1,
            end_line: 3,
            origin: Origin::Ai,
            evidence_strength: EvidenceStrength::Recorded,
            confidence: 0.9,
            state: AttributionState::Exact,
            session_id: "sess_1".into(),
            event_ids: vec!["evt_1".into()],
            agent_id: "codex".into(),
            model_id: Some("gpt-5".into()),
            prompt_hash: Some("prompt_hash".into()),
            context_set_id: None,
            policy_tags: vec![],
            risk_tags: vec![],
            risk_level: None,
            tests_run: vec![],
            tests_passed: false,
            reviewer: None,
            reviewed_at: None,
        },
    }
}

fn setup() -> (AppState, String) {
    let store = Arc::new(SqliteStore::open_in_memory().unwrap());
    store.migrate().unwrap();
    let org = store.create_org("A").unwrap().id;
    store.set_github_installation(&org, 99, "acme").unwrap();
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
        github_app: Some(Arc::new(tellur_server::github_app::GithubAppRuntime::new(
            tellur_server::github_app::GithubAppConfig {
                app_id: "123".into(),
                private_key_pem: test_key(),
                api_base: "https://api.github.com".into(),
                webhook_secret: Some("secret".into()),
            },
            Arc::new(MockGithubApi),
        ))),
    };
    (state, org)
}

async fn webhook(state: &AppState, body: Value, event: &str, secret: &str) -> (StatusCode, Value) {
    let raw = body.to_string();
    let sig = hmac_sha256_hex(secret.as_bytes(), raw.as_bytes());
    let resp = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhook/github")
                .header(CONTENT_TYPE, "application/json")
                .header("x-github-event", event)
                .header("x-hub-signature-256", format!("sha256={sig}"))
                .body(Body::from(raw))
                .unwrap(),
        )
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
async fn signed_push_harvests_note_once_and_syncs_source() {
    let (state, org) = setup();
    let payload = json!({
        "installation": { "id": 99 },
        "repository": {
            "name": "app",
            "full_name": "acme/app",
            "default_branch": "main",
            "owner": { "login": "acme" }
        },
        "commits": [{ "id": COMMIT }]
    });

    let (st, body) = webhook(&state, payload.clone(), "push", "secret").await;
    assert_eq!(st, StatusCode::ACCEPTED);
    assert_eq!(body["repos_synced"], 1);
    assert_eq!(body["notes_imported"], 1);
    assert_eq!(body["notes_skipped"], 0);

    let repo = state.store.find_repo(&org, "acme/app").unwrap().unwrap();
    assert_eq!(state.store.event_count(&org, &repo.id).unwrap(), 1);
    let source = state.store.get_repo_source(&org, &repo.id).unwrap();
    assert_eq!(
        source.raw.as_deref(),
        Some("https://api.github.com/repos/acme/app/contents/{path}?ref=main")
    );

    let (st, body) = webhook(&state, payload, "push", "secret").await;
    assert_eq!(st, StatusCode::ACCEPTED);
    assert_eq!(body["notes_imported"], 0);
    assert_eq!(body["notes_skipped"], 1);
    assert_eq!(state.store.event_count(&org, &repo.id).unwrap(), 1);
}

#[tokio::test]
async fn invalid_signature_is_rejected_before_writing() {
    let (state, org) = setup();
    let payload = json!({
        "installation": { "id": 99 },
        "repository": {
            "name": "app",
            "full_name": "acme/app",
            "default_branch": "main",
            "owner": { "login": "acme" }
        },
        "commits": [{ "id": COMMIT }]
    });
    let (st, _) = webhook(&state, payload, "push", "wrong").await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert!(state.store.find_repo(&org, "acme/app").unwrap().is_none());
}

fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    const BLOCK: usize = 64;
    let mut key_block = [0u8; BLOCK];
    if key.len() > BLOCK {
        key_block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(msg);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner);
    outer
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}
