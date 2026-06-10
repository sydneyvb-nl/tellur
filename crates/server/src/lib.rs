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
#[cfg(feature = "dashboard")]
pub mod dashboard;
pub mod error;
pub mod jobs;
pub mod metrics;
pub mod oidc;
pub mod ratelimit;
pub mod review;
pub mod scim;
pub mod storage;

pub use app::{AppState, build_router};
pub use auth::{Principal, Role};
pub use config::Config;
pub use error::ServerError;
pub use metrics::Metrics;

use std::sync::Arc;

use anyhow::Result;

/// Open + migrate the store and assemble application state.
///
/// Selects the backend by configuration: `TELLUR_DATABASE_URL` ⇒ Postgres
/// (horizontal scale), otherwise the embedded SQLite store (zero-config
/// single-node).
pub fn build_state(config: Config) -> Result<AppState> {
    use storage::Store as _;
    let store: Arc<dyn storage::Store> = match &config.database_url {
        Some(url) => {
            tracing::info!("using Postgres storage backend");
            let store = storage::PostgresStore::connect(url)?;
            store.migrate()?;
            Arc::new(store)
        }
        None => {
            tracing::info!(db = %config.db_path.display(), "using SQLite storage backend");
            let store = storage::SqliteStore::open(&config.db_path)?;
            store.migrate()?;
            Arc::new(store)
        }
    };
    // OIDC SSO is enabled only when fully configured (TELLUR_OIDC_*).
    let oidc = oidc::OidcConfig::from_env().map(|cfg| {
        tracing::info!(issuer = %cfg.issuer, "OIDC SSO enabled");
        Arc::new(oidc::OidcRuntime::new(cfg, Arc::new(oidc::HttpOidcClient)))
    });

    Ok(AppState {
        store,
        config: Arc::new(config),
        rate_limiter: Arc::new(ratelimit::RateLimiter::new(
            120,
            std::time::Duration::from_secs(60),
        )),
        metrics: Arc::new(metrics::Metrics::new()),
        oidc,
    })
}

/// Run the server until a shutdown signal is received.
pub async fn run(config: Config) -> Result<()> {
    let bind = config.bind;
    let state = build_state(config)?;

    // Start the durable-job worker (processes queued exports in the background).
    jobs::spawn_worker(state.store.clone());

    // Start the retention loop: expired sessions/logins are always pruned;
    // finished jobs after TELLUR_RETENTION_DAYS and audit entries after
    // TELLUR_AUDIT_RETENTION_DAYS (both default 0 = keep forever).
    let env_days = |k: &str| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let policy = jobs::RetentionPolicy {
        jobs_days: env_days("TELLUR_RETENTION_DAYS"),
        audit_days: env_days("TELLUR_AUDIT_RETENTION_DAYS"),
    };
    jobs::spawn_maintenance(state.store.clone(), policy);

    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "tellur-server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                if result.is_ok() {
                    tracing::info!("ctrl-c received, shutting down");
                }
            }
            _ = sigterm.recv() => {
                tracing::info!("SIGTERM received, shutting down");
            }
        }
    }

    #[cfg(not(unix))]
    {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("ctrl-c received, shutting down");
        }
    }
}
