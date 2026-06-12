//! Team dashboard (SPA) asset serving.
//!
//! The dashboard is a Svelte single-page app built into `ui/dist` and **embedded
//! into the binary** (`rust-embed`) so self-hosting stays one binary, zero extra
//! infra. It is served same-origin at `/app/*` — the browser session cookie set
//! by OIDC SSO is therefore first-party (no CORS, no token in the URL), and the
//! SPA calls the existing `/v1/...` JSON API with `credentials: include`.
//!
//! Client-side routing: any `GET /app/*` path that isn't a real asset falls back
//! to `index.html` so deep links (e.g. `/app/orgs/<id>/overview`) load the SPA.
//! Unknown `/v1/*` paths are unaffected and still return JSON 404s.
//!
//! When `ui/dist` was not built (e.g. a plain `cargo build` without the web
//! build step) the embed is empty and `/app` serves a small placeholder.

use axum::Router;
use axum::body::Body;
use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rust_embed::RustEmbed;

use crate::app::AppState;

#[derive(RustEmbed)]
#[folder = "ui/dist"]
struct Assets;

/// Strict same-origin CSP for the dashboard. Scripts/styles/images are
/// self-hosted only. `connect-src` additionally allows a small all-list of
/// well-known source-hosting raw origins so the opt-in file-source gutter (A12)
/// can fetch raw bytes **client-side** straight from the provider (the hub never
/// proxies source). Repos on other providers fall back to the per-range links.
const CSP: &str = "default-src 'self'; img-src 'self' data:; \
style-src 'self' 'unsafe-inline'; script-src 'self'; \
connect-src 'self' https://raw.githubusercontent.com https://gitlab.com https://bitbucket.org; \
object-src 'none'; base-uri 'self'; frame-ancestors 'none'";

/// Shown at `/app` when the SPA hasn't been built into `ui/dist`.
const PLACEHOLDER: &str = "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\">\
<title>Tellur</title><body style=\"font:14px/1.5 system-ui;max-width:40rem;margin:4rem auto;padding:0 1rem\">\
<h1>Tellur dashboard</h1><p>The dashboard assets were not built into this binary. \
Build them with <code>pnpm --dir crates/server/ui install &amp;&amp; pnpm --dir crates/server/ui build</code> \
and rebuild the server, or use the released image.</p></body></html>";

/// Routes that serve the embedded dashboard under `/app`.
pub fn router() -> Router<AppState> {
    // Nesting makes `/app` and `/app/` both hit the index handler, and
    // `/app/<anything>` hit the asset/SPA-fallback handler.
    let inner = Router::new()
        .route("/", get(|| serve(String::new())))
        .route("/{*path}", get(|Path(path): Path<String>| serve(path)));
    Router::new().nest("/app", inner)
}

/// Serve an embedded asset, falling back to `index.html` for SPA client routes.
async fn serve(path: String) -> Response {
    let path = path.trim_start_matches('/');
    if let Some(resp) = asset_response(path) {
        return resp;
    }
    // Not a real embedded asset. SPA client routes live under `orgs/…` (and the
    // bare app root), so serve the app shell for those — note a client route can
    // legitimately end in a filename with a dot (e.g. `…/files/src/api.rs`), so we
    // must NOT treat a trailing dot as an asset request. Anything else that isn't
    // a real asset (e.g. a missing hashed bundle under `assets/`) is a true 404.
    if path.is_empty() || path.starts_with("orgs/") {
        return index_response();
    }
    (StatusCode::NOT_FOUND, "not found").into_response()
}

/// Serve `index.html`, or the placeholder when the SPA wasn't built.
fn index_response() -> Response {
    asset_response("index.html").unwrap_or_else(|| {
        (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CONTENT_SECURITY_POLICY, CSP),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            PLACEHOLDER,
        )
            .into_response()
    })
}

/// Build a response for an embedded asset, if present.
fn asset_response(path: &str) -> Option<Response> {
    let asset = Assets::get(path)?;
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let is_html = path.ends_with(".html");
    // Hashed build assets (Vite emits content-hashed filenames) can be cached
    // hard; HTML must always revalidate so a new deploy is picked up.
    let cache = if is_html {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    let mut resp = (
        [
            (header::CONTENT_TYPE, mime.as_ref()),
            (header::CACHE_CONTROL, cache),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        Body::from(asset.data.into_owned()),
    )
        .into_response();
    // The HTML shell carries the CSP that governs the whole app.
    if is_html {
        resp.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            header::HeaderValue::from_static(CSP),
        );
    }
    Some(resp)
}
