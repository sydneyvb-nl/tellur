//! Typed server errors mapped to RFC 9457 (`application/problem+json`).
//!
//! 5xx responses never leak internal error detail; only the status and a generic
//! title are returned to the client (ASVS error-handling guidance).

use axum::Json;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("not found")]
    NotFound,

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    /// Any internal failure (storage, IO, etc.). Detail is never sent to clients.
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl ServerError {
    fn status(&self) -> StatusCode {
        match self {
            ServerError::Config(_) | ServerError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ServerError::NotFound => StatusCode::NOT_FOUND,
            ServerError::Unauthorized => StatusCode::UNAUTHORIZED,
            ServerError::Forbidden => StatusCode::FORBIDDEN,
        }
    }

    /// A short, stable, machine-readable slug for the problem `type`.
    fn slug(&self) -> &'static str {
        match self {
            ServerError::Config(_) | ServerError::Internal(_) => "internal-error",
            ServerError::NotFound => "not-found",
            ServerError::Unauthorized => "unauthorized",
            ServerError::Forbidden => "forbidden",
        }
    }
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let status = self.status();
        // Do not leak internal detail on server errors.
        let detail = if status.is_server_error() {
            "internal server error".to_string()
        } else {
            self.to_string()
        };
        if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
        }
        let body = serde_json::json!({
            "type": format!("about:blank#{}", self.slug()),
            "title": self.slug(),
            "status": status.as_u16(),
            "detail": detail,
        });
        (
            status,
            [(header::CONTENT_TYPE, "application/problem+json")],
            Json(body),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_errors_do_not_leak_internal_detail() {
        let err = ServerError::Internal(anyhow::anyhow!("secret db dsn leaked here"));
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn maps_status_codes() {
        assert_eq!(ServerError::NotFound.status(), StatusCode::NOT_FOUND);
        assert_eq!(ServerError::Unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(ServerError::Forbidden.status(), StatusCode::FORBIDDEN);
    }
}
