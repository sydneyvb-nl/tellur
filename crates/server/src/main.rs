//! `tellur-server` binary: run the hub, or perform admin bootstrap tasks.

use anyhow::Result;
use clap::{Parser, Subcommand};
use tellur_server::auth::Role;
use tellur_server::storage::AuditEntry;
use tellur_server::{Config, build_state, run};

#[derive(Parser)]
#[command(name = "tellur-server", version, about = "Tellur team hub")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server (default).
    Serve,
    /// Administrative bootstrap commands.
    Admin {
        #[command(subcommand)]
        action: AdminAction,
    },
}

#[derive(Subcommand)]
enum AdminAction {
    /// Create an organization (tenant).
    CreateOrg {
        #[arg(long)]
        name: String,
    },
    /// Create a member and mint an API token (printed once).
    CreateToken {
        #[arg(long)]
        org: String,
        #[arg(long, default_value = "admin")]
        role: String,
        #[arg(long, default_value = "token")]
        name: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let config = Config::from_env()?;

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => run(config).await,
        Command::Admin { action } => run_admin(config, action),
    }
}

fn run_admin(config: Config, action: AdminAction) -> Result<()> {
    let state = build_state(config)?;
    let store = state.store;

    match action {
        AdminAction::CreateOrg { name } => {
            let org = store.create_org(&name)?;
            println!("Created org \"{}\"  id={}", org.name, org.id);
        }
        AdminAction::CreateToken { org, role, name } => {
            let role = Role::parse(&role)?;
            let member = store.create_member(&org, &name, role)?;
            let token = store.create_token(&member)?;
            store.append_audit(&AuditEntry {
                org_id: Some(org),
                actor_member_id: Some(member.clone()),
                action: "token.create".to_string(),
                detail: format!("role={}", role.as_str()),
            })?;
            println!("Created member id={member} role={}", role.as_str());
            println!("API token (store it now — shown only once):");
            println!("  {}", token.plaintext);
        }
    }
    Ok(())
}

/// Structured logging via `RUST_LOG`/`TELLUR_SERVER_LOG`. No secrets/PII logged.
fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};

    let filter = EnvFilter::try_from_env("TELLUR_SERVER_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();
}
