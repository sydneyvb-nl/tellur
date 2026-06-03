//! `tellur-server` binary entry point.

use anyhow::Result;
use tellur_server::{Config, run};

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let config = Config::from_env()?;
    run(config).await
}

/// Structured logging via `RUST_LOG`/`TELLUR_SERVER_LOG`. No secrets/PII are
/// ever logged at any level.
fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = EnvFilter::try_from_env("TELLUR_SERVER_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();
}
