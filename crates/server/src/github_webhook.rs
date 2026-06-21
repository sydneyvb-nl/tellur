//! GitHub App webhooks for P3: repo discovery and notes harvesting.
//!
//! Inbound GitHub traffic is unauthenticated by Tellur bearer/session auth, so
//! tenancy is established in two steps: verify the GitHub HMAC signature, then
//! map the delivering installation id to a Tellur org. Only after both pass do
//! we provision repos or append harvested commit-note events.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tellur_core::notes::parse_git_ai_note;

use crate::app::AppState;
use crate::error::ServerError;
use crate::storage::{AuditEntry, IngestEvent};

const MAX_PUSH_COMMITS: usize = 100;

#[derive(Debug, Deserialize)]
struct InstallationWire {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct AccountWire {
    login: String,
}

#[derive(Debug, Deserialize)]
struct RepoWire {
    name: String,
    full_name: String,
    default_branch: String,
    owner: AccountWire,
}

#[derive(Debug, Deserialize)]
struct PushCommitWire {
    id: String,
}

#[derive(Debug, Deserialize)]
struct PushWebhook {
    installation: InstallationWire,
    repository: RepoWire,
    #[serde(default)]
    commits: Vec<PushCommitWire>,
}

#[derive(Debug, Deserialize)]
struct InstallationRepositoriesWebhook {
    installation: InstallationWire,
}

#[derive(Debug, Default, serde::Serialize)]
struct WebhookOutcome {
    event: String,
    repos_synced: usize,
    notes_imported: usize,
    notes_skipped: usize,
}

/// `POST /webhook/github` — GitHub App webhook endpoint.
///
/// Currently handles `push` (notes harvester) and `installation_repositories`
/// (repo discovery refresh). Unknown events are acknowledged so GitHub delivery
/// stays healthy while P4 adds PR Check Runs.
pub async fn github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), ServerError> {
    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    verify_signature(&state, &headers, &body)?;
    let outcome = match event.as_str() {
        "push" => handle_push(state, &body).await?,
        "installation_repositories" => handle_installation_repositories(state, &body).await?,
        _ => WebhookOutcome {
            event,
            ..WebhookOutcome::default()
        },
    };
    Ok((StatusCode::ACCEPTED, Json(json!(outcome))))
}

fn verify_signature(state: &AppState, headers: &HeaderMap, body: &[u8]) -> Result<(), ServerError> {
    let Some(secret) = state
        .github_app
        .as_ref()
        .and_then(|app| app.config.webhook_secret.as_deref())
    else {
        return Err(ServerError::Config(
            "TELLUR_GITHUB_WEBHOOK_SECRET is required for GitHub webhooks".into(),
        ));
    };
    let got = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("sha256="))
        .ok_or(ServerError::Unauthorized)?;
    let expected = hmac_sha256_hex(secret.as_bytes(), body);
    if !constant_time_eq(got.as_bytes(), expected.as_bytes()) {
        return Err(ServerError::Unauthorized);
    }
    Ok(())
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
    hex(&outer.finalize())
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

async fn handle_installation_repositories(
    state: AppState,
    body: &[u8],
) -> Result<WebhookOutcome, ServerError> {
    let payload: InstallationRepositoriesWebhook =
        serde_json::from_slice(body).map_err(|e| ServerError::BadRequest(e.to_string()))?;
    let org_id = mapped_org(&state, payload.installation.id)?;
    let repos = discover_repos(&state, payload.installation.id, "")?;
    let count = sync_repositories(&state, &org_id, &repos)?;
    audit_webhook(
        &state,
        &org_id,
        "github.installation_repositories",
        count,
        0,
    )?;
    Ok(WebhookOutcome {
        event: "installation_repositories".into(),
        repos_synced: count,
        notes_imported: 0,
        notes_skipped: 0,
    })
}

async fn handle_push(state: AppState, body: &[u8]) -> Result<WebhookOutcome, ServerError> {
    let payload: PushWebhook =
        serde_json::from_slice(body).map_err(|e| ServerError::BadRequest(e.to_string()))?;
    let org_id = mapped_org(&state, payload.installation.id)?;
    let repos = discover_repos(&state, payload.installation.id, &payload.repository.name)?;
    let repos_synced = sync_repositories(&state, &org_id, &repos)?;
    let repo = state
        .store
        .ensure_repo(&org_id, &payload.repository.full_name)
        .map_err(ServerError::Internal)?;
    sync_one_source(&state, &org_id, &repo.id, &payload.repository)?;

    let mut imported = 0usize;
    let mut skipped = 0usize;
    for commit in payload.commits.iter().take(MAX_PUSH_COMMITS) {
        match harvest_commit_note(
            &state,
            &org_id,
            &repo.id,
            payload.installation.id,
            &payload.repository.owner.login,
            &payload.repository.name,
            &commit.id,
        )? {
            true => imported += 1,
            false => skipped += 1,
        }
    }
    if payload.commits.len() > MAX_PUSH_COMMITS {
        skipped += payload.commits.len() - MAX_PUSH_COMMITS;
    }
    audit_webhook(&state, &org_id, "github.push", repos_synced, imported)?;
    Ok(WebhookOutcome {
        event: "push".into(),
        repos_synced,
        notes_imported: imported,
        notes_skipped: skipped,
    })
}

fn mapped_org(state: &AppState, installation_id: i64) -> Result<String, ServerError> {
    state
        .store
        .github_installation(installation_id)
        .map_err(ServerError::Internal)?
        .map(|m| m.org_id)
        .ok_or_else(|| ServerError::Forbidden)
}

fn discover_repos(
    state: &AppState,
    installation_id: i64,
    repo_hint: &str,
) -> Result<Vec<crate::github_app::GithubRepository>, ServerError> {
    let app = state.github_app.as_ref().ok_or_else(|| {
        ServerError::Config("TELLUR_GITHUB_APP_ID/private key are required".into())
    })?;
    app.repositories_for_installation(installation_id as u64, repo_hint)
        .map_err(ServerError::Internal)
}

fn sync_repositories(
    state: &AppState,
    org_id: &str,
    repos: &[crate::github_app::GithubRepository],
) -> Result<usize, ServerError> {
    for gh in repos {
        let repo = state
            .store
            .ensure_repo(org_id, &gh.full_name)
            .map_err(ServerError::Internal)?;
        let wire = RepoWire {
            name: gh.name.clone(),
            full_name: gh.full_name.clone(),
            default_branch: gh.default_branch.clone(),
            owner: AccountWire {
                login: gh.owner_login.clone(),
            },
        };
        sync_one_source(state, org_id, &repo.id, &wire)?;
    }
    Ok(repos.len())
}

fn sync_one_source(
    state: &AppState,
    org_id: &str,
    repo_id: &str,
    repo: &RepoWire,
) -> Result<(), ServerError> {
    let link = format!(
        "https://github.com/{}/blob/{}/{{path}}#L{{start}}-L{{end}}",
        repo.full_name, repo.default_branch
    );
    let raw = format!(
        "https://api.github.com/repos/{}/contents/{{path}}?ref={}",
        repo.full_name, repo.default_branch
    );
    let existing = state
        .store
        .get_repo_source(org_id, repo_id)
        .map_err(ServerError::Internal)?;
    // Preserve an existing PAT fallback if the admin configured one; GitHub App
    // tokens are minted dynamically and are never stored here.
    state
        .store
        .set_repo_source(
            org_id,
            repo_id,
            Some(&link),
            Some(&raw),
            existing.token.as_deref(),
        )
        .map_err(ServerError::Internal)
}

fn harvest_commit_note(
    state: &AppState,
    org_id: &str,
    repo_id: &str,
    installation_id: i64,
    owner: &str,
    repo_name: &str,
    commit_sha: &str,
) -> Result<bool, ServerError> {
    let app = state.github_app.as_ref().ok_or_else(|| {
        ServerError::Config("TELLUR_GITHUB_APP_ID/private key are required".into())
    })?;
    let Some((note_sha, note)) = app
        .note_blob_for_commit(installation_id as u64, owner, repo_name, commit_sha)
        .map_err(ServerError::Internal)?
    else {
        return Ok(false);
    };
    if !state
        .store
        .mark_github_note_harvested(org_id, repo_id, commit_sha, &note_sha)
        .map_err(ServerError::Internal)?
    {
        return Ok(false);
    }
    let parsed = parse_git_ai_note(&note).map_err(ServerError::Internal)?;
    let event = IngestEvent {
        session_id: format!("github-note-{commit_sha}"),
        timestamp: chrono::Utc::now().to_rfc3339(),
        event_type: "github.note.harvest".into(),
        actor: "github-app".into(),
        payload: json!({
            "commit_sha": commit_sha,
            "note_sha": note_sha,
            "schema_version": parsed.schema_version,
            "base_commit_sha": parsed.base_commit_sha,
            "files": parsed.files.iter().map(|f| json!({
                "path": f.path,
                "entries": f.entries.len(),
            })).collect::<Vec<_>>(),
            "session_count": parsed.sessions.len(),
            "human_count": parsed.humans.len(),
        }),
    };
    let ids = state
        .store
        .append_events(org_id, repo_id, &[event])
        .map_err(ServerError::Internal)?;
    state.metrics.add_ingested(ids.len() as u64);
    Ok(true)
}

fn audit_webhook(
    state: &AppState,
    org_id: &str,
    action: &str,
    repos: usize,
    notes: usize,
) -> Result<(), ServerError> {
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(org_id.to_string()),
            actor_member_id: None,
            action: action.to_string(),
            detail: format!("repos_synced={repos} notes_imported={notes}"),
        })
        .map_err(ServerError::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_github_header_format() {
        let sig = hmac_sha256_hex(b"secret", br#"{"zen":"Keep it logically awesome."}"#);
        assert_eq!(
            sig,
            "b4d0fd3983e1d5612eaebe005a2092e7176a5e0e6a583899433148eb91c11b4e"
        );
    }

    #[test]
    fn constant_time_compare_rejects_different_lengths() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"abc", b"abd"));
    }
}
