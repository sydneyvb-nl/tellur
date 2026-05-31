//! Local HTTP daemon — event ingestion API
//!
//! Runs a lightweight HTTP server for receiving events from AI tools,
//! editor extensions, and CI systems.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Daemon configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub host: String,
    pub port: u16,
    pub repo_root: PathBuf,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 4917,
            repo_root: PathBuf::from("."),
        }
    }
}

/// API response wrapper
#[derive(Serialize)]
#[allow(dead_code)]
struct ApiResponse<T: Serialize> {
    status: String,
    data: T,
}

/// API error response
#[derive(Serialize)]
#[allow(dead_code)]
struct ApiError {
    status: String,
    error: String,
}

/// Run the daemon
pub async fn run_daemon(config: DaemonConfig) -> Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port).parse()?;
    println!("TraceGit daemon listening on {}", addr);
    println!("Repository: {}", config.repo_root.display());

    // For now, use a simple TCP listener approach
    // In production, this would use axum/actix/warp
    let listener = tokio::net::TcpListener::bind(addr).await?;

    println!("Ready to accept events");
    println!("Endpoints:");
    println!("  POST /event        — Submit a single event");
    println!("  POST /events       — Submit multiple events");
    println!("  GET  /status       — Daemon status");
    println!("  GET  /sessions     — List sessions");
    println!("  GET  /attributions — Get file attributions");
    println!("  POST /export       — Generate export bundle");

    loop {
        let (stream, _addr) = listener.accept().await?;
        // Handle connection — simplified for now
        // Full implementation would parse HTTP and route to handlers
        drop(stream);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DaemonConfig::default();
        assert_eq!(config.host, "127.0.0.1");
        assert_eq!(config.port, 4917);
    }
}
