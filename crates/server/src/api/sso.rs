//! OIDC SSO browser flow (`/auth/login|callback|logout`).

use super::common::*;

/// `GET /auth/login` — begin the OIDC Authorization Code + PKCE flow. Persists
/// a login transaction (CSRF state → PKCE/nonce) and redirects to the IdP.
pub async fn oidc_login(State(state): State<AppState>) -> Result<Response, ServerError> {
    let oidc = state.oidc.clone().ok_or(ServerError::NotFound)?;
    // Opportunistically prune stale login rows so anonymous /auth/login traffic
    // can't grow the table without bound, then enforce a hard cap.
    state
        .store
        .prune_expired_logins(oidc::LOGIN_TTL_SECS)
        .map_err(ServerError::Internal)?;
    if state.store.count_logins().map_err(ServerError::Internal)? >= MAX_OUTSTANDING_LOGINS {
        return Err(ServerError::TooManyRequests);
    }
    let pkce = Pkce::generate();
    let state_tok = oidc::random_token(24);
    let nonce = oidc::random_token(24);
    // Browser-binding secret: stored with the tx and set as a cookie; the
    // callback must present a matching cookie, so a state value leaked/forwarded
    // to a victim cannot complete the flow in their browser (login-CSRF).
    let binding = oidc::random_token(24);
    state
        .store
        .put_login(&state_tok, &pkce.verifier, &nonce, &binding)
        .map_err(ServerError::Internal)?;
    // Discovery may hit the network → run off the async worker.
    let url = run_blocking({
        let oidc = oidc.clone();
        let state_tok = state_tok.clone();
        let nonce = nonce.clone();
        let challenge = pkce.challenge.clone();
        move || {
            let disc = oidc.discovery()?;
            Ok(oidc::build_authorize_url(
                &disc,
                &oidc.config,
                &state_tok,
                &nonce,
                &challenge,
            ))
        }
    })
    .await?;
    let mut resp = Redirect::to(&url).into_response();
    resp.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&login_cookie(&binding))
            .map_err(|e| ServerError::Internal(e.into()))?,
    );
    Ok(resp)
}

/// Query parameters returned by the IdP to the redirect URI.
#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// `GET /auth/callback` — complete the flow: validate state, exchange the code,
/// validate the ID token, map to a provisioned member, and start a session.
pub async fn oidc_callback(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<CallbackParams>,
) -> Result<Response, ServerError> {
    let oidc = state.oidc.clone().ok_or(ServerError::NotFound)?;
    if let Some(err) = params.error {
        return Err(ServerError::BadRequest(format!(
            "IdP returned error: {err}"
        )));
    }
    let (Some(code), Some(state_tok)) = (params.code, params.state) else {
        return Err(ServerError::BadRequest("missing code or state".to_string()));
    };
    // Consume the login transaction. An unknown state is a CSRF / replay signal.
    let login = state
        .store
        .take_login(&state_tok)
        .map_err(ServerError::Internal)?
        .ok_or_else(|| ServerError::BadRequest("unknown or expired login state".to_string()))?;
    if login_expired(&login.created_at) {
        return Err(ServerError::BadRequest("login state expired".to_string()));
    }
    // The callback must come from the browser that initiated the flow: its
    // login cookie must match the stored binding (defends against login-CSRF /
    // session fixation where a leaked callback URL is opened in a victim's
    // browser).
    let presented = cookie_value(&headers, LOGIN_COOKIE).unwrap_or_default();
    if !secret_eq(&presented, &login.browser_binding) {
        return Err(ServerError::BadRequest(
            "login binding mismatch".to_string(),
        ));
    }
    // Exchange the code for an ID token (network, over TLS).
    let id_token = run_blocking({
        let oidc = oidc.clone();
        let verifier = login.pkce_verifier.clone();
        move || oidc.exchange_code(&code, &verifier)
    })
    .await?;
    // Validate ID-token claims (iss/aud/exp/nonce); signature integrity is
    // provided by the TLS-secured token-endpoint channel (see oidc module docs).
    let now = chrono::Utc::now().timestamp();
    let claims = oidc::parse_and_validate_id_token(
        &id_token,
        &oidc.config.issuer,
        &oidc.config.client_id,
        &login.nonce,
        now,
    )
    .map_err(|_| ServerError::Unauthorized)?;

    let principal = resolve_sso_member(&state, &claims, &oidc.config.issuer)?;
    let sid = state
        .store
        .create_session(&principal.member_id, oidc::SESSION_TTL_SECS)
        .map_err(ServerError::Internal)?;
    state
        .store
        .append_audit(&AuditEntry {
            org_id: Some(principal.org_id.clone()),
            actor_member_id: Some(principal.member_id.clone()),
            action: "auth.sso_login".to_string(),
            detail: "via=oidc".to_string(),
        })
        .map_err(ServerError::Internal)?;

    // Honor a validated return path (e.g. the device-approval page that bounced
    // the user through login), else land on the app root. Only same-origin local
    // paths are allowed, so a forged cookie cannot redirect off-site.
    let dest = cookie_value(&headers, RETURN_COOKIE)
        .and_then(|p| safe_return_path(&p))
        .unwrap_or_else(|| "/".to_string());
    let mut resp = Redirect::to(&dest).into_response();
    let headers = resp.headers_mut();
    headers.append(
        SET_COOKIE,
        HeaderValue::from_str(&session_cookie(&sid, oidc::SESSION_TTL_SECS))
            .map_err(|e| ServerError::Internal(e.into()))?,
    );
    // Clear the now-consumed login-binding cookie.
    headers.append(
        SET_COOKIE,
        HeaderValue::from_str(&clear_login_cookie())
            .map_err(|e| ServerError::Internal(e.into()))?,
    );
    // Clear the now-consumed return cookie.
    headers.append(
        SET_COOKIE,
        HeaderValue::from_str(&clear_return_cookie())
            .map_err(|e| ServerError::Internal(e.into()))?,
    );
    Ok(resp)
}

/// Map validated ID-token claims to a provisioned member. No open
/// self-registration: an unknown identity is rejected (403). On first login the
/// OIDC subject is bound so later logins match by subject even if email changes.
fn resolve_sso_member(
    state: &AppState,
    claims: &oidc::IdClaims,
    issuer: &str,
) -> Result<Principal, ServerError> {
    // Subjects are only unique within an issuer, so the binding is keyed by
    // (issuer, subject).
    if let Some(p) = state
        .store
        .find_member_by_oidc_subject(issuer, &claims.subject)
        .map_err(ServerError::Internal)?
    {
        return Ok(p);
    }
    // Fall back to a *verified* email, then bind the subject for next time.
    let email = claims
        .email
        .as_deref()
        .filter(|_| claims.email_verified)
        .ok_or(ServerError::Forbidden)?;
    match state
        .store
        .find_member_by_email(email)
        .map_err(ServerError::Internal)?
    {
        Some(p) => {
            // Bind the (issuer, subject) on first login only. If the member
            // already has a (different) binding, refuse — a second IdP account
            // on the same email must not take over the member.
            let bound = state
                .store
                .bind_oidc_subject(&p.member_id, issuer, &claims.subject)
                .map_err(ServerError::Internal)?;
            if !bound {
                return Err(ServerError::Forbidden);
            }
            Ok(p)
        }
        None => Err(ServerError::Forbidden),
    }
}

/// `GET /auth/logout` — delete the current session and clear the cookie.
pub async fn oidc_logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Response, ServerError> {
    if let Some(sid) = session_cookie_value(&headers) {
        state
            .store
            .delete_session(&sid)
            .map_err(ServerError::Internal)?;
    }
    let mut resp = Redirect::to("/").into_response();
    resp.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&clear_session_cookie())
            .map_err(|e| ServerError::Internal(e.into()))?,
    );
    Ok(resp)
}

// ─── Device authorization (CLI `tellur login`, RFC 8628-style) ───────────────

