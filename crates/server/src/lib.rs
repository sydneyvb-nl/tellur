//! Tellur team/server hub (Tier 1) — self-hostable provenance aggregation.
//!
//! **Licensing:** this crate is source-available under the Functional Source
//! License (FSL-1.1-ALv2), unlike the Apache-2.0 Tellur core it depends on.
//! See `crates/server/LICENSE` and `docs/proposals/LICENSING.md`.
//!
//! B0 scaffolding: secure-by-default config, typed errors, a swappable storage
//! backend, and operational endpoints. Data/tenant endpoints arrive in B1+.

pub mod api;
pub mod app;
pub mod auth;
pub mod config;
pub mod error;
pub mod storage;

pub use app::{AppState, build_router};
pub use auth::{Principal, Role};
pub use config::Config;
pub use error::ServerError;

use std::sync::Arc;

use anyhow::Result;

/// Open + migrate the store and assemble application state.
pub fn build_state(config: Config) -> Result<AppState> {
    use storage::Store as _;
    let store = storage::SqliteStore::open(&config.db_path)?;
    store.migrate()?;
    Ok(AppState {
        store: Arc::new(store),
        config: Arc::new(config),
    })
}

/// Run the server until a shutdown signal is received.
pub async fn run(config: Config) -> Result<()> {
    let bind = config.bind;
    let state = build_state(config)?;
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "tellur-server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    if tokio::signal::ctrl_c().await.is_ok() {
        tracing::info!("shutdown signal received");
    }
}
