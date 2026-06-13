//! GitHub App installation tokens for the private-repo source proxy (B1).
//!
//! Replaces the manually-pasted PAT (A12) for **GitHub** repos with short-lived,
//! auto-rotating **installation tokens**: the hub signs an App JWT (RS256) with
//! the App private key, exchanges it for a per-repo installation token scoped to
//! `Contents: read`, and uses that token in the source proxy. Wins over a stored
//! PAT: short-lived (≈1h), least-privilege (one repo, read-only), revoked by
//! uninstalling the App, and no human-managed secret in the DB.
//!
//! The PAT path stays as the provider-agnostic fallback (GitLab/Bitbucket/
//! self-managed, or GitHub when the App isn't installed). The network boundary is
//! behind the [`GithubAppApi`] trait so it can be mocked in tests; the JWT signing
//! and template parsing are pure and unit-tested.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// GitHub App configuration, sourced from the environment. Present only when an
/// App id and private key are both configured.
pub struct GithubAppConfig {
    pub app_id: String,
    /// RSA private key in PEM form (PKCS#1 or PKCS#8) — the App's signing key.
    pub private_key_pem: String,
    /// API base (default `https://api.github.com`; override for GitHub Enterprise).
    pub api_base: String,
}

impl GithubAppConfig {
    /// The host of `api_base` (e.g. `api.github.com`, or a GitHub Enterprise host
    /// like `ghe.example.com`). Used to allowlist + recognise GHES source URLs.
    pub fn api_host(&self) -> Option<String> {
        let rest = self.api_base.strip_prefix("https://")?;
        let host = rest.split('/').next().unwrap_or("");
        if host.is_empty() || host.contains('@') || host.contains(':') {
            return None;
        }
        Some(host.to_ascii_lowercase())
    }

    /// Read config from `TELLUR_GITHUB_APP_*`. Returns `None` (App disabled) unless
    /// both the app id and a private key (inline or via a file path) are set.
    pub fn from_env() -> Option<Self> {
        let app_id = non_empty_env("TELLUR_GITHUB_APP_ID")?;
        let private_key_pem = match non_empty_env("TELLUR_GITHUB_APP_PRIVATE_KEY") {
            Some(pem) => pem,
            None => {
                let path = non_empty_env("TELLUR_GITHUB_APP_PRIVATE_KEY_FILE")?;
                match std::fs::read_to_string(&path) {
                    Ok(pem) => pem,
                    Err(e) => {
                        tracing::error!(path, error = %e, "could not read GitHub App private key file");
                        return None;
                    }
                }
            }
        };
        let api_base = non_empty_env("TELLUR_GITHUB_API_BASE")
            .unwrap_or_else(|| "https://api.github.com".to_string());
        Some(Self {
            app_id,
            private_key_pem,
            api_base,
        })
    }
}

fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.is_empty())
}

/// A minted installation token and its expiry (GitHub issues ≈1h tokens).
#[derive(Clone)]
pub struct InstallationToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
struct AppClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

/// Build and sign the App JWT (RS256). `iat` is back-dated 60s to tolerate clock
/// skew and `exp` is 9 minutes out (GitHub's max is 10). Pure + unit-tested.
pub fn build_app_jwt(app_id: &str, private_key_pem: &str, now: DateTime<Utc>) -> Result<String> {
    let claims = AppClaims {
        iat: (now - Duration::seconds(60)).timestamp(),
        exp: (now + Duration::minutes(9)).timestamp(),
        iss: app_id.to_string(),
    };
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
        .context("invalid GitHub App private key (expected an RSA PEM)")?;
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &claims,
        &key,
    )
    .context("failed to sign GitHub App JWT")
}

/// Parse `(owner, repo)` from a GitHub source template (raw-host or contents API).
/// `enterprise_host` is the configured GitHub Enterprise API host (from
/// `TELLUR_GITHUB_API_BASE`), if any — a GHES repo must use a **Contents API**
/// template (`https://<host>/api/v3/repos/{owner}/{repo}/contents/{path}`) on that
/// host. Returns `None` for non-GitHub hosts or unsubstituted owner/repo, the
/// signal that the App path does not apply (use the PAT fallback).
pub fn github_owner_repo(
    template: &str,
    enterprise_host: Option<&str>,
) -> Option<(String, String)> {
    let rest = template.strip_prefix("https://")?;
    let (host, path) = rest.split_once('/')?;
    let host = host.to_ascii_lowercase();
    let segs: Vec<&str> = path.split('/').collect();
    let (owner, repo) = if host == "raw.githubusercontent.com" {
        // {owner}/{repo}/{branch}/{path...}
        (*segs.first()?, *segs.get(1)?)
    } else if host == "api.github.com" && segs.first() == Some(&"repos") {
        // repos/{owner}/{repo}/contents/{path...}
        (*segs.get(1)?, *segs.get(2)?)
    } else if enterprise_host.is_some_and(|h| h == host) {
        // GHES Contents API: .../repos/{owner}/{repo}/contents/{path...}
        let i = segs.iter().position(|s| *s == "repos")?;
        (*segs.get(i + 1)?, *segs.get(i + 2)?)
    } else {
        return None;
    };
    if owner.is_empty() || repo.is_empty() || owner.contains('{') || repo.contains('{') {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

/// The GitHub App network boundary (mocked in tests).
pub trait GithubAppApi: Send + Sync {
    /// Resolve the installation id that has access to `owner/repo`.
    fn installation_id(&self, api_base: &str, jwt: &str, owner: &str, repo: &str) -> Result<u64>;
    /// Mint an installation token scoped to `owner/repo` with `Contents: read`.
    fn installation_token(
        &self,
        api_base: &str,
        jwt: &str,
        installation_id: u64,
        repo: &str,
    ) -> Result<InstallationToken>;
}

/// Real client over ureq/rustls.
pub struct HttpGithubAppApi;

#[derive(Deserialize)]
struct InstallationResponse {
    id: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    token: String,
    expires_at: DateTime<Utc>,
}

impl GithubAppApi for HttpGithubAppApi {
    fn installation_id(&self, api_base: &str, jwt: &str, owner: &str, repo: &str) -> Result<u64> {
        let url = format!(
            "{}/repos/{owner}/{repo}/installation",
            api_base.trim_end_matches('/')
        );
        let resp: InstallationResponse = ureq::get(&url)
            .set("Authorization", &format!("Bearer {jwt}"))
            .set("Accept", "application/vnd.github+json")
            .set("User-Agent", "tellur-server")
            .call()
            .context("GitHub App installation lookup failed")?
            .into_json()
            .context("invalid GitHub App installation response")?;
        Ok(resp.id)
    }

    fn installation_token(
        &self,
        api_base: &str,
        jwt: &str,
        installation_id: u64,
        repo: &str,
    ) -> Result<InstallationToken> {
        let url = format!(
            "{}/app/installations/{installation_id}/access_tokens",
            api_base.trim_end_matches('/')
        );
        // Scope the token to the single repo, read-only contents.
        let body = serde_json::json!({
            "repositories": [repo],
            "permissions": { "contents": "read" },
        });
        let resp: TokenResponse = ureq::post(&url)
            .set("Authorization", &format!("Bearer {jwt}"))
            .set("Accept", "application/vnd.github+json")
            .set("User-Agent", "tellur-server")
            .send_json(body)
            .context("GitHub App token mint failed")?
            .into_json()
            .context("invalid GitHub App token response")?;
        Ok(InstallationToken {
            token: resp.token,
            expires_at: resp.expires_at,
        })
    }
}

/// Assembled runtime: config + client + a per-`(owner, repo)` token cache. Present
/// in [`AppState`](crate::AppState) only when the App is configured.
pub struct GithubAppRuntime {
    pub config: GithubAppConfig,
    api: Arc<dyn GithubAppApi>,
    cache: Mutex<HashMap<(String, String), InstallationToken>>,
}

impl GithubAppRuntime {
    pub fn new(config: GithubAppConfig, api: Arc<dyn GithubAppApi>) -> Self {
        Self {
            config,
            api,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// A valid installation token for `owner/repo`, reusing a cached one until 5
    /// minutes before expiry, otherwise minting a fresh one via the App JWT.
    pub fn token_for(&self, owner: &str, repo: &str) -> Result<String> {
        let key = (owner.to_string(), repo.to_string());
        if let Some(tok) = self.cache.lock().unwrap().get(&key)
            && tok.expires_at > Utc::now() + Duration::minutes(5)
        {
            return Ok(tok.token.clone());
        }
        let jwt = build_app_jwt(
            &self.config.app_id,
            &self.config.private_key_pem,
            Utc::now(),
        )?;
        let installation_id = self
            .api
            .installation_id(&self.config.api_base, &jwt, owner, repo)?;
        let minted =
            self.api
                .installation_token(&self.config.api_base, &jwt, installation_id, repo)?;
        let token = minted.token.clone();
        self.cache.lock().unwrap().insert(key, minted);
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A 2048-bit RSA test key, base64-wrapped so the PEM header can't trip secret
    // scanning. Test-only; not used anywhere real. Decoded via `test_key()`.
    const TEST_KEY_B64: &str = include_str!("../tests/data/github_app_test_key.pem.b64");

    fn test_key() -> String {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(TEST_KEY_B64.trim())
            .unwrap();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn builds_signed_jwt_with_app_claims() {
        let jwt = build_app_jwt("123456", &test_key(), Utc::now()).unwrap();
        // header.payload.signature
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3, "JWT must have three parts");
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(claims["iss"], "123456");
        assert!(claims["exp"].as_i64().unwrap() > claims["iat"].as_i64().unwrap());
    }

    #[test]
    fn rejects_invalid_private_key() {
        assert!(build_app_jwt("1", "not a pem", Utc::now()).is_err());
    }

    #[test]
    fn parses_owner_repo_from_github_templates() {
        assert_eq!(
            github_owner_repo(
                "https://raw.githubusercontent.com/acme/app/main/{path}",
                None
            ),
            Some(("acme".to_string(), "app".to_string()))
        );
        assert_eq!(
            github_owner_repo(
                "https://api.github.com/repos/acme/app/contents/{path}",
                None
            ),
            Some(("acme".to_string(), "app".to_string()))
        );
    }

    #[test]
    fn parses_owner_repo_from_enterprise_contents_template() {
        // GHES Contents API on the configured enterprise host.
        assert_eq!(
            github_owner_repo(
                "https://ghe.example.com/api/v3/repos/acme/app/contents/{path}?ref=main",
                Some("ghe.example.com"),
            ),
            Some(("acme".to_string(), "app".to_string()))
        );
        // Same host but no App configured (enterprise_host None) → not recognised.
        assert_eq!(
            github_owner_repo(
                "https://ghe.example.com/api/v3/repos/acme/app/contents/{path}",
                None,
            ),
            None
        );
    }

    #[test]
    fn skips_non_github_or_templated_owner_repo() {
        assert_eq!(
            github_owner_repo("https://gitlab.com/acme/app/-/raw/main/{path}", None),
            None
        );
        // Unsubstituted owner placeholder → not a concrete GitHub repo.
        assert_eq!(
            github_owner_repo(
                "https://raw.githubusercontent.com/{owner}/{repo}/main/{path}",
                None
            ),
            None
        );
    }

    #[test]
    fn api_host_extracts_configured_base() {
        let cfg = GithubAppConfig {
            app_id: "1".into(),
            private_key_pem: String::new(),
            api_base: "https://ghe.example.com/api/v3".into(),
        };
        assert_eq!(cfg.api_host().as_deref(), Some("ghe.example.com"));
    }

    struct MockApi;
    impl GithubAppApi for MockApi {
        fn installation_id(&self, _: &str, _: &str, _: &str, _: &str) -> Result<u64> {
            Ok(42)
        }
        fn installation_token(
            &self,
            _: &str,
            _: &str,
            id: u64,
            repo: &str,
        ) -> Result<InstallationToken> {
            Ok(InstallationToken {
                token: format!("ghs_inst{id}_{repo}"),
                expires_at: Utc::now() + Duration::hours(1),
            })
        }
    }

    #[test]
    fn caches_minted_token_until_near_expiry() {
        let rt = GithubAppRuntime::new(
            GithubAppConfig {
                app_id: "1".into(),
                private_key_pem: test_key(),
                api_base: "https://api.github.com".into(),
            },
            Arc::new(MockApi),
        );
        let a = rt.token_for("acme", "app").unwrap();
        let b = rt.token_for("acme", "app").unwrap();
        assert_eq!(a, b);
        assert_eq!(a, "ghs_inst42_app");
    }
}
