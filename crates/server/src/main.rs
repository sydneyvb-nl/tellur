//! `tellur-server` binary: run the hub, or perform admin bootstrap tasks.

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
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
        /// Role for the new member. Required so admin tokens are always explicit.
        #[arg(long, value_enum)]
        role: AdminRoleArg,
        #[arg(long, default_value = "token")]
        name: String,
    },
    /// Upload (or update) an org policy from a YAML file.
    SetPolicy {
        #[arg(long)]
        org: String,
        #[arg(long, default_value = "default")]
        name: String,
        #[arg(long)]
        file: std::path::PathBuf,
    },
    /// Grant a member an additive per-repo role.
    GrantRepoRole {
        #[arg(long)]
        org: String,
        /// Repo id or name.
        #[arg(long)]
        repo: String,
        #[arg(long)]
        member: String,
        #[arg(long, value_enum)]
        role: AdminRoleArg,
    },
    /// Revoke a member's per-repo role grant.
    RevokeRepoRole {
        #[arg(long)]
        org: String,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        member: String,
    },
    /// List per-repo role grants for a repo.
    ListRepoRoles {
        #[arg(long)]
        org: String,
        #[arg(long)]
        repo: String,
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

#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
enum AdminRoleArg {
    Viewer,
    Contributor,
    Admin,
}

impl AdminRoleArg {
    fn as_role(self) -> Role {
        match self {
            AdminRoleArg::Viewer => Role::Viewer,
            AdminRoleArg::Contributor => Role::Contributor,
            AdminRoleArg::Admin => Role::Admin,
        }
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
            let role = role.as_role();
            let member = store.create_member(&org, &name, role)?;
            let token = store.create_token(&member)?;
            store.append_audit(&AuditEntry {
                org_id: Some(org),
                actor_member_id: None,
                action: "token.create".to_string(),
                detail: format!("role={} member={member} via=admin-cli", role.as_str()),
            })?;
            println!("Created member id={member} role={}", role.as_str());
            println!("API token (store it now — shown only once):");
            println!("  {}", token.plaintext);
        }
        AdminAction::SetPolicy { org, name, file } => {
            let content = std::fs::read_to_string(&file)?;
            // Validate before storing so a broken policy is never distributed.
            tellur_core::policy::PolicyEngine::from_yaml_str(&content)?;
            let version = store.put_policy(&org, &name, &content)?;
            store.append_audit(&AuditEntry {
                org_id: Some(org),
                actor_member_id: None,
                action: "policy.put".to_string(),
                detail: format!("name={name} version={version} via=admin-cli"),
            })?;
            println!("Stored policy \"{name}\" version {version}");
        }
        AdminAction::GrantRepoRole {
            org,
            repo,
            member,
            role,
        } => {
            let role = role.as_role();
            let repo = store
                .find_repo(&org, &repo)?
                .ok_or_else(|| anyhow::anyhow!("repo {repo} not found in org {org}"))?;
            store.set_repo_role(&org, &repo.id, &member, role)?;
            store.append_audit(&AuditEntry {
                org_id: Some(org),
                actor_member_id: None,
                action: "repo_role.set".to_string(),
                detail: format!(
                    "repo={} member={member} role={} via=admin-cli",
                    repo.id,
                    role.as_str()
                ),
            })?;
            println!(
                "Granted member {member} role {} on repo {}",
                role.as_str(),
                repo.id
            );
        }
        AdminAction::RevokeRepoRole { org, repo, member } => {
            let repo = store
                .find_repo(&org, &repo)?
                .ok_or_else(|| anyhow::anyhow!("repo {repo} not found in org {org}"))?;
            let removed = store.remove_repo_role(&org, &repo.id, &member)?;
            store.append_audit(&AuditEntry {
                org_id: Some(org),
                actor_member_id: None,
                action: "repo_role.remove".to_string(),
                detail: format!(
                    "repo={} member={member} removed={removed} via=admin-cli",
                    repo.id
                ),
            })?;
            println!("Revoke on repo {}: removed={removed}", repo.id);
        }
        AdminAction::ListRepoRoles { org, repo } => {
            let repo = store
                .find_repo(&org, &repo)?
                .ok_or_else(|| anyhow::anyhow!("repo {repo} not found in org {org}"))?;
            let grants = store.list_repo_roles(&org, &repo.id)?;
            if grants.is_empty() {
                println!("No per-repo grants on repo {}", repo.id);
            } else {
                println!("Per-repo grants on repo {}:", repo.id);
                for g in grants {
                    println!(
                        "  member={} role={} updated_at={}",
                        g.member_id, g.role, g.updated_at
                    );
                }
            }
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
