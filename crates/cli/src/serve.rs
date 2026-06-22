//! Long-running server entrypoints: the local HTTP `daemon` and the stdio `mcp`
//! server.

use anyhow::Result;

use tellur_core::storage::RepoStorage;

pub(crate) async fn cmd_daemon(host: &str, port: u16) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }
    let config = tellur_core::daemon::DaemonConfig {
        host: host.to_string(),
        port,
        repo_root: storage.root.clone(),
    };
    tellur_core::daemon::run_daemon(config).await
}

pub(crate) fn cmd_mcp() -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        eprintln!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }
    tellur_core::mcp::serve_stdio(&storage.root)
}
