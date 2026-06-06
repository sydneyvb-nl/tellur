//! SCIM 2.0 provisioning (RFC 7643/7644) — `/scim/v2/Users`.
//!
//! Lets an IdP create, update, and **deprovision** hub members automatically.
//! Authentication is a dedicated, org-scoped SCIM bearer token (separate from
//! member API tokens), so the org is derived from the token, not the URL.
//!
//! Scope: the **User** resource (the core of provisioning). A SCIM user maps to
//! a Tellur member + SSO identity: `userName` → email, `displayName`/`name` →
//! display name, `active` → can-authenticate, optional `roles` → org role
//! (default `viewer`). Deprovisioning (`DELETE`, or `PATCH active=false`)
//! deactivates the member so all auth paths (token, session, SSO) reject it.
//! Group-based role sync is intentionally out of scope for this slice.

use axum::Json;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::auth::Role;
use crate::error::ServerError;
use crate::storage::ScimUser;

const USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
const LIST_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";
const ERROR_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:Error";
const SCIM_CONTENT_TYPE: &str = "application/scim+json";

/// An authenticated SCIM caller, scoped to an org by its provisioning token.
pub struct ScimAuth {
    pub org_id: String,
}

impl FromRequestParts<AppState> for ScimAuth {
    type Rejection = ServerError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Some(token) = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(str::to_string)
        else {
            return Err(ServerError::Unauthorized);
        };
        let store = state.store.clone();
        let org = tokio::task::spawn_blocking(move || store.authenticate_scim(&token))
            .await
            .map_err(|e| ServerError::Internal(anyhow::anyhow!("scim auth task failed: {e}")))?
            .map_err(ServerError::Internal)?;
        match org {
            Some(org_id) => Ok(ScimAuth { org_id }),
            None => Err(ServerError::Unauthorized),
        }
    }
}

/// Render a stored user as a SCIM User resource.
fn user_resource(u: &ScimUser) -> Value {
    json!({
        "schemas": [USER_SCHEMA],
        "id": u.member_id,
        "externalId": u.external_id,
        "userName": u.email,
        "displayName": u.display_name,
        "name": { "formatted": u.display_name },
        "active": u.active,
        "emails": [{ "value": u.email, "primary": true }],
        "roles": [{ "value": u.role.as_str() }],
        "meta": {
            "resourceType": "User",
            "location": format!("/scim/v2/Users/{}", u.member_id),
        },
    })
}

/// A SCIM JSON response with the correct content type.
fn scim_json(status: StatusCode, body: Value) -> Response {
    (status, [(CONTENT_TYPE, SCIM_CONTENT_TYPE)], Json(body)).into_response()
}

/// A SCIM error response.
fn scim_error(status: StatusCode, detail: &str) -> Response {
    scim_json(
        status,
        json!({
            "schemas": [ERROR_SCHEMA],
            "status": status.as_u16().to_string(),
            "detail": detail,
        }),
    )
}

/// Inbound SCIM User (POST/PUT). Unknown fields are ignored.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct UserInput {
    #[serde(rename = "userName")]
    pub user_name: Option<String>,
    #[serde(rename = "displayName")]
    pub display_name: Option<String>,
    pub name: Option<NameInput>,
    pub active: Option<bool>,
    #[serde(rename = "externalId")]
    pub external_id: Option<String>,
    pub roles: Option<Vec<RoleInput>>,
    pub emails: Option<Vec<EmailInput>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct NameInput {
    pub formatted: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RoleInput {
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct EmailInput {
    pub value: String,
}

impl UserInput {
    /// Resolve the email (userName, or the primary/first email).
    fn email(&self) -> Option<String> {
        self.user_name.clone().or_else(|| {
            self.emails
                .as_ref()
                .and_then(|e| e.first())
                .map(|e| e.value.clone())
        })
    }

    fn display(&self, fallback: &str) -> String {
        self.display_name
            .clone()
            .or_else(|| self.name.as_ref().and_then(|n| n.formatted.clone()))
            .unwrap_or_else(|| fallback.to_string())
    }

    /// First recognized role value, if any (`viewer`/`contributor`/`admin`).
    fn role(&self) -> Option<Role> {
        self.roles
            .as_ref()?
            .iter()
            .find_map(|r| Role::parse(&r.value).ok())
    }
}

/// Pagination/filter query for `GET /Users`.
#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub filter: Option<String>,
}

/// Parse a SCIM `userName eq "x"` filter into the email value (the only filter
/// we support; anything else yields no filter → full list).
fn parse_username_filter(filter: &str) -> Option<String> {
    let f = filter.trim();
    let rest = f
        .strip_prefix("userName eq ")
        .or_else(|| f.strip_prefix("userName Eq "))?;
    let v = rest.trim().trim_matches('"');
    (!v.is_empty()).then(|| v.to_string())
}

/// `GET /scim/v2/Users` — list users (optionally filtered by `userName`).
pub async fn list_users(
    auth: ScimAuth,
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Response, ServerError> {
    let email_filter = q.filter.as_deref().and_then(parse_username_filter);
    let users = state
        .store
        .scim_list_users(&auth.org_id, email_filter.as_deref())
        .map_err(ServerError::Internal)?;
    let resources: Vec<Value> = users.iter().map(user_resource).collect();
    Ok(scim_json(
        StatusCode::OK,
        json!({
            "schemas": [LIST_SCHEMA],
            "totalResults": resources.len(),
            "startIndex": 1,
            "itemsPerPage": resources.len(),
            "Resources": resources,
        }),
    ))
}

/// `POST /scim/v2/Users` — provision a user.
pub async fn create_user(
    auth: ScimAuth,
    State(state): State<AppState>,
    Json(input): Json<UserInput>,
) -> Result<Response, ServerError> {
    let Some(email) = input.email() else {
        return Ok(scim_error(
            StatusCode::BAD_REQUEST,
            "userName (or an email) is required",
        ));
    };
    // Idempotency: a repeat create for an existing userName is a 409 (SCIM).
    let existing = state
        .store
        .scim_list_users(&auth.org_id, Some(&email))
        .map_err(ServerError::Internal)?;
    if !existing.is_empty() {
        return Ok(scim_error(
            StatusCode::CONFLICT,
            "a user with this userName already exists",
        ));
    }
    let role = input.role().unwrap_or(Role::Viewer);
    let display = input.display(&email);
    match state.store.scim_create_user(
        &auth.org_id,
        &email,
        &display,
        role,
        input.external_id.as_deref(),
    ) {
        Ok(user) => {
            // Honor an explicit active=false at creation time.
            let user = if input.active == Some(false) {
                state
                    .store
                    .scim_update_user(&auth.org_id, &user.member_id, None, None, Some(false), None)
                    .map_err(ServerError::Internal)?
                    .unwrap_or(user)
            } else {
                user
            };
            Ok(scim_json(StatusCode::CREATED, user_resource(&user)))
        }
        Err(_) => Ok(scim_error(
            StatusCode::CONFLICT,
            "a user with this userName already exists",
        )),
    }
}

/// `GET /scim/v2/Users/{id}`.
pub async fn get_user(
    auth: ScimAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ServerError> {
    match state
        .store
        .scim_get_user(&auth.org_id, &id)
        .map_err(ServerError::Internal)?
    {
        Some(u) => Ok(scim_json(StatusCode::OK, user_resource(&u))),
        None => Ok(scim_error(StatusCode::NOT_FOUND, "user not found")),
    }
}

/// `PUT /scim/v2/Users/{id}` — replace mutable attributes.
pub async fn replace_user(
    auth: ScimAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<UserInput>,
) -> Result<Response, ServerError> {
    let display = input.display_name.clone().or_else(|| {
        input
            .name
            .as_ref()
            .and_then(|n| n.formatted.clone())
            .or_else(|| input.email())
    });
    let updated = state
        .store
        .scim_update_user(
            &auth.org_id,
            &id,
            display.as_deref(),
            input.role(),
            input.active,
            input.external_id.as_deref(),
        )
        .map_err(ServerError::Internal)?;
    match updated {
        Some(u) => Ok(scim_json(StatusCode::OK, user_resource(&u))),
        None => Ok(scim_error(StatusCode::NOT_FOUND, "user not found")),
    }
}

/// SCIM PATCH operation body.
#[derive(Debug, Deserialize)]
pub struct PatchOp {
    #[serde(rename = "Operations", default)]
    pub operations: Vec<PatchOperation>,
}

#[derive(Debug, Deserialize)]
pub struct PatchOperation {
    #[serde(default)]
    pub op: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub value: Value,
}

/// `PATCH /scim/v2/Users/{id}` — apply attribute operations (commonly the
/// `active` toggle IdPs use for (de)provisioning).
pub async fn patch_user(
    auth: ScimAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(patch): Json<PatchOp>,
) -> Result<Response, ServerError> {
    let mut active: Option<bool> = None;
    let mut display: Option<String> = None;
    let mut role: Option<Role> = None;

    for op in &patch.operations {
        if !matches!(op.op.to_ascii_lowercase().as_str(), "replace" | "add") {
            continue;
        }
        match op.path.as_deref() {
            Some("active") => active = op.value.as_bool().or(active),
            Some("displayName") => display = op.value.as_str().map(str::to_string).or(display),
            Some("roles") => {
                role = role_from_value(&op.value).or(role);
            }
            // Pathless replace: a partial User object in `value`.
            None => {
                if let Some(a) = op.value.get("active").and_then(Value::as_bool) {
                    active = Some(a);
                }
                if let Some(d) = op.value.get("displayName").and_then(Value::as_str) {
                    display = Some(d.to_string());
                }
                if let Some(r) = role_from_value(op.value.get("roles").unwrap_or(&Value::Null)) {
                    role = Some(r);
                }
            }
            _ => {}
        }
    }

    let updated = state
        .store
        .scim_update_user(&auth.org_id, &id, display.as_deref(), role, active, None)
        .map_err(ServerError::Internal)?;
    match updated {
        Some(u) => Ok(scim_json(StatusCode::OK, user_resource(&u))),
        None => Ok(scim_error(StatusCode::NOT_FOUND, "user not found")),
    }
}

/// Extract a role from a SCIM `roles` value (array of `{value}` or a string).
fn role_from_value(v: &Value) -> Option<Role> {
    match v {
        Value::String(s) => Role::parse(s).ok(),
        Value::Array(items) => items.iter().find_map(|i| {
            i.get("value")
                .and_then(Value::as_str)
                .and_then(|s| Role::parse(s).ok())
        }),
        _ => None,
    }
}

/// `DELETE /scim/v2/Users/{id}` — deprovision (deactivate). Returns 204.
pub async fn delete_user(
    auth: ScimAuth,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ServerError> {
    let updated = state
        .store
        .scim_update_user(&auth.org_id, &id, None, None, Some(false), None)
        .map_err(ServerError::Internal)?;
    match updated {
        Some(_) => Ok(StatusCode::NO_CONTENT.into_response()),
        None => Ok(scim_error(StatusCode::NOT_FOUND, "user not found")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_username_filter() {
        assert_eq!(
            parse_username_filter("userName eq \"a@b.com\"").as_deref(),
            Some("a@b.com")
        );
        assert!(parse_username_filter("displayName eq \"x\"").is_none());
    }

    #[test]
    fn maps_role_from_value() {
        assert_eq!(
            role_from_value(&json!([{ "value": "admin" }])),
            Some(Role::Admin)
        );
        assert_eq!(
            role_from_value(&json!("contributor")),
            Some(Role::Contributor)
        );
        assert_eq!(role_from_value(&json!([{ "value": "nope" }])), None);
    }
}
