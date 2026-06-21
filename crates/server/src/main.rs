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
    /// Mint an org-scoped SCIM provisioning token (printed once) for an IdP.
    CreateScimToken {
        #[arg(long)]
        org: String,
    },
    /// Provision an SSO member (email-mapped, no API token) so they may sign in
    /// via the configured IdP.
    AddMember {
        #[arg(long)]
        org: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        email: String,
        #[arg(long, value_enum)]
        role: AdminRoleArg,
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
    /// Connect a repo to its source provider (A12) — set deep-link / raw
    /// templates and an optional token for the private-repo proxy.
    SetRepoSource {
        #[arg(long)]
        org: String,
        /// Repo id or name.
        #[arg(long)]
        repo: String,
        /// Web-view deep-link template (https://…/{path}#L{start}-L{end}).
        #[arg(long)]
        link: Option<String>,
        /// Raw-bytes template (https://…/{path}) for the inline gutter.
        #[arg(long)]
        raw: Option<String>,
        /// Provider access token for the private-repo proxy (stored, not logged).
        #[arg(long)]
        token: Option<String>,
        /// Remove the entire source connection for this repo.
        #[arg(long)]
        clear: bool,
    },
    /// Map a GitHub App installation id to an org for webhook ingestion.
    SetGithubInstallation {
        #[arg(long)]
        org: String,
        #[arg(long)]
        installation_id: i64,
        #[arg(long)]
        account: String,
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
        AdminAction::CreateScimToken { org } => {
            let token = store.create_scim_token(&org)?;
            store.append_audit(&AuditEntry {
                org_id: Some(org),
                actor_member_id: None,
                action: "scim_token.create".to_string(),
                detail: "via=admin-cli".to_string(),
            })?;
            println!("SCIM provisioning token (store it now — shown only once):");
            println!("  {}", token.plaintext);
        }
        AdminAction::AddMember {
            org,
            name,
            email,
            role,
        } => {
            let role = role.as_role();
            let member = store.provision_member(&org, &name, role, &email)?;
            store.append_audit(&AuditEntry {
                org_id: Some(org),
                actor_member_id: None,
                action: "member.provision".to_string(),
                detail: format!("member={member} role={} via=admin-cli", role.as_str()),
            })?;
            println!(
                "Provisioned SSO member id={member} role={} (signs in via IdP)",
                role.as_str()
            );
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
        AdminAction::SetRepoSource {
            org,
            repo,
            link,
            raw,
            token,
            clear,
        } => {
            let repo = store
                .find_repo(&org, &repo)?
                .ok_or_else(|| anyhow::anyhow!("repo {repo} not found in org {org}"))?;
            let validate = |t: &Option<String>| -> anyhow::Result<Option<String>> {
                match t.as_deref().map(str::trim) {
                    Some("") | None => Ok(None),
                    Some(v) if v.starts_with("https://") && v.len() <= 2048 => {
                        Ok(Some(v.to_string()))
                    }
                    Some(_) => anyhow::bail!("templates must be https:// URLs under 2048 chars"),
                }
            };
            let (link, raw, token) = if clear {
                (None, None, None)
            } else {
                // Preserve an existing token when --token is omitted (matches the
                // API: editing templates shouldn't require re-entering the secret).
                let tok = token
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .or_else(|| {
                        store
                            .get_repo_source(&org, &repo.id)
                            .ok()
                            .and_then(|s| s.token)
                    });
                (validate(&link)?, validate(&raw)?, tok)
            };
            store.set_repo_source(
                &org,
                &repo.id,
                link.as_deref(),
                raw.as_deref(),
                token.as_deref(),
            )?;
            store.append_audit(&AuditEntry {
                org_id: Some(org),
                actor_member_id: None,
                action: "repo.source.set".to_string(),
                detail: format!(
                    "repo={} link={} raw={} token={} via=admin-cli",
                    repo.id,
                    if link.is_some() { "set" } else { "cleared" },
                    if raw.is_some() { "set" } else { "cleared" },
                    if token.is_some() { "set" } else { "cleared" }
                ),
            })?;
            println!(
                "Source connection for repo {} — link={} raw={} token={}",
                repo.id,
                if link.is_some() { "set" } else { "—" },
                if raw.is_some() { "set" } else { "—" },
                if token.is_some() { "set" } else { "—" }
            );
        }
        AdminAction::SetGithubInstallation {
            org,
            installation_id,
            account,
        } => {
            store.set_github_installation(&org, installation_id, &account)?;
            store.append_audit(&AuditEntry {
                org_id: Some(org),
                actor_member_id: None,
                action: "github_installation.set".to_string(),
                detail: format!(
                    "installation_id={installation_id} account={account} via=admin-cli"
                ),
            })?;
            println!("Mapped GitHub installation {installation_id} ({account})");
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
