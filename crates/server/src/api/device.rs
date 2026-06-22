//! Device-authorization flow for CLI `tellur login`.

use super::common::*;

/// `POST /v1/device/authorize` — begin a CLI login. Issues a secret
/// `device_code` (polled by the CLI) and a short `user_code` (typed by the human
/// in the browser). No auth: the request is only a claim ticket; nothing is
/// granted until a signed-in member approves the `user_code`.
pub async fn device_authorize(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<Value>, ServerError> {
    // Device login rides on the same SSO machinery; if SSO is off there is no
    // identity to approve with, so the flow is unavailable.
    if state.oidc.is_none() {
        return Err(ServerError::NotFound);
    }
    // Prune stale rows, then enforce a hard cap so anonymous traffic can't grow
    // the table without bound (mirrors the OIDC-login anti-flood).
    state
        .store
        .prune_expired_device_auths(DEVICE_TTL_SECS)
        .map_err(ServerError::Internal)?;
    if state
        .store
        .count_device_auths()
        .map_err(ServerError::Internal)?
        >= MAX_OUTSTANDING_DEVICE
    {
        return Err(ServerError::TooManyRequests);
    }
    let device_code = oidc::random_token(32);
    let user_code = oidc::random_user_code();
    state
        .store
        .create_device_auth(&device_code, &user_code)
        .map_err(ServerError::Internal)?;
    let base = request_base(&headers);
    Ok(Json(json!({
        "device_code": device_code,
        "user_code": user_code,
        "verification_uri": format!("{base}/auth/device"),
        "verification_uri_complete": format!("{base}/auth/device?user_code={user_code}"),
        "expires_in": DEVICE_TTL_SECS,
        "interval": DEVICE_POLL_INTERVAL_SECS,
    })))
}

/// Body of a device-token poll.
#[derive(Debug, Deserialize)]
pub struct DeviceTokenRequest {
    pub device_code: String,
}

/// `POST /v1/device/token` — the CLI polls here until the request is approved.
/// Mirrors RFC 8628 error codes: `authorization_pending`, `access_denied`,
/// `expired_token`. On approval the hub mints a member API token and returns it
/// once (the row is consumed), reflecting the member's role at approval time.
pub async fn device_token(
    State(state): State<AppState>,
    Json(req): Json<DeviceTokenRequest>,
) -> Result<Response, ServerError> {
    state
        .store
        .prune_expired_device_auths(DEVICE_TTL_SECS)
        .map_err(ServerError::Internal)?;
    match state
        .store
        .poll_device(&req.device_code, DEVICE_TTL_SECS)
        .map_err(ServerError::Internal)?
    {
        DevicePoll::Pending => Ok(device_pending("authorization_pending")),
        DevicePoll::Denied => Ok(device_pending("access_denied")),
        DevicePoll::NotFound => Ok(device_pending("expired_token")),
        DevicePoll::Approved(member_id) => {
            // Resolve the member fresh: a member deactivated between approval and
            // poll must not receive a working token.
            let principal = state
                .store
                .member_principal(&member_id)
                .map_err(ServerError::Internal)?
                .ok_or(ServerError::Forbidden)?;
            let token = state
                .store
                .create_token(&principal.member_id)
                .map_err(ServerError::Internal)?;
            state
                .store
                .append_audit(&AuditEntry {
                    org_id: Some(principal.org_id.clone()),
                    actor_member_id: Some(principal.member_id.clone()),
                    action: "auth.device_login".to_string(),
                    detail: "via=device_code".to_string(),
                })
                .map_err(ServerError::Internal)?;
            Ok(Json(json!({
                "access_token": token.plaintext,
                "token_type": "Bearer",
                "org_id": principal.org_id,
                "member_id": principal.member_id,
                "role": principal.role.as_str(),
            }))
            .into_response())
        }
    }
}

/// A still-pending / terminal-but-not-approved device poll: 400 + an `error`
/// code the CLI switches on (RFC 8628 grant-error shape).
fn device_pending(error: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response()
}

/// Query for the device-approval page.
#[derive(Debug, Deserialize)]
pub struct DeviceVerifyParams {
    #[serde(default)]
    pub user_code: Option<String>,
}

/// `GET /auth/device` — the human-facing approval page. Requires a signed-in
/// session; an unauthenticated visitor is sent through SSO and bounced back here
/// (return cookie). Renders a small confirmation form pre-filled with the code.
pub async fn device_page(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<DeviceVerifyParams>,
) -> Result<Response, ServerError> {
    let user_code = params.user_code.unwrap_or_default();
    // Require a session; otherwise start SSO and remember to return here.
    let principal = match current_session_principal(&state, &headers)? {
        Some(p) => p,
        None => {
            let return_to = format!("/auth/device?user_code={}", percent_encode(&user_code));
            let mut resp = Redirect::to("/auth/login").into_response();
            resp.headers_mut().insert(
                SET_COOKIE,
                HeaderValue::from_str(&return_cookie(&return_to))
                    .map_err(|e| ServerError::Internal(e.into()))?,
            );
            return Ok(resp);
        }
    };
    let known = state
        .store
        .find_device_by_user_code(&user_code)
        .map_err(ServerError::Internal)?
        .filter(|d| d.status == "pending");
    Ok(Html(render_device_page(&user_code, &principal, known.is_some())).into_response())
}

/// Form body of a device-approval decision.
#[derive(Debug, Deserialize)]
pub struct DeviceDecisionForm {
    pub user_code: String,
    /// `approve` or `deny`.
    pub decision: String,
}

/// `POST /auth/device/decision` — record the signed-in member's approve/deny for
/// a device request. Requires a session (a cross-site POST can't carry the
/// SameSite=Lax session cookie, so this is the CSRF defense).
pub async fn device_decision(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Form(form): axum::extract::Form<DeviceDecisionForm>,
) -> Result<Response, ServerError> {
    let principal =
        current_session_principal(&state, &headers)?.ok_or(ServerError::Unauthorized)?;
    let approve = form.decision == "approve";
    let updated = state
        .store
        .set_device_decision(&form.user_code, &principal.member_id, approve)
        .map_err(ServerError::Internal)?;
    if updated {
        state
            .store
            .append_audit(&AuditEntry {
                org_id: Some(principal.org_id.clone()),
                actor_member_id: Some(principal.member_id.clone()),
                action: "auth.device_decision".to_string(),
                detail: format!("decision={}", if approve { "approve" } else { "deny" }),
            })
            .map_err(ServerError::Internal)?;
    }
    let outcome = if !updated {
        DeviceOutcome::Unknown
    } else if approve {
        DeviceOutcome::Approved
    } else {
        DeviceOutcome::Denied
    };
    Ok(Html(render_device_result(outcome)).into_response())
}

/// Resolve the caller's session cookie to a principal, if any.
fn current_session_principal(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<Option<Principal>, ServerError> {
    let Some(sid) = session_cookie_value(headers) else {
        return Ok(None);
    };
    state
        .store
        .session_principal(&sid)
        .map_err(ServerError::Internal)
}

/// Best-effort public base URL (`scheme://host`) from request headers, for the
/// informational `verification_uri`. The CLI also builds this from its own hub
/// URL, so a spoofed `Host` only affects the displayed link, never authorization.
fn request_base(headers: &axum::http::HeaderMap) -> String {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let loopback = host.starts_with("localhost")
                || host.starts_with("127.0.0.1")
                || host.starts_with("[::1]");
            if loopback {
                "http".into()
            } else {
                "https".into()
            }
        });
    format!("{scheme}://{host}")
}

/// Minimal percent-encoding for a path query value (the user_code alphabet is
/// already URL-safe, but encode defensively).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Escape text for safe interpolation into HTML element content/attributes.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Result of a device-approval decision, for the confirmation page.
enum DeviceOutcome {
    Approved,
    Denied,
    Unknown,
}

/// Shared chrome for the small device-flow pages (self-contained, no SPA).
fn device_shell(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{title} · Tellur</title>\
<style>\
:root{{color-scheme:light dark}}\
*{{box-sizing:border-box}}\
body{{margin:0;min-height:100vh;display:grid;place-items:center;\
font:15px/1.5 system-ui,-apple-system,Segoe UI,Roboto,sans-serif;\
background:#f6f7f9;color:#1b1f24}}\
@media(prefers-color-scheme:dark){{body{{background:#0e1116;color:#e6e8eb}}\
.card{{background:#161b22;border-color:#30363d}}}}\
.card{{background:#fff;border:1px solid #e2e5e9;border-radius:14px;\
padding:32px;max-width:420px;width:calc(100% - 32px);\
box-shadow:0 8px 30px rgba(0,0,0,.08)}}\
h1{{font-size:18px;margin:0 0 6px}}\
p{{margin:8px 0;color:#5b6470}}\
@media(prefers-color-scheme:dark){{p{{color:#9aa4b2}}}}\
.code{{font:600 28px/1 ui-monospace,SFMono-Regular,Menlo,monospace;\
letter-spacing:3px;text-align:center;padding:16px;margin:16px 0;\
border-radius:10px;background:#eef1f4}}\
@media(prefers-color-scheme:dark){{.code{{background:#21262d}}}}\
.row{{display:flex;gap:10px;margin-top:20px}}\
button{{flex:1;padding:11px 14px;border-radius:9px;border:1px solid transparent;\
font:inherit;font-weight:600;cursor:pointer}}\
.approve{{background:#1f6feb;color:#fff}}\
.deny{{background:transparent;border-color:#d0d7de;color:inherit}}\
.muted{{font-size:13px;color:#8a929c}}\
.brand{{font-weight:700;letter-spacing:.5px;margin-bottom:18px;font-size:13px;\
text-transform:uppercase;color:#8a929c}}\
</style></head><body><div class=\"card\"><div class=\"brand\">Tellur</div>{body}</div></body></html>"
    )
}

/// Render the approval page for a (possibly unknown) user_code.
fn render_device_page(user_code: &str, principal: &Principal, known: bool) -> String {
    let safe_code = html_escape(user_code);
    if !known {
        let body = format!(
            "<h1>Code not found</h1>\
<p>The code <strong>{safe_code}</strong> is unknown or has expired. \
Start the login again from your terminal with <code>tellur login</code>.</p>"
        );
        return device_shell("Device login", &body);
    }
    let body = format!(
        "<h1>Authorize this device?</h1>\
<p>A terminal is requesting access to your Tellur account. Confirm the code \
matches the one shown in your terminal.</p>\
<div class=\"code\">{safe_code}</div>\
<p class=\"muted\">Signed in as <strong>{member}</strong> · role {role} · org {org}</p>\
<form method=\"post\" action=\"/auth/device/decision\">\
<input type=\"hidden\" name=\"user_code\" value=\"{safe_code}\">\
<div class=\"row\">\
<button class=\"approve\" name=\"decision\" value=\"approve\" type=\"submit\">Authorize</button>\
<button class=\"deny\" name=\"decision\" value=\"deny\" type=\"submit\">Cancel</button>\
</div></form>",
        member = html_escape(&principal.member_id),
        role = html_escape(principal.role.as_str()),
        org = html_escape(&principal.org_id),
    );
    device_shell("Authorize device", &body)
}

/// Render the post-decision confirmation page.
fn render_device_result(outcome: DeviceOutcome) -> String {
    let body = match outcome {
        DeviceOutcome::Approved => {
            "<h1>Device authorized ✓</h1>\
<p>You can return to your terminal — it will finish signing in automatically. \
You may close this tab.</p>"
        }
        DeviceOutcome::Denied => {
            "<h1>Request denied</h1>\
<p>The device was not authorized. You can close this tab.</p>"
        }
        DeviceOutcome::Unknown => {
            "<h1>Nothing to do</h1>\
<p>This code is unknown, already handled, or expired. \
Start again from your terminal with <code>tellur login</code>.</p>"
        }
    };
    device_shell("Device login", body)
}

