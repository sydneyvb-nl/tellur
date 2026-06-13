//! OIDC SSO: Authorization Code flow with PKCE for dashboard login.
//!
//! Flow: `/auth/login` builds an IdP authorize URL (state + nonce + PKCE S256)
//! and persists the transaction; `/auth/callback` validates `state`, exchanges
//! the code at the IdP token endpoint over **TLS**, validates the returned ID
//! token's claims, maps the verified email/subject to a provisioned member, and
//! creates a session.
//!
//! **ID-token verification:** because the token is fetched over a direct,
//! TLS-validated channel to the discovered token endpoint, OIDC Core §3.1.3.7
//! permits relying on TLS for integrity instead of verifying the JWT signature.
//! We still validate `iss`, `aud`, `exp`, and the `nonce` (which binds the token
//! to our own login request), so a replayed or misissued token is rejected. The
//! IdP boundary is behind an [`OidcClient`] trait so the flow logic is fully
//! unit-tested with a mock (no network) while the real impl uses `ureq`+rustls.

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand_core::{OsRng, RngCore};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Session lifetime for a browser login (8 hours).
pub const SESSION_TTL_SECS: i64 = 8 * 60 * 60;

/// Login transactions older than this are considered expired (CSRF/PKCE state).
pub const LOGIN_TTL_SECS: i64 = 10 * 60;

/// OIDC configuration, sourced from the environment. SSO is enabled only when
/// all four values are present.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    /// Allow a plaintext `http` issuer/endpoints on **any** host (not just
    /// loopback). INSECURE — only for a trusted private network or local dev,
    /// where ID-token integrity can't be MITM'd. Off by default; opt in with
    /// `TELLUR_OIDC_ALLOW_INSECURE_HTTP=1`.
    pub allow_insecure_http: bool,
}

impl OidcConfig {
    /// Build from `TELLUR_OIDC_*`. Returns `None` when SSO is not configured.
    pub fn from_env() -> Option<Self> {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let flag = |k: &str| matches!(get(k).as_deref(), Some("1" | "true" | "yes"));
        Some(Self {
            issuer: get("TELLUR_OIDC_ISSUER")?,
            client_id: get("TELLUR_OIDC_CLIENT_ID")?,
            client_secret: get("TELLUR_OIDC_CLIENT_SECRET")?,
            redirect_uri: get("TELLUR_OIDC_REDIRECT_URI")?,
            allow_insecure_http: flag("TELLUR_OIDC_ALLOW_INSECURE_HTTP"),
        })
    }

    /// Whether the configured issuer would be rejected as non-secure (so the
    /// caller can warn at startup rather than surface an opaque 500 at login).
    pub fn issuer_is_secure(&self) -> bool {
        require_secure("issuer", &self.issuer, self.allow_insecure_http).is_ok()
    }
}

/// The IdP endpoints we use, from OIDC discovery.
#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    /// Echoed back for an issuer sanity check.
    #[serde(default)]
    pub issuer: String,
}

/// The IdP boundary: discovery + code exchange. Behind a trait so the flow logic
/// is testable without network access.
pub trait OidcClient: Send + Sync {
    /// Fetch the issuer's OIDC discovery document.
    fn discover(&self, issuer: &str) -> Result<Discovery>;

    /// Exchange an authorization code for tokens; returns the raw `id_token` JWT.
    fn exchange_code(
        &self,
        token_endpoint: &str,
        code: &str,
        redirect_uri: &str,
        client_id: &str,
        client_secret: &str,
        pkce_verifier: &str,
    ) -> Result<String>;
}

/// Real IdP client over `ureq` (blocking) with rustls TLS.
pub struct HttpOidcClient;

#[derive(Deserialize)]
struct TokenResponse {
    id_token: String,
}

impl OidcClient for HttpOidcClient {
    fn discover(&self, issuer: &str) -> Result<Discovery> {
        let url = format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        );
        let disc: Discovery = ureq::get(&url)
            .call()
            .context("OIDC discovery request failed")?
            .into_json()
            .context("invalid OIDC discovery document")?;
        Ok(disc)
    }

    fn exchange_code(
        &self,
        token_endpoint: &str,
        code: &str,
        redirect_uri: &str,
        client_id: &str,
        client_secret: &str,
        pkce_verifier: &str,
    ) -> Result<String> {
        let resp: TokenResponse = ureq::post(token_endpoint)
            .send_form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("code_verifier", pkce_verifier),
            ])
            .context("OIDC token exchange failed")?
            .into_json()
            .context("invalid OIDC token response")?;
        Ok(resp.id_token)
    }
}

/// Assembled OIDC runtime: config + IdP client + cached discovery. Present in
/// [`AppState`](crate::AppState) only when SSO is configured.
pub struct OidcRuntime {
    pub config: OidcConfig,
    client: Arc<dyn OidcClient>,
    cached: Mutex<Option<Discovery>>,
}

impl OidcRuntime {
    pub fn new(config: OidcConfig, client: Arc<dyn OidcClient>) -> Self {
        Self {
            config,
            client,
            cached: Mutex::new(None),
        }
    }

    /// Resolve discovery, caching the first successful result. Enforces HTTPS on
    /// the issuer and all discovered endpoints: ID-token integrity relies on the
    /// TLS channel (we don't verify the JWT signature locally), so a plaintext
    /// issuer/endpoint would let a network attacker forge a login.
    pub fn discovery(&self) -> Result<Discovery> {
        if let Some(d) = self
            .cached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return Ok(d);
        }
        let insecure = self.config.allow_insecure_http;
        require_secure("issuer", &self.config.issuer, insecure)?;
        let disc = self.client.discover(&self.config.issuer)?;
        // The metadata's `issuer` must exactly match what we configured (OIDC
        // Discovery §4.3). Otherwise a misconfigured/redirected discovery
        // document could point us at a different provider whose tokens we would
        // accept (we don't verify ID-token signatures locally).
        if disc.issuer.trim_end_matches('/') != self.config.issuer.trim_end_matches('/') {
            bail!(
                "OIDC discovery issuer mismatch (configured {}, metadata {})",
                self.config.issuer,
                disc.issuer
            );
        }
        require_secure(
            "authorization_endpoint",
            &disc.authorization_endpoint,
            insecure,
        )?;
        require_secure("token_endpoint", &disc.token_endpoint, insecure)?;
        *self
            .cached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(disc.clone());
        Ok(disc)
    }

    /// Exchange a code for the raw ID token at the discovered token endpoint.
    pub fn exchange_code(&self, code: &str, pkce_verifier: &str) -> Result<String> {
        let disc = self.discovery()?;
        self.client.exchange_code(
            &disc.token_endpoint,
            code,
            &self.config.redirect_uri,
            &self.config.client_id,
            &self.config.client_secret,
            pkce_verifier,
        )
    }
}

/// PKCE pair (RFC 7636, S256).
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        let verifier = random_token(48);
        let digest = Sha256::digest(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(digest);
        Self {
            verifier,
            challenge,
        }
    }
}

/// Reject a non-secure URL. `https` always passes. Plaintext `http` passes only
/// when the host is *exactly* a loopback host (`localhost`, `127.0.0.1`, `::1`)
/// — or, when `allow_insecure` is set, for any host (an explicit, documented
/// opt-in for a trusted private network / dev; see `TELLUR_OIDC_ALLOW_INSECURE_HTTP`).
/// The host is parsed (not prefix-matched) so `http://localhost.evil.example` is
/// still rejected without the opt-in.
fn require_secure(what: &str, url: &str, allow_insecure: bool) -> Result<()> {
    if url.starts_with("https://") {
        return Ok(());
    }
    if let Some(rest) = url.strip_prefix("http://")
        && (allow_insecure || is_loopback_host(rest))
    {
        return Ok(());
    }
    bail!(
        "OIDC {what} must use https (got {url}) — set TELLUR_OIDC_ALLOW_INSECURE_HTTP=1 \
         to allow http on a trusted private network"
    );
}

/// Extract the host from a URL authority (`host[:port]/...`) and test whether it
/// is exactly a loopback host. Userinfo (`user@`) is not accepted.
fn is_loopback_host(after_scheme: &str) -> bool {
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    if authority.contains('@') {
        return false;
    }
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        // [::1]:port → ::1
        stripped.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

/// A high-entropy URL-safe random token (used for state, nonce, PKCE verifier,
/// and session/login ids).
pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// A short, human-typable device user-code formatted `XXXX-XXXX`. Drawn from an
/// unambiguous alphabet (no vowels — avoids accidental words — and no
/// 0/O/1/I/L lookalikes) so it reads cleanly aloud and over the phone.
pub fn random_user_code() -> String {
    const ALPHABET: &[u8] = b"BCDFGHJKMNPQRSTVWXYZ23456789";
    let mut buf = [0u8; 8];
    OsRng.fill_bytes(&mut buf);
    let c: Vec<char> = buf
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    format!(
        "{}{}{}{}-{}{}{}{}",
        c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]
    )
}

/// Build the IdP authorization URL for the code flow with PKCE.
pub fn build_authorize_url(
    disc: &Discovery,
    cfg: &OidcConfig,
    state: &str,
    nonce: &str,
    challenge: &str,
) -> String {
    let q = [
        ("response_type", "code"),
        ("client_id", &cfg.client_id),
        ("redirect_uri", &cfg.redirect_uri),
        ("scope", "openid email profile"),
        ("state", state),
        ("nonce", nonce),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
    ];
    let query: Vec<String> = q
        .iter()
        .map(|(k, v)| format!("{}={}", k, percent_encode(v)))
        .collect();
    let sep = if disc.authorization_endpoint.contains('?') {
        '&'
    } else {
        '?'
    };
    format!("{}{}{}", disc.authorization_endpoint, sep, query.join("&"))
}

/// Validated ID-token claims we rely on.
#[derive(Debug, Clone)]
pub struct IdClaims {
    pub subject: String,
    pub email: Option<String>,
    pub email_verified: bool,
}

#[derive(Deserialize)]
struct RawClaims {
    iss: String,
    sub: String,
    aud: serde_json::Value,
    exp: i64,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    email_verified: Option<bool>,
}

/// Parse and validate an ID token's claims. Signature is not verified locally —
/// see the module docs (TLS-secured direct token-endpoint channel).
pub fn parse_and_validate_id_token(
    jwt: &str,
    issuer: &str,
    client_id: &str,
    expected_nonce: &str,
    now_unix: i64,
) -> Result<IdClaims> {
    let mut parts = jwt.split('.');
    let (_h, payload, _s) = match (parts.next(), parts.next(), parts.next()) {
        (Some(h), Some(p), Some(s)) => (h, p, s),
        _ => bail!("malformed ID token (expected 3 JWT segments)"),
    };
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .context("ID token payload is not valid base64url")?;
    let claims: RawClaims =
        serde_json::from_slice(&bytes).context("ID token payload is not valid JSON")?;

    if claims.iss.trim_end_matches('/') != issuer.trim_end_matches('/') {
        bail!("ID token issuer mismatch");
    }
    if !aud_contains(&claims.aud, client_id) {
        bail!("ID token audience does not include this client");
    }
    if claims.exp <= now_unix {
        bail!("ID token is expired");
    }
    match claims.nonce.as_deref() {
        Some(n) if n == expected_nonce => {}
        _ => bail!("ID token nonce mismatch"),
    }
    Ok(IdClaims {
        subject: claims.sub,
        email: claims.email,
        email_verified: claims.email_verified.unwrap_or(false),
    })
}

/// `aud` may be a single string or an array of strings.
fn aud_contains(aud: &serde_json::Value, client_id: &str) -> bool {
    match aud {
        serde_json::Value::String(s) => s == client_id,
        serde_json::Value::Array(items) => items.iter().any(|v| v.as_str() == Some(client_id)),
        _ => false,
    }
}

/// Percent-encode a query-parameter value (encode everything that isn't an
/// unreserved character per RFC 3986).
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for &b in value.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_secure_https_loopback_and_insecure_optin() {
        // https always passes.
        assert!(require_secure("issuer", "https://idp.example/realms/x", false).is_ok());
        // loopback http passes without the opt-in.
        assert!(require_secure("issuer", "http://127.0.0.1:8080/x", false).is_ok());
        assert!(require_secure("issuer", "http://localhost:8080/x", false).is_ok());
        // a LAN http issuer is rejected by default …
        assert!(require_secure("issuer", "http://192.168.1.65:8080/realms/x", false).is_err());
        // … but allowed with the explicit opt-in.
        assert!(require_secure("issuer", "http://192.168.1.65:8080/realms/x", true).is_ok());
        // a spoofed loopback host is still rejected without the opt-in.
        assert!(require_secure("issuer", "http://localhost.evil.example/x", false).is_err());
    }

    fn jwt_with(payload: serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let body = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
        let sig = URL_SAFE_NO_PAD.encode(b"unverified");
        format!("{header}.{body}.{sig}")
    }

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let p = Pkce::generate();
        let expect = URL_SAFE_NO_PAD.encode(Sha256::digest(p.verifier.as_bytes()));
        assert_eq!(p.challenge, expect);
        assert_ne!(p.verifier, p.challenge);
    }

    #[test]
    fn authorize_url_has_pkce_and_state() {
        let disc = Discovery {
            authorization_endpoint: "https://idp.example/authorize".to_string(),
            token_endpoint: "https://idp.example/token".to_string(),
            issuer: "https://idp.example".to_string(),
        };
        let cfg = OidcConfig {
            issuer: "https://idp.example".to_string(),
            client_id: "client 1".to_string(),
            client_secret: "sec".to_string(),
            redirect_uri: "https://hub.example/auth/callback".to_string(),
            allow_insecure_http: false,
        };
        let url = build_authorize_url(&disc, &cfg, "st8", "non", "chal");
        assert!(url.starts_with("https://idp.example/authorize?"));
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=st8"));
        assert!(url.contains("nonce=non"));
        // Reserved characters in values are encoded.
        assert!(url.contains("client_id=client%201"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Fhub.example%2Fauth%2Fcallback"));
    }

    #[test]
    fn valid_id_token_is_accepted() {
        let jwt = jwt_with(serde_json::json!({
            "iss": "https://idp.example",
            "sub": "user-123",
            "aud": "client-1",
            "exp": 2000,
            "nonce": "n1",
            "email": "a@b.com",
            "email_verified": true,
        }));
        let claims =
            parse_and_validate_id_token(&jwt, "https://idp.example", "client-1", "n1", 1000)
                .unwrap();
        assert_eq!(claims.subject, "user-123");
        assert_eq!(claims.email.as_deref(), Some("a@b.com"));
        assert!(claims.email_verified);
    }

    #[test]
    fn aud_array_is_supported() {
        let jwt = jwt_with(serde_json::json!({
            "iss": "https://idp.example", "sub": "u", "aud": ["other", "client-1"],
            "exp": 2000, "nonce": "n1",
        }));
        assert!(
            parse_and_validate_id_token(&jwt, "https://idp.example", "client-1", "n1", 1000)
                .is_ok()
        );
    }

    #[test]
    fn rejects_bad_issuer_aud_exp_and_nonce() {
        let base = |over: serde_json::Value| {
            let mut m = serde_json::json!({
                "iss": "https://idp.example", "sub": "u", "aud": "client-1",
                "exp": 2000, "nonce": "n1",
            });
            for (k, v) in over.as_object().unwrap() {
                m[k] = v.clone();
            }
            jwt_with(m)
        };
        let v = |jwt: String, nonce: &str| {
            parse_and_validate_id_token(&jwt, "https://idp.example", "client-1", nonce, 1000)
        };
        assert!(v(base(serde_json::json!({"iss": "https://evil"})), "n1").is_err());
        assert!(v(base(serde_json::json!({"aud": "other"})), "n1").is_err());
        assert!(v(base(serde_json::json!({"exp": 500})), "n1").is_err());
        assert!(v(base(serde_json::json!({"nonce": "wrong"})), "n1").is_err());
        // Nonce mismatch against our expectation.
        assert!(v(base(serde_json::json!({})), "different").is_err());
    }

    #[test]
    fn rejects_malformed_token() {
        assert!(parse_and_validate_id_token("not.a", "i", "c", "n", 0).is_err());
        assert!(parse_and_validate_id_token("a.b.c", "i", "c", "n", 0).is_err());
    }

    #[test]
    fn require_secure_rejects_plaintext_endpoints() {
        assert!(require_secure("issuer", "https://idp.example", false).is_ok());
        assert!(require_secure("issuer", "http://idp.example", false).is_err());
        // Loopback http is allowed for local dev/testing only.
        assert!(require_secure("token", "http://127.0.0.1:8080/token", false).is_ok());
        assert!(require_secure("token", "http://localhost/token", false).is_ok());
    }
}
