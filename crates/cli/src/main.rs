//! Tellur CLI — AI Code Provenance from the terminal
//!
//! Commands:
//!   tellur init       — Initialize Tellur in a repository
//!   tellur doctor     — Check setup and detect AI tools
//!   tellur status     — Show current status
//!   tellur explain    — Explain who/what changed a line
//!   tellur blame      — Show AI attribution for a file
//!   tellur pr-report  — Generate a PR risk report
//!   tellur policy     — Check policy compliance
//!   tellur export     — Export provenance data
//!   tellur watch      — Start capturing AI development activity
//!   tellur event      — Emit a single event (generic adapter)
//!   tellur gc         — Garbage collect expired data
//!   tellur verify     — Verify provenance integrity

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

use tellur_core::capture::{
    CaptureContext, capture_working_changes, capture_working_changes_for_paths,
};
use tellur_core::policy::PolicyEngine;
use tellur_core::schema::types::{Actor, AgentInfo, EventActor, ModelInfo, Session};
use tellur_core::storage::{EventWriter, RepoStorage, TraceIndex};

#[derive(Parser)]
#[command(name = "tellur")]
#[command(
    version,
    about = "AI Code Provenance — line-level attribution, session replay, PR risk reports"
)]
#[command(
    long_about = "Tellur records, attributes, and reports on AI-assisted development.\n\n\
Git tells you what changed. Tellur tells you how AI participated."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize Tellur in the current repository
    Init {
        /// Setup profile: default | team | oss-maintainer
        #[arg(long, default_value = "default")]
        profile: String,
    },

    /// Check Tellur setup and detect AI tools
    Doctor,

    /// Show current Tellur status
    Status,

    /// Explain who/what changed a specific line
    Explain {
        /// File path and line number (e.g., src/main.rs:42)
        target: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show AI attribution for a file
    Blame {
        /// File path
        file: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Generate a PR risk report
    PrReport {
        /// Base ref (default: main)
        #[arg(long, default_value = "main")]
        base: String,
        /// Head ref (default: HEAD)
        #[arg(long, default_value = "HEAD")]
        head: String,
    },

    /// Check policy compliance
    Policy {
        #[command(subcommand)]
        action: PolicyActions,
    },

    /// Export provenance data
    Export {
        /// Export format: native | agent-trace | markdown | json
        #[arg(long, default_value = "native")]
        format: String,
        /// Output file (stdout if omitted)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Import events from an external source
    Import {
        /// Adapter to import from: claude-code | aider | cursor | generic | codex | copilot | gemini-cli | antigravity | windsurf | jetbrains | devin | continue | cline
        adapter: String,
        /// Source path
        source: PathBuf,
    },

    /// Start watching for AI development activity
    Watch {
        /// Agent/tool identifier to attach to inferred file changes
        #[arg(long, default_value = "watch")]
        agent_id: String,
        /// Human-readable agent/tool name for the session list
        #[arg(long, default_value = "Tellur Watch")]
        agent_name: String,
        /// Optional model identifier, for example openai:gpt-5 or copilot:gpt-4.1
        #[arg(long)]
        model_id: Option<String>,
    },

    /// Emit a single event (for generic adapter / CI)
    Event {
        /// Event type (e.g., file.write, command.post_execute)
        #[arg(long)]
        event_type: String,
        /// Session ID
        #[arg(long)]
        session: String,
        /// File path (for file events)
        #[arg(long)]
        file: Option<String>,
        /// Command (for command events)
        #[arg(long)]
        command: Option<String>,
        /// Exit code (for command events)
        #[arg(long)]
        exit_code: Option<i32>,
        /// Structured JSON payload to merge into the event payload
        #[arg(long)]
        payload_json: Option<String>,
    },

    /// Garbage collect expired data
    Gc {
        /// Dry run — show what would be deleted
        #[arg(long)]
        dry_run: bool,
    },

    /// Verify provenance integrity
    Verify,

    /// Redact sensitive content from stored events
    Redact,

    /// Show session details
    Sessions {
        /// Specific session ID to show
        session_id: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Run the local HTTP daemon (event ingestion API)
    Daemon {
        /// Host to bind (loopback only)
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Port to bind
        #[arg(long, default_value_t = 4917)]
        port: u16,
    },

    /// Run the MCP server over stdio (for AI agents)
    Mcp,

    /// Manage Git AI-compatible authorship notes (default ref: refs/notes/ai)
    Notes {
        #[command(subcommand)]
        action: NotesActions,
    },

    /// Team-level reports aggregated from Git AI authorship notes (no server)
    Team {
        #[command(subcommand)]
        action: TeamActions,
    },

    /// Manage editor/agent hook integrations
    Hooks {
        #[command(subcommand)]
        action: HookActions,
    },

    /// Install one-time global integrations for AI coding agents
    Setup {
        #[command(subcommand)]
        action: SetupActions,
    },
}

#[derive(Subcommand)]
enum NotesActions {
    /// Export indexed attribution to a Git AI-compatible note
    Export {
        /// Commit to annotate
        #[arg(default_value = "HEAD")]
        commit: String,
        /// Notes ref to write
        #[arg(long, default_value = tellur_core::notes::GIT_AI_NOTES_REF)]
        notes_ref: String,
        /// Print note content instead of writing Git notes
        #[arg(long)]
        print: bool,
    },
    /// Show and parse the authorship note for a commit
    Show {
        /// Commit to inspect
        #[arg(default_value = "HEAD")]
        commit: String,
        /// Notes ref to read
        #[arg(long, default_value = tellur_core::notes::GIT_AI_NOTES_REF)]
        notes_ref: String,
        /// Output parsed note as JSON
        #[arg(long)]
        json: bool,
    },
    /// Import a Git AI authorship note into the local Tellur index
    Import {
        /// Commit to import notes from
        #[arg(default_value = "HEAD")]
        commit: String,
        /// Notes ref to read
        #[arg(long, default_value = tellur_core::notes::GIT_AI_NOTES_REF)]
        notes_ref: String,
    },
    /// Fetch authorship notes from a remote
    Fetch {
        /// Remote name
        #[arg(default_value = "origin")]
        remote: String,
        /// Notes ref to fetch
        #[arg(long, default_value = tellur_core::notes::GIT_AI_NOTES_REF)]
        notes_ref: String,
    },
    /// Push authorship notes to a remote
    Push {
        /// Remote name
        #[arg(default_value = "origin")]
        remote: String,
        /// Notes ref to push
        #[arg(long, default_value = tellur_core::notes::GIT_AI_NOTES_REF)]
        notes_ref: String,
    },
    /// Configure this repository to fetch and rewrite authorship notes
    InstallConfig {
        /// Remote name
        #[arg(default_value = "origin")]
        remote: String,
        /// Notes ref to configure
        #[arg(long, default_value = tellur_core::notes::GIT_AI_NOTES_REF)]
        notes_ref: String,
    },
}

#[derive(Subcommand)]
enum TeamActions {
    /// Aggregate AI involvement across a commit range from Git authorship notes
    Report {
        /// Base ref (default: main)
        #[arg(long, default_value = "main")]
        base: String,
        /// Head ref (default: HEAD)
        #[arg(long, default_value = "HEAD")]
        head: String,
        /// Notes ref to read
        #[arg(long, default_value = tellur_core::notes::GIT_AI_NOTES_REF)]
        notes_ref: String,
        /// Output the report as JSON instead of Markdown
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum HookActions {
    /// Install Tellur hooks into Claude Code settings (.claude/settings.json)
    Install {
        /// Which tool's hooks to install
        #[arg(default_value = "claude-code")]
        tool: String,
    },
    /// Internal: handle a Claude Code hook payload from stdin
    #[command(hide = true)]
    Claude,
    /// Internal: ingest a supported agent hook payload from stdin
    #[command(hide = true)]
    Ingest {
        /// Hook source: claude-code | codex | gemini-cli | antigravity | vscode | cursor
        #[arg(long)]
        source: String,
        /// Initialize Tellur automatically when inside a Git repository
        #[arg(long)]
        auto_init: bool,
        /// Print an empty JSON object for hook systems that require JSON stdout
        #[arg(long, hide = true)]
        json_response: bool,
    },
}

#[derive(Subcommand)]
enum SetupActions {
    /// Install global Codex, Claude Code, Cursor, and VS Code integrations
    Agents {
        /// Override home directory, intended for tests and portable installs
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Install global Codex integration
    Codex {
        /// Override home directory, intended for tests and portable installs
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Install global Claude Code integration
    ClaudeCode {
        /// Override home directory, intended for tests and portable installs
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Install global Cursor integration
    Cursor {
        /// Override home directory, intended for tests and portable installs
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Install global VS Code integration
    Vscode {
        /// Override home directory, intended for tests and portable installs
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Install global Windsurf / Cascade integration
    Windsurf {
        /// Override home directory, intended for tests and portable installs
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Install global Gemini CLI integration
    GeminiCli {
        /// Override home directory, intended for tests and portable installs
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Install global Google Antigravity 2.0 integration
    Antigravity {
        /// Override home directory, intended for tests and portable installs
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Show global integration status
    Status {
        /// Override home directory, intended for tests and portable installs
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Remove global integrations installed by Tellur
    Uninstall {
        /// Override home directory, intended for tests and portable installs
        #[arg(long)]
        home: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum PolicyActions {
    /// Check all policies
    Check,
    /// Explain what a policy does
    Explain {
        /// Policy rule ID
        rule_id: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { profile } => cmd_init(&profile).await,
        Commands::Doctor => cmd_doctor().await,
        Commands::Status => cmd_status(),
        Commands::Explain { target, json } => cmd_explain(&target, json),
        Commands::Blame { file, json } => cmd_blame(&file, json),
        Commands::PrReport { base, head } => cmd_pr_report(&base, &head),
        Commands::Policy { action } => match action {
            PolicyActions::Check => cmd_policy_check(),
            PolicyActions::Explain { rule_id } => cmd_policy_explain(rule_id.as_deref()),
        },
        Commands::Export { format, output } => cmd_export(&format, output.as_deref()),
        Commands::Import { adapter, source } => cmd_import(&adapter, &source).await,
        Commands::Watch {
            agent_id,
            agent_name,
            model_id,
        } => cmd_watch(&agent_id, &agent_name, model_id).await,
        Commands::Event {
            event_type,
            session,
            file,
            command,
            exit_code,
            payload_json,
        } => cmd_event(
            &event_type,
            &session,
            file.as_deref(),
            command.as_deref(),
            exit_code,
            payload_json.as_deref(),
        ),
        Commands::Gc { dry_run } => cmd_gc(dry_run),
        Commands::Verify => cmd_verify(),
        Commands::Redact => cmd_redact(),
        Commands::Sessions { session_id, json } => cmd_sessions(session_id.as_deref(), json),
        Commands::Daemon { host, port } => cmd_daemon(&host, port).await,
        Commands::Mcp => cmd_mcp(),
        Commands::Notes { action } => match action {
            NotesActions::Export {
                commit,
                notes_ref,
                print,
            } => cmd_notes_export(&commit, &notes_ref, print),
            NotesActions::Show {
                commit,
                notes_ref,
                json,
            } => cmd_notes_show(&commit, &notes_ref, json),
            NotesActions::Import { commit, notes_ref } => cmd_notes_import(&commit, &notes_ref),
            NotesActions::Fetch { remote, notes_ref } => cmd_notes_fetch(&remote, &notes_ref),
            NotesActions::Push { remote, notes_ref } => cmd_notes_push(&remote, &notes_ref),
            NotesActions::InstallConfig { remote, notes_ref } => {
                cmd_notes_install_config(&remote, &notes_ref)
            }
        },
        Commands::Team { action } => match action {
            TeamActions::Report {
                base,
                head,
                notes_ref,
                json,
            } => cmd_team_report(&base, &head, &notes_ref, json),
        },
        Commands::Hooks { action } => match action {
            HookActions::Install { tool } => cmd_hooks_install(&tool),
            HookActions::Claude => cmd_hooks_claude(),
            HookActions::Ingest {
                source,
                auto_init,
                json_response,
            } => cmd_hooks_ingest(&source, auto_init, json_response),
        },
        Commands::Setup { action } => match action {
            SetupActions::Agents { home } => cmd_setup_agents(home.as_deref()),
            SetupActions::Codex { home } => cmd_setup_codex(home.as_deref()),
            SetupActions::ClaudeCode { home } => cmd_setup_claude_code(home.as_deref()),
            SetupActions::Cursor { home } => cmd_setup_cursor(home.as_deref()),
            SetupActions::Vscode { home } => cmd_setup_vscode(home.as_deref()),
            SetupActions::Windsurf { home } => cmd_setup_windsurf(home.as_deref()),
            SetupActions::GeminiCli { home } => cmd_setup_gemini_cli(home.as_deref()),
            SetupActions::Antigravity { home } => cmd_setup_antigravity(home.as_deref()),
            SetupActions::Status { home } => cmd_setup_status(home.as_deref()),
            SetupActions::Uninstall { home } => cmd_setup_uninstall(home.as_deref()),
        },
    }
}

/// Build an Actor for the current OS/git user.
fn current_actor() -> Actor {
    let name = std::env::var("GIT_AUTHOR_NAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_string());
    Actor {
        name,
        email: std::env::var("GIT_AUTHOR_EMAIL").ok(),
        email_hash: None,
        actor_type: EventActor::Human,
    }
}

/// Load the first policy engine from the policies dir, if any.
fn load_policy(storage: &RepoStorage) -> Option<PolicyEngine> {
    let path = storage.policies_dir.join("default.yml");
    PolicyEngine::load_from_file(&path).ok()
}

// ─── Command Implementations ────────────────────────────────────────────────

async fn cmd_init(profile: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if storage.is_initialized() {
        println!("Tellur already initialized. Run `tellur doctor` to check setup.");
        return Ok(());
    }

    storage.init()?;
    println!("✓ Tellur initialized (profile: {})", profile);
    println!("  Config: {}", storage.config_path.display());
    println!("  Policies: {}", storage.policies_dir.display());
    println!("  Traces: {}", storage.traces_dir.display());
    println!();
    println!("Next: run `tellur doctor` to verify setup");
    Ok(())
}

async fn cmd_doctor() -> Result<()> {
    let storage = RepoStorage::discover()?;

    println!("Tellur Doctor");
    println!("═══════════════");
    println!();

    // Check config
    if storage.is_initialized() {
        println!("✓ Config found");
    } else {
        println!("✗ Config not found — run `tellur init` first");
    }

    // Check policies
    let policies: Vec<_> = match std::fs::read_dir(&storage.policies_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "yml"))
            .collect(),
        Err(_) => Vec::new(),
    };
    println!(
        "✓ {} polic{} found",
        policies.len(),
        if policies.len() == 1 { "y" } else { "ies" }
    );
    for p in &policies {
        println!(
            "  - {}",
            p.path().file_name().unwrap_or_default().to_string_lossy()
        );
    }

    // Check index
    if storage.index_path.exists() {
        let index = TraceIndex::open(&storage.index_path)?;
        let events = index.event_count()?;
        let sessions = index.session_count()?;
        println!("✓ Index found ({} events, {} sessions)", events, sessions);
    } else {
        println!("⚠ No index yet");
    }

    // Check traces
    if storage.traces_dir.exists() {
        let trace_files: Vec<_> = match std::fs::read_dir(&storage.traces_dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "jsonl"))
                .collect(),
            Err(_) => Vec::new(),
        };
        println!("✓ Traces directory ({} log files)", trace_files.len());
    }

    // Detect AI tools
    println!();
    println!("AI Tool Detection:");
    let mut detected = 0;

    // Check for Claude Code
    if std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join(".claude")
        .exists()
    {
        detected += 1;
        println!("  ✓ Claude Code (~/.claude found)");
    }

    // Check for Cursor
    if storage.root.join(".cursor").exists() {
        detected += 1;
        println!("  ✓ Cursor (.cursor/ found)");
    }

    // Check for Aider
    if std::process::Command::new("which")
        .arg("aider")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        detected += 1;
        println!("  ✓ Aider (installed)");
    }

    // Check for Codex CLI
    if std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
        .join(".codex")
        .exists()
        || std::process::Command::new("which")
            .arg("codex")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    {
        detected += 1;
        println!("  ✓ Codex CLI (~/.codex or codex binary found)");
    }

    // Check for common Copilot workspace config
    if storage
        .root
        .join(".github")
        .join("copilot-instructions.md")
        .exists()
    {
        detected += 1;
        println!("  ✓ GitHub Copilot instructions (.github/copilot-instructions.md found)");
    }

    if detected == 0 {
        println!("  No AI coding tools detected");
    }

    println!();
    if storage.is_initialized() {
        println!("Setup looks good. Run `tellur watch` to start capturing.");
    }

    Ok(())
}

fn cmd_status() -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let index = TraceIndex::open(&storage.index_path)?;
    let events = index.event_count()?;
    let sessions = index.session_count()?;

    println!("Sessions: {}", sessions);
    println!("Events: {}", events);

    if events == 0 {
        println!();
        println!("No events recorded yet. Run `tellur watch` to start capturing.");
    }

    Ok(())
}

fn cmd_explain(target: &str, json: bool) -> Result<()> {
    // Parse file:line format
    let (file, line) = if let Some((f, l)) = target.rsplit_once(':') {
        let line_num: u32 = l.parse().context("Invalid line number")?;
        (f, line_num)
    } else {
        anyhow::bail!("Usage: tellur explain <file>:<line>");
    };

    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let index = TraceIndex::open(&storage.index_path)?;
    let attributions = index.get_file_attributions(file)?;

    // Find the range that contains this line.
    let found = attributions
        .iter()
        .find(|(_, attr)| line >= attr.start_line && line <= attr.end_line);

    if json {
        let payload = match found {
            Some((_, attr)) => serde_json::json!({
                "file_path": file,
                "line": line,
                "origin": format!("{:?}", attr.origin).to_lowercase(),
                "confidence": attr.confidence,
                "evidence_strength": format!("{:?}", attr.evidence_strength).to_lowercase(),
                "state": format!("{:?}", attr.state).to_lowercase(),
                "agent_id": attr.agent_id,
                "model_id": attr.model_id,
                "session_id": attr.session_id,
                "prompt_hash": attr.prompt_hash,
                "risk_level": attr.risk_level.as_ref().map(|r| format!("{:?}", r).to_lowercase()),
                "policy_tags": attr.policy_tags,
            }),
            None => serde_json::json!(null),
        };
        println!("{}", serde_json::to_string(&payload)?);
        return Ok(());
    }

    let Some((_, attr)) = found else {
        if attributions.is_empty() {
            println!("No attribution data for {}", file);
            println!("Run `tellur watch` (or install hooks) to start capturing AI activity.");
        } else {
            println!("Line {} in {} — no AI attribution recorded", line, file);
        }
        return Ok(());
    };

    println!("Line {} in {}", line, file);
    println!();
    println!("Origin:     {:?}", attr.origin);
    println!("Evidence:   {:?}", attr.evidence_strength);
    println!("Confidence: {:.0}%", attr.confidence * 100.0);
    println!("State:      {:?}", attr.state);
    println!("Session:    {}", attr.session_id);
    println!("Agent:      {}", attr.agent_id);
    if let Some(ref model) = attr.model_id {
        println!("Model:      {}", model);
    }
    if let Some(ref ph) = attr.prompt_hash {
        println!("Prompt:     {}", ph);
    }
    if let Some(ref reviewer) = attr.reviewer {
        println!("Reviewer:   {}", reviewer);
    }
    if !attr.tests_run.is_empty() {
        println!("Tests:      {}", attr.tests_run.join(", "));
        println!("Tests pass: {}", attr.tests_passed);
    }
    if !attr.policy_tags.is_empty() {
        println!("Tags:       {}", attr.policy_tags.join(", "));
    }
    Ok(())
}

fn cmd_blame(file: &str, json: bool) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let index = TraceIndex::open(&storage.index_path)?;
    let attributions = index.get_file_attributions(file)?;

    if json {
        let ranges: Vec<_> = attributions.iter().map(|(_, a)| a).collect();
        let payload = serde_json::json!({ "file_path": file, "ranges": ranges });
        println!("{}", serde_json::to_string(&payload)?);
        return Ok(());
    }

    if attributions.is_empty() {
        println!("No attribution data for {}", file);
        return Ok(());
    }

    println!("Attribution for {}", file);
    println!("─────────────────────────────────────────────");
    for (_blob_sha, attr) in &attributions {
        println!(
            "  L{:3}-{:<3} {:?} {} conf={:.0}% [{:?}]",
            attr.start_line,
            attr.end_line,
            attr.origin,
            attr.agent_id,
            attr.confidence * 100.0,
            attr.state,
        );
    }

    Ok(())
}

fn cmd_pr_report(base: &str, head: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let report = tellur_core::report::build_repo_pr_report(&storage, base, head)?;
    println!(
        "{}",
        tellur_core::report::PRReportGenerator::to_markdown(&report)
    );
    Ok(())
}

fn cmd_policy_check() -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let policy_path = storage.policies_dir.join("default.yml");
    if !policy_path.exists() {
        println!("No policy file found.");
        return Ok(());
    }

    let engine = tellur_core::policy::PolicyEngine::load_from_file(&policy_path)?;
    let policy = engine.policy();

    println!("Policy Check");
    println!("════════════");
    println!();

    if let Some(ref paths) = policy.sensitive_paths {
        println!("Sensitive paths ({}):", paths.len());
        for sp in paths {
            println!("  {} [{}]", sp.path, sp.tags.join(", "));
        }
    }

    if let Some(ref rules) = policy.rules {
        if rules.is_empty() {
            println!("Custom rules: none");
        } else {
            println!("Custom rules ({}):", rules.len());
            for rule in rules {
                println!("  {} — {}", rule.id, rule.description);
            }
        }
    }

    Ok(())
}

fn cmd_policy_explain(rule_id: Option<&str>) -> Result<()> {
    let storage = RepoStorage::discover()?;
    let policy_path = storage.policies_dir.join("default.yml");
    if !policy_path.exists() {
        println!("No policy file found.");
        return Ok(());
    }

    let engine = tellur_core::policy::PolicyEngine::load_from_file(&policy_path)?;
    let policy = engine.policy();

    if let Some(id) = rule_id {
        if let Some(ref rules) = policy.rules {
            if let Some(rule) = rules.iter().find(|r| r.id == id) {
                println!("Rule: {}", rule.id);
                println!("Description: {}", rule.description);
                if let Some(ref rationale) = rule.rationale {
                    println!("Rationale: {}", rationale);
                }
                println!("Action: {:?}", rule.action);
                println!("When: {}", serde_json::to_string_pretty(&rule.when)?);
            } else {
                println!("Rule '{}' not found.", id);
            }
        }
    } else {
        println!("Available rules:");
        if let Some(ref rules) = policy.rules {
            for rule in rules {
                println!("  {} — {}", rule.id, rule.description);
            }
        }
        if policy.rules.is_none() || policy.rules.as_ref().map(|r| r.is_empty()).unwrap_or(true) {
            println!("  (no custom rules defined)");
        }
    }

    Ok(())
}

fn cmd_export(format: &str, output: Option<&std::path::Path>) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let events = tellur_core::storage::read_events(&storage.traces_dir)?;
    if events.is_empty() {
        println!("No events to export.");
        return Ok(());
    }

    let result = match format {
        "json" => serde_json::to_string_pretty(&events)?,
        "jsonl" => events
            .iter()
            .map(|e| serde_json::to_string(e).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n"),
        "markdown" | "md" => {
            let mut md = String::from("# Tellur Export\n\n");
            for e in &events {
                md.push_str(&format!("## Event {}\n", e.id));
                md.push_str(&format!("- **Session:** {}\n", e.session_id));
                md.push_str(&format!("- **Time:** {}\n", e.timestamp));
                md.push_str(&format!("- **Type:** {:?}\n", e.event_type));
                md.push_str(&format!("- **Actor:** {:?}\n", e.actor));
                if !e.payload.is_null() {
                    md.push_str(&format!("- **Payload:** `{}`\n", e.payload));
                }
                md.push('\n');
            }
            md
        }
        _ => serde_json::to_string_pretty(&events)?,
    };

    match output {
        Some(path) => {
            std::fs::write(path, &result)?;
            println!("Exported {} events to {}", events.len(), path.display());
        }
        None => println!("{}", result),
    }

    Ok(())
}

async fn cmd_import(adapter: &str, source: &std::path::Path) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    println!("Importing from {} adapter: {}", adapter, source.display());

    let events: Vec<tellur_core::schema::types::TraceEvent> = match adapter {
        "claude-code" | "claude" => {
            let a = tellur_adapters::ClaudeCodeAdapter::new();
            a.parse_transcript(source, "imported")?
        }
        "aider" => {
            let a = tellur_adapters::AiderAdapter::new();
            if !source.is_dir() {
                anyhow::bail!(
                    "Aider import source must be a git repository directory: {}",
                    source.display()
                );
            }
            a.parse_git_log(source, "2020-01-01")?
        }
        "cursor" => {
            let a = tellur_adapters::CursorAdapter::new();
            a.parse_trace_file(source, "imported")?
        }
        "generic" => {
            let a = tellur_adapters::GenericAdapter::new();
            a.import_jsonl(source)?
        }
        "codex" | "codex-cli" => {
            let a = tellur_adapters::CodexAdapter::new();
            a.parse_jsonl(source, "imported")?
        }
        "copilot" | "github-copilot" => {
            let a = tellur_adapters::CopilotAdapter::new();
            a.parse_metadata_file(source, "imported")?
        }
        "gemini" | "gemini-cli" => {
            let a = tellur_adapters::GeminiAdapter::new();
            a.parse_jsonl(source, "imported")?
        }
        "antigravity" | "google-antigravity" => {
            let a = tellur_adapters::AntigravityAdapter::new();
            a.parse_jsonl(source, "imported")?
        }
        "windsurf" | "cascade" => {
            let a = tellur_adapters::WindsurfAdapter::new();
            a.parse_jsonl(source, "imported")?
        }
        "jetbrains" | "jetbrains-ai" | "junie" => {
            let a = tellur_adapters::JetBrainsAdapter::new();
            a.parse_export(source, "imported")?
        }
        "devin" => {
            let a = tellur_adapters::DevinAdapter::new();
            a.parse_export(source, "imported")?
        }
        "continue" | "continue-dev" => {
            let a = tellur_adapters::ContinueAdapter::new();
            a.parse_jsonl(source, "imported")?
        }
        "cline" | "roo" | "roo-code" => {
            let a = tellur_adapters::ClineAdapter::new();
            a.parse_task(source, "imported")?
        }
        _ => {
            println!(
                "Unknown adapter: {}. Supported: claude-code, aider, cursor, generic, codex, copilot, gemini-cli, antigravity, windsurf, jetbrains, devin, continue, cline",
                adapter
            );
            return Ok(());
        }
    };

    if events.is_empty() {
        println!("No events found to import.");
        return Ok(());
    }

    // Write events via EventWriter for hash chain integrity
    let mut writer = EventWriter::new(&storage.traces_dir);
    writer.open()?;
    let index = TraceIndex::open(&storage.index_path)?;
    let mut count = 0u32;
    for e in events {
        // Preserve source identity/timestamps while recomputing the local hash chain.
        let event = writer.write_imported_event(e)?;
        index.index_event(&event)?;
        count += 1;
    }
    writer.close();

    println!("Imported {} events from {}", count, adapter);
    Ok(())
}

async fn cmd_watch(agent_id: &str, agent_name: &str, model_id: Option<String>) -> Result<()> {
    use notify::{RecursiveMode, Watcher};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{RecvTimeoutError, channel};
    use std::time::Duration;

    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    println!("Tellur Watch");
    println!("══════════════");
    println!("Watching {} for changes...", storage.root.display());
    println!("Press Ctrl+C to stop.");
    println!();

    // Create and index a watch session.
    let repo_id = tellur_core::schema::ids::hash_content(&storage.root.to_string_lossy());
    let session = Session::new(
        repo_id,
        current_actor(),
        AgentInfo {
            id: agent_id.to_string(),
            name: agent_name.to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        },
    );
    let session = if let Some(model_id) = model_id.as_deref() {
        let mut parts = model_id.splitn(2, ':');
        let provider = parts.next().unwrap_or("unknown").to_string();
        let name = parts.next().unwrap_or(model_id).to_string();
        Session {
            model: Some(ModelInfo {
                provider,
                name,
                version: None,
            }),
            ..session
        }
    } else {
        session
    };
    let session_id = session.id.clone();
    let index = TraceIndex::open(&storage.index_path)?;
    index.index_session(&session)?;
    println!("Session: {}", session_id);
    println!();

    let mut writer = EventWriter::new(&storage.traces_dir);
    writer.open()?;
    writer.write_event(
        &session_id,
        "session.start",
        "agent",
        serde_json::json!({
            "mode": "watch",
            "tool": "tellur-cli",
            "agent_id": agent_id,
            "model_id": model_id,
        }),
        None,
    )?;

    let policy = load_policy(&storage);
    let ctx = CaptureContext::inferred_watch_with_metadata(&session_id, agent_id, model_id.clone());

    // Filesystem watcher → debounce → capture.
    let (tx, rx) = channel();
    let mut watcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })?;
    watcher.watch(&storage.root, RecursiveMode::Recursive)?;

    let running = Arc::new(AtomicBool::new(true));
    {
        let r = running.clone();
        let _ = ctrlc::set_handler(move || r.store(false, Ordering::SeqCst));
    }

    // Initial capture of any pre-existing working-tree changes.
    run_capture(&storage, &mut writer, &index, policy.as_ref(), &ctx);

    let mut dirty = false;
    while running.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(400)) {
            Ok(event) => {
                // Ignore our own metadata and noisy build/vendor dirs.
                let relevant = event
                    .paths
                    .iter()
                    .any(|p| tellur_core::storage::file_watcher::should_track(p, &storage.root));
                if relevant {
                    dirty = true;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if dirty {
                    let summary = run_capture(&storage, &mut writer, &index, policy.as_ref(), &ctx);
                    if summary > 0 {
                        println!("  captured {} change(s)", summary);
                    }
                    dirty = false;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    writer.write_event(
        &session_id,
        "session.end",
        "system",
        serde_json::json!({"mode": "watch"}),
        None,
    )?;
    writer.close();
    println!();
    println!("Watch stopped. Session {} ended.", session_id);
    Ok(())
}

/// Run one capture pass, printing errors but never aborting the watch loop.
/// Returns the number of files captured.
fn run_capture(
    storage: &RepoStorage,
    writer: &mut EventWriter,
    index: &TraceIndex,
    policy: Option<&PolicyEngine>,
    ctx: &CaptureContext,
) -> usize {
    match capture_working_changes(storage, writer, index, policy, ctx) {
        Ok(summary) => {
            for blocked in &summary.skipped_blocked {
                eprintln!("  skipped (block_ai_read): {}", blocked);
            }
            summary.files_captured
        }
        Err(e) => {
            eprintln!("  capture error: {}", e);
            0
        }
    }
}

fn cmd_event(
    event_type: &str,
    session: &str,
    file: Option<&str>,
    command: Option<&str>,
    exit_code: Option<i32>,
    payload_json: Option<&str>,
) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    // The event type is used verbatim as the wire string. Unknown types are
    // preserved as `custom` rather than being coerced, so no information is
    // lost (e.g. `command.post_execute` keeps its underscore).
    let normalized_type = event_type;

    let mut payload = serde_json::json!({});
    if let Some(f) = file {
        payload["file"] = serde_json::json!(f);
    }
    if let Some(c) = command {
        payload["command"] = serde_json::json!(c);
    }
    if let Some(ec) = exit_code {
        payload["exit_code"] = serde_json::json!(ec);
    }
    if let Some(raw_payload) = payload_json {
        let extra: serde_json::Value =
            serde_json::from_str(raw_payload).context("Invalid --payload-json")?;
        let Some(extra_obj) = extra.as_object() else {
            anyhow::bail!("--payload-json must be a JSON object");
        };
        let Some(payload_obj) = payload.as_object_mut() else {
            anyhow::bail!("Internal payload error");
        };
        for (key, value) in extra_obj {
            payload_obj.insert(key.clone(), value.clone());
        }
    }

    let mut writer = EventWriter::new(&storage.traces_dir);
    writer.open()?;
    let event = writer.write_event(session, normalized_type, "agent", payload, None)?;
    writer.close();

    // Index the event
    let index = TraceIndex::open(&storage.index_path)?;
    index.index_event(&event)?;

    println!("Event recorded: {} ({})", event.id, normalized_type);
    Ok(())
}

fn cmd_gc(dry_run: bool) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }
    println!(
        "Garbage collection{}",
        if dry_run { " (dry run)" } else { "" }
    );

    // Retention window from config (default 90 days).
    let keep_days = read_retention_days(&storage).unwrap_or(90);
    let cutoff = chrono::Utc::now() - chrono::Duration::days(keep_days as i64);
    println!(
        "  Keeping events newer than {} ({} days)",
        cutoff.to_rfc3339(),
        keep_days
    );

    // Rewrite each JSONL log, dropping events older than the cutoff.
    let mut removed = 0u64;
    let mut kept = 0u64;
    let log_files = std::fs::read_dir(&storage.traces_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "jsonl"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    for path in &log_files {
        let content = std::fs::read_to_string(path)?;
        let mut surviving = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let keep = serde_json::from_str::<tellur_core::schema::types::TraceEvent>(line)
                .ok()
                .and_then(|e| chrono::DateTime::parse_from_rfc3339(&e.timestamp).ok())
                .map(|ts| ts.with_timezone(&chrono::Utc) >= cutoff)
                // If we cannot parse the timestamp, keep the line (safe default).
                .unwrap_or(true);
            if keep {
                surviving.push(line.to_string());
                kept += 1;
            } else {
                removed += 1;
            }
        }
        if !dry_run && removed > 0 {
            std::fs::write(
                path,
                surviving.join("\n") + if surviving.is_empty() { "" } else { "\n" },
            )?;
        }
    }

    println!(
        "  {} event(s) kept, {} event(s) {}",
        kept,
        removed,
        if dry_run {
            "would be removed"
        } else {
            "removed"
        }
    );

    if !dry_run && removed > 0 {
        // Rebuild the index from the surviving logs so it stays consistent.
        rebuild_index(&storage)?;
        println!("  Index rebuilt from surviving events.");
    }

    Ok(())
}

/// Read `retention.keep_days` from `.tellur/config.yml`.
fn read_retention_days(storage: &RepoStorage) -> Option<u32> {
    let content = std::fs::read_to_string(&storage.config_path).ok()?;
    let value: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;
    value
        .get("retention")
        .and_then(|r| r.get("keep_days"))
        .and_then(|d| d.as_u64())
        .map(|d| d as u32)
}

/// Rebuild the SQLite index from the JSONL logs (events table only).
fn rebuild_index(storage: &RepoStorage) -> Result<()> {
    // Start a fresh database file.
    if storage.index_path.exists() {
        std::fs::remove_file(&storage.index_path)?;
    }
    let index = TraceIndex::open(&storage.index_path)?;
    let events = tellur_core::storage::read_events(&storage.traces_dir)?;
    for event in &events {
        index.index_event(event)?;
    }
    Ok(())
}

fn cmd_verify() -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let events = tellur_core::storage::read_events(&storage.traces_dir)?;
    if events.is_empty() {
        println!("No events to verify.");
        return Ok(());
    }

    println!("Verifying {} events...", events.len());

    let result = tellur_core::storage::event_log::verify_chain(&events);
    for problem in &result.problems {
        println!("✗ {}", problem);
    }

    println!();
    if result.broken == 0 {
        println!("✓ All {} events verified — hash chain intact", events.len());
    } else {
        println!("✗ {} valid, {} broken", result.valid, result.broken);
        std::process::exit(1);
    }

    Ok(())
}

fn cmd_redact() -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let events = tellur_core::storage::read_events(&storage.traces_dir)?;
    if events.is_empty() {
        println!("No events to redact.");
        return Ok(());
    }

    let engine = tellur_core::redaction::RedactionEngine::new(
        tellur_core::redaction::RedactionConfig::default(),
    );

    // Rewrite each log file in place, redacting secrets found in payloads.
    let log_files = std::fs::read_dir(&storage.traces_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "jsonl"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut redacted_events = 0u64;
    for path in &log_files {
        let content = std::fs::read_to_string(path)?;
        let mut out_lines = Vec::new();
        let mut changed = false;
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<tellur_core::schema::types::TraceEvent>(line) {
                Ok(mut event) => {
                    let payload_str = serde_json::to_string(&event.payload)?;
                    let result = engine.scan_and_redact(&payload_str);
                    if result.has_secrets {
                        if let Some(red) = result.redacted_content
                            && let Ok(new_payload) = serde_json::from_str(&red)
                        {
                            event.payload = new_payload;
                        }
                        event.redaction = Some(tellur_core::schema::types::RedactionInfo {
                            applied: true,
                            mode: tellur_core::schema::types::RedactionMode::Automatic,
                            rules_applied: Some(
                                result
                                    .findings
                                    .iter()
                                    .map(|f| f.pattern_name.clone())
                                    .collect(),
                            ),
                        });
                        redacted_events += 1;
                        changed = true;
                    }
                    out_lines.push(serde_json::to_string(&event)?);
                }
                Err(_) => out_lines.push(line.to_string()),
            }
        }
        if changed {
            std::fs::write(path, out_lines.join("\n") + "\n")?;
        }
    }

    if redacted_events == 0 {
        println!("No secrets detected in {} events.", events.len());
    } else {
        // Redaction changes payloads, which necessarily invalidates the original
        // hash chain. Re-seal it so `verify` reflects the post-redaction state,
        // then rebuild the index from the re-sealed logs.
        let resealed = tellur_core::storage::event_log::reseal_chain(&storage.traces_dir)?;
        rebuild_index(&storage)?;
        println!(
            "Redacted secrets in {} of {} events.",
            redacted_events,
            events.len()
        );
        println!(
            "Re-sealed hash chain over {} events; run `tellur verify` to confirm.",
            resealed
        );
    }

    Ok(())
}

fn cmd_sessions(session_id: Option<&str>, json: bool) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let index = TraceIndex::open(&storage.index_path)?;

    if let Some(sid) = session_id {
        let events = index.get_session_events(sid)?;
        if json {
            println!("{}", serde_json::to_string(&events)?);
            return Ok(());
        }
        if events.is_empty() {
            println!("No events found for session {}", sid);
            return Ok(());
        }

        println!("Session: {}", sid);
        println!("Events: {}", events.len());
        println!("─────────────────────────────────");
        for event in &events {
            println!(
                "  {} {} {:?}",
                &event.timestamp[..19.min(event.timestamp.len())],
                event.event_type.as_wire(),
                event.actor,
            );
        }
    } else {
        let sessions = index.list_sessions(100)?;
        if json {
            println!("{}", serde_json::to_string(&sessions)?);
            return Ok(());
        }
        if sessions.is_empty() {
            println!("No sessions recorded yet.");
            return Ok(());
        }
        println!("{} session(s):", sessions.len());
        for s in &sessions {
            println!(
                "  {} — {} ({}) · {} events · {}",
                s.id,
                s.agent_name,
                s.model_name.clone().unwrap_or_else(|| "—".to_string()),
                s.event_count,
                s.status,
            );
        }
    }

    Ok(())
}

// ─── New commands: daemon, mcp, hooks ────────────────────────────────────────

async fn cmd_daemon(host: &str, port: u16) -> Result<()> {
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

fn cmd_mcp() -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        eprintln!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }
    tellur_core::mcp::serve_stdio(&storage.root)
}

fn cmd_notes_export(commit: &str, notes_ref: &str, print: bool) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let index = TraceIndex::open(&storage.index_path)?;
    let attributions = index.list_attributions()?;
    if attributions.is_empty() {
        println!("No attribution data to export.");
        return Ok(());
    }

    let commit_sha = resolve_commit(&storage.root, commit)?;
    let note = tellur_core::notes::render_git_ai_note(
        &attributions,
        &commit_sha,
        env!("CARGO_PKG_VERSION"),
    )?;

    if print {
        print!("{}", note);
        return Ok(());
    }

    write_git_note(&storage.root, notes_ref, &commit_sha, &note)?;
    println!(
        "Exported {} attribution range(s) to {} on {}",
        attributions.len(),
        notes_ref,
        short_sha(&commit_sha)
    );
    println!("Push with: tellur notes push");
    Ok(())
}

fn cmd_notes_show(commit: &str, notes_ref: &str, json: bool) -> Result<()> {
    let storage = RepoStorage::discover()?;
    let commit_sha = resolve_commit(&storage.root, commit)?;
    let note = read_git_note(&storage.root, notes_ref, &commit_sha)?;
    let parsed = tellur_core::notes::parse_git_ai_note(&note)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": parsed.schema_version,
                "base_commit_sha": parsed.base_commit_sha,
                "files": parsed.files.iter().map(|f| &f.path).collect::<Vec<_>>(),
                "session_count": parsed.sessions.len(),
                "human_count": parsed.humans.len(),
            }))?
        );
        return Ok(());
    }

    println!("Git AI authorship note ({})", notes_ref);
    println!("Commit: {}", short_sha(&commit_sha));
    println!("Schema: {}", parsed.schema_version);
    println!("Base: {}", short_sha(&parsed.base_commit_sha));
    println!("Files: {}", parsed.files.len());
    println!("Sessions: {}", parsed.sessions.len());
    println!("Humans: {}", parsed.humans.len());
    for file in parsed.files {
        println!(
            "  {} ({} entr{})",
            file.path,
            file.entries.len(),
            if file.entries.len() == 1 { "y" } else { "ies" }
        );
    }
    Ok(())
}

fn cmd_notes_import(commit: &str, notes_ref: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let commit_sha = resolve_commit(&storage.root, commit)?;
    let note = read_git_note(&storage.root, notes_ref, &commit_sha)?;
    let parsed = tellur_core::notes::parse_git_ai_note(&note)?;
    let index = TraceIndex::open(&storage.index_path)?;

    let mut imported = 0u32;
    for file in &parsed.files {
        let blob_sha = git_output(
            &storage.root,
            &["rev-parse", &format!("{}:{}", commit_sha, file.path)],
        )
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| commit_sha.clone());
        for entry in &file.entries {
            for (start, end) in &entry.ranges {
                let (origin, session_id, agent_id, model_id, reviewer) =
                    if let Some(session_key) = entry.key.split_once("::").map(|(s, _)| s) {
                        let session = parsed.sessions.get(session_key);
                        (
                            tellur_core::schema::types::Origin::Ai,
                            session
                                .map(|s| s.agent_id.id.clone())
                                .unwrap_or_else(|| session_key.to_string()),
                            session
                                .map(|s| s.agent_id.tool.clone())
                                .unwrap_or_else(|| "unknown".to_string()),
                            session.map(|s| s.agent_id.model.clone()),
                            session.and_then(|s| s.human_author.clone()),
                        )
                    } else if let Some(human) = parsed.humans.get(&entry.key) {
                        (
                            tellur_core::schema::types::Origin::Human,
                            entry.key.clone(),
                            "human".to_string(),
                            None,
                            Some(human.author.clone()),
                        )
                    } else {
                        (
                            tellur_core::schema::types::Origin::Ai,
                            entry.key.clone(),
                            "unknown".to_string(),
                            None,
                            None,
                        )
                    };

                let range = tellur_core::schema::types::AttributionRange {
                    range_id: format!(
                        "gitai_{}_{}_{}_{}_{}",
                        short_sha(&commit_sha),
                        sanitize_id(&file.path),
                        sanitize_id(&entry.key),
                        start,
                        end
                    ),
                    start_line: *start,
                    end_line: *end,
                    origin,
                    evidence_strength: tellur_core::schema::types::EvidenceStrength::Imported,
                    confidence: 1.0,
                    state: tellur_core::schema::types::AttributionState::Exact,
                    session_id,
                    event_ids: vec![],
                    agent_id,
                    model_id,
                    prompt_hash: None,
                    context_set_id: None,
                    policy_tags: vec![],
                    risk_tags: vec![],
                    risk_level: None,
                    tests_run: vec![],
                    tests_passed: false,
                    reviewer,
                    reviewed_at: None,
                };
                index.index_attribution(
                    &range,
                    &file.path,
                    &blob_sha,
                    &chrono::Utc::now().to_rfc3339(),
                )?;
                imported += 1;
            }
        }
    }

    println!(
        "Imported {} attribution range(s) from {} on {}",
        imported,
        notes_ref,
        short_sha(&commit_sha)
    );
    Ok(())
}

fn cmd_notes_fetch(remote: &str, notes_ref: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    run_git(
        &storage.root,
        &["fetch", remote, &format!("{}:{}", notes_ref, notes_ref)],
    )?;
    println!("Fetched {} from {}", notes_ref, remote);
    Ok(())
}

fn cmd_notes_push(remote: &str, notes_ref: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    run_git(&storage.root, &["push", remote, notes_ref])?;
    println!("Pushed {} to {}", notes_ref, remote);
    Ok(())
}

fn cmd_notes_install_config(remote: &str, notes_ref: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    run_git(
        &storage.root,
        &[
            "config",
            "--add",
            &format!("remote.{}.fetch", remote),
            &format!("+{}:{}", notes_ref, notes_ref),
        ],
    )?;
    run_git(
        &storage.root,
        &["config", "--add", "notes.rewriteRef", notes_ref],
    )?;
    run_git(
        &storage.root,
        &["config", "notes.rewriteMode", "concatenate"],
    )?;
    println!(
        "Configured {} fetch and rewrite support for {}",
        remote, notes_ref
    );
    Ok(())
}

fn cmd_team_report(base: &str, head: &str, notes_ref: &str, json: bool) -> Result<()> {
    let storage = RepoStorage::discover()?;
    let range = format!("{base}..{head}");
    let revs = git_output(&storage.root, &["rev-list", &range])
        .with_context(|| format!("failed to list commits in range {range}"))?;
    let commits: Vec<tellur_core::report::TeamCommitNote> = revs
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|sha| tellur_core::report::TeamCommitNote {
            note: read_git_note(&storage.root, notes_ref, sha).ok(),
            sha: sha.to_string(),
        })
        .collect();

    let report = tellur_core::report::aggregate_team_report(base, head, &commits);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", tellur_core::report::team_report::to_markdown(&report));
    }
    Ok(())
}

fn resolve_commit(repo_root: &std::path::Path, commit: &str) -> Result<String> {
    let output = git_output(repo_root, &["rev-parse", commit])?;
    Ok(output.trim().to_string())
}

fn write_git_note(
    repo_root: &std::path::Path,
    notes_ref: &str,
    commit: &str,
    note: &str,
) -> Result<()> {
    let path = std::env::temp_dir().join(format!(
        "tellur-note-{}-{}.txt",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::write(&path, note)?;
    let result = run_git(
        repo_root,
        &[
            "notes",
            "--ref",
            notes_ref,
            "add",
            "-f",
            "-F",
            &path.to_string_lossy(),
            commit,
        ],
    );
    let _ = std::fs::remove_file(path);
    result
}

fn read_git_note(repo_root: &std::path::Path, notes_ref: &str, commit: &str) -> Result<String> {
    git_output(repo_root, &["notes", "--ref", notes_ref, "show", commit])
}

fn run_git(repo_root: &std::path::Path, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn git_output(repo_root: &std::path::Path, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(8).collect()
}

fn sanitize_id(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

const TELLUR_CODEX_HOOK_SOURCE: &str = "codex";
const TELLUR_CLAUDE_HOOK_SOURCE: &str = "claude-code";
const TELLUR_CURSOR_HOOK_SOURCE: &str = "cursor";
const TELLUR_VSCODE_HOOK_SOURCE: &str = "vscode";
const TELLUR_WINDSURF_HOOK_SOURCE: &str = "windsurf";
const TELLUR_GEMINI_HOOK_SOURCE: &str = "gemini-cli";
const TELLUR_ANTIGRAVITY_HOOK_SOURCE: &str = "antigravity";

fn home_dir_override(home: Option<&Path>) -> Result<PathBuf> {
    if let Some(home) = home {
        return Ok(home.to_path_buf());
    }
    std::env::var("HOME")
        .map(PathBuf::from)
        .context("HOME is not set; pass --home explicitly")
}

fn cmd_setup_agents(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let tellur_exe = tellur_executable_path()?;
    let codex_command = tellur_hook_command(TELLUR_CODEX_HOOK_SOURCE)?;
    let claude_command = tellur_hook_command(TELLUR_CLAUDE_HOOK_SOURCE)?;
    install_claude_global_hooks(&home, &claude_command)?;
    install_codex_global_hooks(&home, &codex_command)?;
    install_codex_personal_plugin(&home, &codex_command)?;
    install_cursor_integration(&home, &tellur_exe)?;
    install_vscode_integration(&home, &tellur_exe)?;
    install_windsurf_integration(&home, &tellur_exe)?;
    install_gemini_cli_integration(&home)?;
    install_antigravity_integration(&home, &tellur_exe)?;
    println!(
        "✓ Installed Tellur global integrations for Claude Code, Codex, Cursor, VS Code, Windsurf, Gemini CLI, and Antigravity"
    );
    println!(
        "  Claude Code hooks: {}",
        home.join(".claude/settings.json").display()
    );
    println!(
        "  Codex hooks: {}",
        home.join(".codex/hooks.json").display()
    );
    println!(
        "  Codex plugin marketplace: {}",
        home.join(".agents/plugins/marketplace.json").display()
    );
    println!(
        "  Cursor MCP/settings: {}",
        cursor_mcp_path(&home).display()
    );
    println!(
        "  VS Code settings: {}",
        vscode_user_settings_path(&home).display()
    );
    println!(
        "  Windsurf MCP/settings: {}",
        windsurf_mcp_path(&home).display()
    );
    println!(
        "  Gemini CLI settings: {}",
        gemini_settings_path(&home).display()
    );
    println!(
        "  Antigravity hooks: {}",
        antigravity_hooks_path(&home).display()
    );
    println!("  Restart Codex/Claude Code and review/trust hooks once when prompted.");
    Ok(())
}

fn cmd_setup_codex(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let codex_command = tellur_hook_command(TELLUR_CODEX_HOOK_SOURCE)?;
    install_codex_global_hooks(&home, &codex_command)?;
    install_codex_personal_plugin(&home, &codex_command)?;
    println!("✓ Installed Tellur global Codex integration");
    println!("  Hooks: {}", home.join(".codex/hooks.json").display());
    println!(
        "  Plugin marketplace: {}",
        home.join(".agents/plugins/marketplace.json").display()
    );
    Ok(())
}

fn cmd_setup_claude_code(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let claude_command = tellur_hook_command(TELLUR_CLAUDE_HOOK_SOURCE)?;
    install_claude_global_hooks(&home, &claude_command)?;
    println!("✓ Installed Tellur global Claude Code integration");
    println!("  Hooks: {}", home.join(".claude/settings.json").display());
    Ok(())
}

fn cmd_setup_cursor(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let tellur_exe = tellur_executable_path()?;
    install_cursor_integration(&home, &tellur_exe)?;
    println!("✓ Installed Tellur global Cursor integration");
    println!("  MCP: {}", cursor_mcp_path(&home).display());
    println!("  Settings: {}", cursor_user_settings_path(&home).display());
    Ok(())
}

fn cmd_setup_vscode(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let tellur_exe = tellur_executable_path()?;
    install_vscode_integration(&home, &tellur_exe)?;
    println!("✓ Installed Tellur global VS Code integration");
    println!("  Settings: {}", vscode_user_settings_path(&home).display());
    Ok(())
}

fn cmd_setup_windsurf(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let tellur_exe = tellur_executable_path()?;
    install_windsurf_integration(&home, &tellur_exe)?;
    println!("✓ Installed Tellur global Windsurf integration");
    println!("  MCP: {}", windsurf_mcp_path(&home).display());
    println!(
        "  Settings: {}",
        windsurf_user_settings_path(&home).display()
    );
    Ok(())
}

fn cmd_setup_gemini_cli(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    install_gemini_cli_integration(&home)?;
    println!("✓ Installed Tellur global Gemini CLI integration");
    println!("  Settings: {}", gemini_settings_path(&home).display());
    Ok(())
}

fn cmd_setup_antigravity(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let tellur_exe = tellur_executable_path()?;
    install_antigravity_integration(&home, &tellur_exe)?;
    println!("✓ Installed Tellur global Antigravity integration");
    println!("  Hooks: {}", antigravity_hooks_path(&home).display());
    println!(
        "  MCP: {}, {}",
        antigravity_mcp_path(&home).display(),
        antigravity_cli_mcp_path(&home).display()
    );
    Ok(())
}

fn cmd_setup_status(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    let claude = hook_config_has_tellur_source(
        &home.join(".claude/settings.json"),
        TELLUR_CLAUDE_HOOK_SOURCE,
    );
    let codex =
        hook_config_has_tellur_source(&home.join(".codex/hooks.json"), TELLUR_CODEX_HOOK_SOURCE);
    let plugin = codex_plugin_status(&home);
    let cursor = cursor_integration_status(&home);
    let vscode = vscode_integration_status(&home);
    let windsurf = windsurf_integration_status(&home);
    let gemini = gemini_integration_status(&home);
    let antigravity = antigravity_integration_status(&home);
    println!(
        "Claude Code global hooks: {}",
        if claude { "installed" } else { "missing" }
    );
    println!(
        "Codex global hooks: {}",
        if codex { "installed" } else { "missing" }
    );
    println!(
        "Codex personal plugin: {}",
        if plugin { "installed" } else { "missing" }
    );
    println!(
        "Cursor global integration: {}",
        if cursor { "installed" } else { "missing" }
    );
    println!(
        "VS Code global integration: {}",
        if vscode { "installed" } else { "missing" }
    );
    println!(
        "Windsurf global integration: {}",
        if windsurf { "installed" } else { "missing" }
    );
    println!(
        "Gemini CLI global integration: {}",
        if gemini { "installed" } else { "missing" }
    );
    println!(
        "Antigravity global integration: {}",
        if antigravity { "installed" } else { "missing" }
    );
    Ok(())
}

fn cmd_setup_uninstall(home: Option<&Path>) -> Result<()> {
    let home = home_dir_override(home)?;
    remove_hook_command_from_json(
        &home.join(".claude/settings.json"),
        TELLUR_CLAUDE_HOOK_SOURCE,
    )?;
    remove_hook_command_from_json(&home.join(".codex/hooks.json"), TELLUR_CODEX_HOOK_SOURCE)?;
    let _ = std::fs::remove_dir_all(home.join(".codex/plugins/tellur-provenance"));
    remove_codex_marketplace_entry(&home)?;
    uninstall_cursor_integration(&home)?;
    uninstall_vscode_integration(&home)?;
    uninstall_windsurf_integration(&home)?;
    uninstall_gemini_cli_integration(&home)?;
    uninstall_antigravity_integration(&home)?;
    println!("✓ Removed Tellur global integrations where present");
    Ok(())
}

fn tellur_executable_path() -> Result<PathBuf> {
    std::env::current_exe().context("failed to resolve tellur executable path")
}

fn tellur_hook_command(source: &str) -> Result<String> {
    let exe = tellur_executable_path()?;
    Ok(format!(
        "{} hooks ingest --source {} --auto-init",
        shell_quote(&exe.to_string_lossy()),
        source
    ))
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn hook_config_has_tellur_source(path: &Path, source: &str) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    value
        .get("hooks")
        .and_then(|hooks| hooks.as_object())
        .is_some_and(|hooks| {
            hooks.values().any(|entries| {
                entries.as_array().is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry
                            .get("hooks")
                            .and_then(|hooks| hooks.as_array())
                            .is_some_and(|handlers| {
                                handlers.iter().any(|handler| {
                                    hook_command_matches_source(handler, source)
                                        && hook_command_executable_exists(handler)
                                })
                            })
                    })
                })
            })
        })
}

fn hook_command_matches_source(handler: &serde_json::Value, source: &str) -> bool {
    handler
        .get("command")
        .and_then(|command| command.as_str())
        .is_some_and(|command| {
            command.contains("hooks ingest")
                && command.contains("--auto-init")
                && command.contains(&format!("--source {}", source))
        })
}

fn hook_command_executable_exists(handler: &serde_json::Value) -> bool {
    let Some(command) = handler.get("command").and_then(|command| command.as_str()) else {
        return false;
    };
    command_executable_path(command).is_some_and(|path| path.exists())
}

fn command_executable_path(command: &str) -> Option<PathBuf> {
    let command = command.trim_start();
    if let Some(rest) = command.strip_prefix('\'') {
        let mut parsed = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\'' {
                break;
            }
            if ch == '\\' && chars.peek() == Some(&'\'') {
                let _ = chars.next();
                parsed.push('\'');
            } else {
                parsed.push(ch);
            }
        }
        return Some(PathBuf::from(parsed));
    }
    command
        .split_whitespace()
        .next()
        .filter(|part| part.starts_with('/'))
        .map(PathBuf::from)
}

fn codex_plugin_status(home: &Path) -> bool {
    let plugin_manifest = home.join(".codex/plugins/tellur-provenance/.codex-plugin/plugin.json");
    let hooks = home.join(".codex/plugins/tellur-provenance/hooks/hooks.json");
    let marketplace = home.join(".agents/plugins/marketplace.json");
    plugin_manifest.exists()
        && hooks.exists()
        && marketplace_plugin_path(&marketplace)
            .as_deref()
            .is_some_and(|path| path == "./.codex/plugins/tellur-provenance")
        && codex_config_plugin_enabled(home)
}

fn codex_config_path(home: &Path) -> PathBuf {
    home.join(".codex/config.toml")
}

fn codex_config_plugin_enabled(home: &Path) -> bool {
    std::fs::read_to_string(codex_config_path(home)).is_ok_and(|content| {
        content
            .lines()
            .position(|line| line.trim() == r#"[plugins."tellur-provenance@tellur-local"]"#)
            .is_some_and(|idx| {
                content
                    .lines()
                    .skip(idx + 1)
                    .take_while(|line| !line.trim_start().starts_with('['))
                    .any(|line| line.trim() == "enabled = true")
            })
    })
}

fn marketplace_plugin_path(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&content).ok()?;
    value
        .get("plugins")?
        .as_array()?
        .iter()
        .find(|plugin| {
            plugin.get("name").and_then(|name| name.as_str()) == Some("tellur-provenance")
        })
        .and_then(|plugin| plugin.get("source"))
        .and_then(|source| source.get("path"))
        .and_then(|path| path.as_str())
        .map(ToString::to_string)
}

fn cursor_mcp_path(home: &Path) -> PathBuf {
    home.join(".cursor/mcp.json")
}

fn cursor_user_settings_path(home: &Path) -> PathBuf {
    editor_user_settings_path(home, "Cursor")
}

fn vscode_user_settings_path(home: &Path) -> PathBuf {
    editor_user_settings_path(home, "Code")
}

fn windsurf_user_settings_path(home: &Path) -> PathBuf {
    editor_user_settings_path(home, "Windsurf")
}

fn windsurf_mcp_path(home: &Path) -> PathBuf {
    home.join(".codeium/windsurf/mcp_config.json")
}

fn gemini_settings_path(home: &Path) -> PathBuf {
    home.join(".gemini/settings.json")
}

fn antigravity_hooks_path(home: &Path) -> PathBuf {
    home.join(".gemini/config/hooks.json")
}

fn antigravity_mcp_path(home: &Path) -> PathBuf {
    home.join(".gemini/antigravity/mcp_config.json")
}

fn antigravity_cli_mcp_path(home: &Path) -> PathBuf {
    home.join(".gemini/antigravity-cli/mcp_config.json")
}

fn editor_user_settings_path(home: &Path, app_name: &str) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library")
            .join("Application Support")
            .join(app_name)
            .join("User/settings.json")
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join("AppData/Roaming"))
            .join(app_name)
            .join("User/settings.json")
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        let config_dir = match app_name {
            "Code" => "Code",
            "Cursor" => "Cursor",
            other => other,
        };
        home.join(".config")
            .join(config_dir)
            .join("User/settings.json")
    }
}

fn install_cursor_integration(home: &Path, tellur_exe: &Path) -> Result<()> {
    install_editor_settings(
        &cursor_user_settings_path(home),
        tellur_exe,
        TELLUR_CURSOR_HOOK_SOURCE,
        "Cursor",
    )?;
    install_cursor_mcp(home, tellur_exe)?;
    Ok(())
}

fn install_vscode_integration(home: &Path, tellur_exe: &Path) -> Result<()> {
    install_editor_settings(
        &vscode_user_settings_path(home),
        tellur_exe,
        TELLUR_VSCODE_HOOK_SOURCE,
        "VS Code AI",
    )
}

fn install_editor_settings(
    path: &Path,
    tellur_exe: &Path,
    agent_id: &str,
    agent_name: &str,
) -> Result<()> {
    let mut settings = read_json_object_or_empty(path)?;
    settings.insert(
        "tellur.tellurPath".to_string(),
        serde_json::Value::String(tellur_exe.to_string_lossy().to_string()),
    );
    settings.insert("tellur.autoInit".to_string(), serde_json::json!(true));
    settings.insert("tellur.autoWatch".to_string(), serde_json::json!(true));
    settings.insert("tellur.captureOnSave".to_string(), serde_json::json!(true));
    settings.insert(
        "tellur.vscodeAgentId".to_string(),
        serde_json::Value::String(agent_id.to_string()),
    );
    settings.insert(
        "tellur.vscodeAgentName".to_string(),
        serde_json::Value::String(agent_name.to_string()),
    );
    write_json_object(path, settings)
}

fn install_cursor_mcp(home: &Path, tellur_exe: &Path) -> Result<()> {
    install_tellur_mcp_server(&cursor_mcp_path(home), tellur_exe)
}

/// Write a `tellur mcp` server entry into an `mcpServers` JSON config, preserving
/// any other servers already configured. Shared by Cursor and Windsurf, which
/// both use the standard `mcpServers` config shape.
fn install_tellur_mcp_server(path: &Path, tellur_exe: &Path) -> Result<()> {
    let mut config = read_json_object_or_empty(path)?;
    let servers = config
        .entry("mcpServers".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !servers.is_object() {
        *servers = serde_json::json!({});
    }
    servers.as_object_mut().unwrap().insert(
        "tellur".to_string(),
        serde_json::json!({
            "command": tellur_exe.to_string_lossy(),
            "args": ["mcp"]
        }),
    );
    write_json_object(path, config)
}

fn read_json_object_or_empty(path: &Path) -> Result<serde_json::Map<String, serde_json::Value>> {
    if !path.exists() {
        return Ok(serde_json::Map::new());
    }
    let content = std::fs::read_to_string(path)?;
    if content.trim().is_empty() {
        return Ok(serde_json::Map::new());
    }
    let value = serde_json::from_str::<serde_json::Value>(&content)
        .with_context(|| format!("invalid JSON in {}; refusing to overwrite", path.display()))?;
    value
        .as_object()
        .cloned()
        .with_context(|| format!("{} must contain a JSON object", path.display()))
}

fn write_json_object(
    path: &Path,
    object: serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(&serde_json::Value::Object(object))?,
    )?;
    Ok(())
}

fn cursor_integration_status(home: &Path) -> bool {
    editor_settings_status(&cursor_user_settings_path(home), TELLUR_CURSOR_HOOK_SOURCE)
        && cursor_mcp_status(home)
}

fn vscode_integration_status(home: &Path) -> bool {
    editor_settings_status(&vscode_user_settings_path(home), TELLUR_VSCODE_HOOK_SOURCE)
}

fn editor_settings_status(path: &Path, agent_id: &str) -> bool {
    let Ok(settings) = read_json_object_or_empty(path) else {
        return false;
    };
    let Some(tellur_path) = settings.get("tellur.tellurPath").and_then(|v| v.as_str()) else {
        return false;
    };
    Path::new(tellur_path).exists()
        && settings
            .get("tellur.autoInit")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        && settings
            .get("tellur.captureOnSave")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        && settings
            .get("tellur.vscodeAgentId")
            .and_then(|v| v.as_str())
            == Some(agent_id)
}

fn cursor_mcp_status(home: &Path) -> bool {
    tellur_mcp_server_status(&cursor_mcp_path(home))
}

fn tellur_mcp_server_status(path: &Path) -> bool {
    let Ok(config) = read_json_object_or_empty(path) else {
        return false;
    };
    let Some(server) = config
        .get("mcpServers")
        .and_then(|v| v.get("tellur"))
        .and_then(|v| v.as_object())
    else {
        return false;
    };
    let Some(command) = server.get("command").and_then(|v| v.as_str()) else {
        return false;
    };
    Path::new(command).exists()
        && server
            .get("args")
            .and_then(|v| v.as_array())
            .is_some_and(|args| args.iter().any(|arg| arg.as_str() == Some("mcp")))
}

fn uninstall_cursor_integration(home: &Path) -> Result<()> {
    remove_editor_settings(&cursor_user_settings_path(home))?;
    remove_cursor_mcp(home)
}

fn uninstall_vscode_integration(home: &Path) -> Result<()> {
    remove_editor_settings(&vscode_user_settings_path(home))
}

fn remove_editor_settings(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut settings = read_json_object_or_empty(path)?;
    for key in [
        "tellur.tellurPath",
        "tellur.autoInit",
        "tellur.autoWatch",
        "tellur.captureOnSave",
        "tellur.vscodeAgentId",
        "tellur.vscodeAgentName",
        "tellur.vscodeModelId",
        "tellur.vscodePromptSessionId",
    ] {
        settings.remove(key);
    }
    write_json_object(path, settings)
}

fn remove_cursor_mcp(home: &Path) -> Result<()> {
    remove_tellur_mcp_server(&cursor_mcp_path(home))
}

fn remove_tellur_mcp_server(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut config = read_json_object_or_empty(path)?;
    if let Some(servers) = config.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        servers.remove("tellur");
    }
    write_json_object(path, config)
}

fn install_windsurf_integration(home: &Path, tellur_exe: &Path) -> Result<()> {
    install_editor_settings(
        &windsurf_user_settings_path(home),
        tellur_exe,
        TELLUR_WINDSURF_HOOK_SOURCE,
        "Windsurf / Cascade",
    )?;
    install_tellur_mcp_server(&windsurf_mcp_path(home), tellur_exe)?;
    Ok(())
}

fn windsurf_integration_status(home: &Path) -> bool {
    editor_settings_status(
        &windsurf_user_settings_path(home),
        TELLUR_WINDSURF_HOOK_SOURCE,
    ) && tellur_mcp_server_status(&windsurf_mcp_path(home))
}

fn uninstall_windsurf_integration(home: &Path) -> Result<()> {
    remove_editor_settings(&windsurf_user_settings_path(home))?;
    remove_tellur_mcp_server(&windsurf_mcp_path(home))
}

fn install_gemini_cli_integration(home: &Path) -> Result<()> {
    let command = tellur_hook_command_with_json_response(TELLUR_GEMINI_HOOK_SOURCE)?;
    install_gemini_hooks_json(&gemini_settings_path(home), &command)
}

fn tellur_hook_command_with_json_response(source: &str) -> Result<String> {
    let exe = tellur_executable_path()?;
    Ok(format!(
        "{} hooks ingest --source {} --auto-init --json-response",
        shell_quote(&exe.to_string_lossy()),
        source
    ))
}

fn install_gemini_hooks_json(path: &Path, command: &str) -> Result<()> {
    let mut settings = read_json_object_or_empty(path)?;
    let hooks = settings
        .entry("hooks".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !hooks.is_object() {
        *hooks = serde_json::json!({});
    }
    let hooks = hooks.as_object_mut().unwrap();
    for (event, matcher) in [
        ("SessionStart", "startup|resume"),
        ("BeforeAgent", "*"),
        (
            "BeforeTool",
            "write_file|replace|edit|run_command|run_shell_command|shell",
        ),
        (
            "AfterTool",
            "write_file|replace|edit|run_command|run_shell_command|shell",
        ),
        ("SessionEnd", "exit|shutdown"),
    ] {
        merge_named_setup_hook(
            hooks,
            event,
            matcher,
            "tellur-provenance",
            command,
            TELLUR_GEMINI_HOOK_SOURCE,
        );
    }
    let hooks_config = settings
        .entry("hooksConfig".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !hooks_config.is_object() {
        *hooks_config = serde_json::json!({});
    }
    hooks_config
        .as_object_mut()
        .unwrap()
        .insert("enabled".to_string(), serde_json::Value::Bool(true));
    write_json_object(path, settings)
}

fn merge_named_setup_hook(
    hooks: &mut serde_json::Map<String, serde_json::Value>,
    event: &str,
    matcher: &str,
    name: &str,
    command: &str,
    source: &str,
) {
    let arr = hooks
        .entry(event.to_string())
        .or_insert_with(|| serde_json::json!([]));
    if !arr.is_array() {
        *arr = serde_json::json!([]);
    }
    if let Some(entries) = arr.as_array_mut() {
        for entry in entries {
            if let Some(handlers) = entry
                .get_mut("hooks")
                .and_then(|hooks| hooks.as_array_mut())
            {
                for handler in handlers {
                    let name_matches =
                        handler.get("name").and_then(|value| value.as_str()) == Some(name);
                    if name_matches || hook_command_matches_source(handler, source) {
                        *handler = serde_json::json!({
                            "name": name,
                            "type": "command",
                            "command": command,
                            "timeout": 30
                        });
                        return;
                    }
                }
            }
        }
    }
    arr.as_array_mut().unwrap().push(serde_json::json!({
        "matcher": matcher,
        "hooks": [
            {
                "name": name,
                "type": "command",
                "command": command,
                "timeout": 30
            }
        ]
    }));
}

fn install_antigravity_integration(home: &Path, tellur_exe: &Path) -> Result<()> {
    let command = tellur_hook_command_with_json_response(TELLUR_ANTIGRAVITY_HOOK_SOURCE)?;
    install_antigravity_hooks_json(&antigravity_hooks_path(home), &command)?;
    install_antigravity_mcp(&antigravity_mcp_path(home), tellur_exe)?;
    install_antigravity_mcp(&antigravity_cli_mcp_path(home), tellur_exe)?;
    Ok(())
}

fn install_antigravity_hooks_json(path: &Path, command: &str) -> Result<()> {
    let mut root = read_json_object_or_empty(path)?;
    let hook = root
        .entry("tellur-provenance".to_string())
        .or_insert_with(|| serde_json::json!({ "enabled": true }));
    if !hook.is_object() {
        *hook = serde_json::json!({ "enabled": true });
    }
    let hook = hook.as_object_mut().unwrap();
    hook.insert("enabled".to_string(), serde_json::Value::Bool(true));
    for (event, matcher) in [
        ("SessionStart", "startup|resume"),
        (
            "PreToolUse",
            "write_file|replace|edit|run_command|run_shell_command|shell",
        ),
        (
            "PostToolUse",
            "write_file|replace|edit|run_command|run_shell_command|shell",
        ),
        ("SessionEnd", "exit|shutdown"),
    ] {
        merge_named_setup_hook(
            hook,
            event,
            matcher,
            "tellur-provenance",
            command,
            TELLUR_ANTIGRAVITY_HOOK_SOURCE,
        );
    }
    write_json_object(path, root)
}

fn install_antigravity_mcp(path: &Path, tellur_exe: &Path) -> Result<()> {
    let mut config = read_json_object_or_empty(path)?;
    let servers = config
        .entry("mcpServers".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !servers.is_object() {
        *servers = serde_json::json!({});
    }
    servers.as_object_mut().unwrap().insert(
        "tellur".to_string(),
        serde_json::json!({
            "command": tellur_exe.to_string_lossy(),
            "args": ["mcp"]
        }),
    );
    write_json_object(path, config)
}

fn gemini_integration_status(home: &Path) -> bool {
    hook_config_has_tellur_source(&gemini_settings_path(home), TELLUR_GEMINI_HOOK_SOURCE)
}

fn antigravity_integration_status(home: &Path) -> bool {
    antigravity_hook_status(home)
        && antigravity_mcp_status(&antigravity_mcp_path(home))
        && antigravity_mcp_status(&antigravity_cli_mcp_path(home))
}

fn antigravity_hook_status(home: &Path) -> bool {
    let Ok(root) = read_json_object_or_empty(&antigravity_hooks_path(home)) else {
        return false;
    };
    root.get("tellur-provenance")
        .and_then(|hook| hook.as_object())
        .is_some_and(|hook| {
            hook.values().any(|entries| {
                entries.as_array().is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry
                            .get("hooks")
                            .and_then(|hooks| hooks.as_array())
                            .is_some_and(|handlers| {
                                handlers.iter().any(|handler| {
                                    hook_command_matches_source(
                                        handler,
                                        TELLUR_ANTIGRAVITY_HOOK_SOURCE,
                                    ) && hook_command_executable_exists(handler)
                                })
                            })
                    })
                })
            })
        })
}

fn antigravity_mcp_status(path: &Path) -> bool {
    let Ok(config) = read_json_object_or_empty(path) else {
        return false;
    };
    let Some(server) = config
        .get("mcpServers")
        .and_then(|v| v.get("tellur"))
        .and_then(|v| v.as_object())
    else {
        return false;
    };
    let Some(command) = server.get("command").and_then(|v| v.as_str()) else {
        return false;
    };
    Path::new(command).exists()
        && server
            .get("args")
            .and_then(|v| v.as_array())
            .is_some_and(|args| args.iter().any(|arg| arg.as_str() == Some("mcp")))
}

fn uninstall_gemini_cli_integration(home: &Path) -> Result<()> {
    remove_gemini_hooks(&gemini_settings_path(home), TELLUR_GEMINI_HOOK_SOURCE)
}

fn uninstall_antigravity_integration(home: &Path) -> Result<()> {
    remove_antigravity_hooks(home)?;
    remove_antigravity_mcp(&antigravity_mcp_path(home))?;
    remove_antigravity_mcp(&antigravity_cli_mcp_path(home))
}

fn remove_gemini_hooks(path: &Path, source: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut settings = read_json_object_or_empty(path)?;
    if let Some(hooks) = settings.get_mut("hooks").and_then(|v| v.as_object_mut()) {
        remove_matching_named_hooks(hooks, source);
    }
    write_json_object(path, settings)
}

fn remove_matching_named_hooks(
    hooks: &mut serde_json::Map<String, serde_json::Value>,
    source: &str,
) {
    for entries in hooks.values_mut() {
        if let Some(arr) = entries.as_array_mut() {
            arr.retain(|entry| {
                !entry
                    .get("hooks")
                    .and_then(|hooks| hooks.as_array())
                    .is_some_and(|handlers| {
                        handlers
                            .iter()
                            .any(|handler| hook_command_matches_source(handler, source))
                    })
            });
        }
    }
}

fn remove_antigravity_hooks(home: &Path) -> Result<()> {
    let path = antigravity_hooks_path(home);
    if !path.exists() {
        return Ok(());
    }
    let mut root = read_json_object_or_empty(&path)?;
    root.remove("tellur-provenance");
    write_json_object(&path, root)
}

fn remove_antigravity_mcp(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut config = read_json_object_or_empty(path)?;
    if let Some(servers) = config.get_mut("mcpServers").and_then(|v| v.as_object_mut()) {
        servers.remove("tellur");
    }
    write_json_object(path, config)
}

fn install_claude_global_hooks(home: &Path, command: &str) -> Result<()> {
    let path = home.join(".claude/settings.json");
    install_hooks_json(&path, command, false)
}

fn install_codex_global_hooks(home: &Path, command: &str) -> Result<()> {
    let path = home.join(".codex/hooks.json");
    install_hooks_json(&path, command, true)
}

fn install_hooks_json(path: &Path, command: &str, include_codex_matchers: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut settings = if path.exists() {
        let content = std::fs::read_to_string(path)?;
        serde_json::from_str::<serde_json::Value>(&content)
            .with_context(|| format!("invalid JSON in {}; refusing to overwrite", path.display()))?
    } else {
        serde_json::json!({})
    };
    if !settings
        .get("hooks")
        .map(|hooks| hooks.is_object())
        .unwrap_or(false)
    {
        settings["hooks"] = serde_json::json!({});
    }
    let hooks = settings["hooks"].as_object_mut().unwrap();
    merge_setup_hook(
        hooks,
        "SessionStart",
        Some("startup|resume|clear|compact"),
        command,
    );
    merge_setup_hook(hooks, "UserPromptSubmit", None, command);
    merge_setup_hook(hooks, "Stop", None, command);
    if include_codex_matchers {
        merge_setup_hook(
            hooks,
            "PreToolUse",
            Some("Bash|apply_patch|Edit|Write"),
            command,
        );
        merge_setup_hook(
            hooks,
            "PostToolUse",
            Some("Bash|apply_patch|Edit|Write"),
            command,
        );
    } else {
        merge_setup_hook(
            hooks,
            "PreToolUse",
            Some("Bash|Write|Edit|MultiEdit"),
            command,
        );
        merge_setup_hook(
            hooks,
            "PostToolUse",
            Some("Bash|Write|Edit|MultiEdit"),
            command,
        );
    }
    std::fs::write(path, serde_json::to_string_pretty(&settings)?)?;
    Ok(())
}

fn merge_setup_hook(
    hooks: &mut serde_json::Map<String, serde_json::Value>,
    event: &str,
    matcher: Option<&str>,
    command: &str,
) {
    let arr = hooks
        .entry(event.to_string())
        .or_insert_with(|| serde_json::json!([]));
    if !arr.is_array() {
        *arr = serde_json::json!([]);
    }
    let already = arr.as_array().is_some_and(|items| {
        items.iter().any(|item| {
            item.get("hooks")
                .and_then(|h| h.as_array())
                .is_some_and(|hs| {
                    hs.iter()
                        .any(|h| h.get("command").and_then(|c| c.as_str()) == Some(command))
                })
        })
    });
    if already {
        return;
    }
    let mut entry = serde_json::json!({
        "hooks": [
            {
                "type": "command",
                "command": command,
                "timeout": 30,
                "statusMessage": "Recording Tellur provenance"
            }
        ]
    });
    if let Some(matcher) = matcher {
        entry["matcher"] = serde_json::Value::String(matcher.to_string());
    }
    arr.as_array_mut().unwrap().push(entry);
}

fn remove_hook_command_from_json(path: &Path, source: &str) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(path)?;
    let mut value = serde_json::from_str::<serde_json::Value>(&content)
        .with_context(|| format!("invalid JSON in {}; refusing to overwrite", path.display()))?;
    if let Some(hooks) = value.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for entries in hooks.values_mut() {
            if let Some(arr) = entries.as_array_mut() {
                arr.retain(|entry| {
                    !entry
                        .get("hooks")
                        .and_then(|h| h.as_array())
                        .is_some_and(|hs| hs.iter().any(|h| hook_command_matches_source(h, source)))
                });
            }
        }
    }
    std::fs::write(path, serde_json::to_string_pretty(&value)?)?;
    Ok(())
}

fn install_codex_personal_plugin(home: &Path, command: &str) -> Result<()> {
    let plugin_root = home.join(".codex/plugins/tellur-provenance");
    std::fs::create_dir_all(plugin_root.join(".codex-plugin"))?;
    std::fs::create_dir_all(plugin_root.join("skills/tellur-provenance"))?;
    std::fs::create_dir_all(plugin_root.join("hooks"))?;

    std::fs::write(
        plugin_root.join(".codex-plugin/plugin.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "name": "tellur-provenance",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Tellur AI provenance workflows for Codex",
            "skills": "./skills/"
        }))?,
    )?;
    std::fs::write(
        plugin_root.join("skills/tellur-provenance/SKILL.md"),
        r#"---
name: tellur-provenance
description: Use Tellur to inspect AI provenance, verify event integrity, and generate PR provenance reports.
---

Use the local `tellur` CLI for provenance workflows:

- `tellur status`
- `tellur sessions`
- `tellur verify`
- `tellur pr-report --base main`

Do not store raw prompts. Tellur records prompt hashes and sanitized metadata.
"#,
    )?;
    let hooks = tellur_hooks_json(command, true);
    std::fs::write(
        plugin_root.join("hooks/hooks.json"),
        serde_json::to_string_pretty(&hooks)?,
    )?;

    let marketplace_path = home.join(".agents/plugins/marketplace.json");
    if let Some(parent) = marketplace_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut marketplace = if marketplace_path.exists() {
        let content = std::fs::read_to_string(&marketplace_path)?;
        serde_json::from_str::<serde_json::Value>(&content).with_context(|| {
            format!(
                "invalid JSON in {}; refusing to overwrite",
                marketplace_path.display()
            )
        })?
    } else {
        serde_json::json!({
            "name": "tellur-local",
            "interface": { "displayName": "Tellur Local" },
            "plugins": []
        })
    };
    marketplace["name"] = serde_json::json!("tellur-local");
    marketplace["interface"] = serde_json::json!({ "displayName": "Tellur Local" });
    if !marketplace
        .get("plugins")
        .map(|plugins| plugins.is_array())
        .unwrap_or(false)
    {
        marketplace["plugins"] = serde_json::json!([]);
    }
    let plugins = marketplace["plugins"].as_array_mut().unwrap();
    plugins.retain(|p| p.get("name").and_then(|n| n.as_str()) != Some("tellur-provenance"));
    plugins.push(serde_json::json!({
        "name": "tellur-provenance",
        "source": {
            "source": "local",
            "path": "./.codex/plugins/tellur-provenance"
        },
        "policy": {
            "installation": "AVAILABLE",
            "authentication": "ON_INSTALL"
        },
        "category": "Productivity"
    }));
    std::fs::write(
        marketplace_path,
        serde_json::to_string_pretty(&marketplace)?,
    )?;
    enable_codex_plugin_in_config(home)?;
    Ok(())
}

fn enable_codex_plugin_in_config(home: &Path) -> Result<()> {
    let path = codex_config_path(home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let content = remove_toml_section(&content, r#"[plugins."tellur-provenance@tellur-local"]"#);
    let mut content = content.trim_end().to_string();
    if !content.is_empty() {
        content.push_str("\n\n");
    }
    content.push_str(
        r#"[plugins."tellur-provenance@tellur-local"]
enabled = true
"#,
    );
    std::fs::write(path, content)?;
    Ok(())
}

fn tellur_hooks_json(command: &str, codex: bool) -> serde_json::Value {
    let mut value = serde_json::json!({ "hooks": {} });
    let hooks = value["hooks"].as_object_mut().unwrap();
    merge_setup_hook(
        hooks,
        "SessionStart",
        Some("startup|resume|clear|compact"),
        command,
    );
    merge_setup_hook(hooks, "UserPromptSubmit", None, command);
    merge_setup_hook(hooks, "Stop", None, command);
    if codex {
        merge_setup_hook(
            hooks,
            "PreToolUse",
            Some("Bash|apply_patch|Edit|Write"),
            command,
        );
        merge_setup_hook(
            hooks,
            "PostToolUse",
            Some("Bash|apply_patch|Edit|Write"),
            command,
        );
    } else {
        merge_setup_hook(
            hooks,
            "PreToolUse",
            Some("Bash|Write|Edit|MultiEdit"),
            command,
        );
        merge_setup_hook(
            hooks,
            "PostToolUse",
            Some("Bash|Write|Edit|MultiEdit"),
            command,
        );
    }
    value
}

fn remove_codex_marketplace_entry(home: &Path) -> Result<()> {
    let marketplace_path = home.join(".agents/plugins/marketplace.json");
    if !marketplace_path.exists() {
        disable_codex_plugin_in_config(home)?;
        return Ok(());
    }
    let content = std::fs::read_to_string(&marketplace_path)?;
    let mut marketplace =
        serde_json::from_str::<serde_json::Value>(&content).with_context(|| {
            format!(
                "invalid JSON in {}; refusing to overwrite",
                marketplace_path.display()
            )
        })?;
    if let Some(plugins) = marketplace
        .get_mut("plugins")
        .and_then(|p| p.as_array_mut())
    {
        plugins.retain(|p| p.get("name").and_then(|n| n.as_str()) != Some("tellur-provenance"));
    }
    std::fs::write(
        marketplace_path,
        serde_json::to_string_pretty(&marketplace)?,
    )?;
    disable_codex_plugin_in_config(home)?;
    Ok(())
}

fn disable_codex_plugin_in_config(home: &Path) -> Result<()> {
    let path = codex_config_path(home);
    if !path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(&path)?;
    let content = remove_toml_section(&content, r#"[plugins."tellur-provenance@tellur-local"]"#);
    std::fs::write(path, content.trim_end())?;
    Ok(())
}

fn remove_toml_section(content: &str, section: &str) -> String {
    let mut output = Vec::new();
    let mut skipping = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == section {
            skipping = true;
            continue;
        }
        if skipping && trimmed.starts_with('[') {
            skipping = false;
        }
        if !skipping {
            output.push(line);
        }
    }
    output.join("\n")
}

fn cmd_hooks_install(tool: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }
    match tool {
        "claude-code" | "claude" => {
            tellur_adapters::ClaudeCodeAdapter::install_hooks(&storage.root)?;
            println!(
                "✓ Installed Claude Code hooks into {}/.claude/settings.json",
                storage.root.display()
            );
            println!(
                "  PostToolUse (Write|Edit|MultiEdit) and SessionStart now record provenance."
            );
        }
        other => {
            println!("Unknown tool: {}. Supported: claude-code", other);
        }
    }
    Ok(())
}

/// Handle a Claude Code hook payload delivered on stdin: capture the current
/// working-tree changes and attribute them to the AI session.
fn cmd_hooks_claude() -> Result<()> {
    use std::io::Read;
    let storage = match RepoStorage::discover() {
        Ok(s) if s.is_initialized() => s,
        // Never fail a hook — just no-op if Tellur isn't set up here.
        _ => return Ok(()),
    };

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload = tellur_adapters::claude_code::HookPayload::parse(&input)?;
    let session_id = payload
        .session_id
        .clone()
        .unwrap_or_else(tellur_core::schema::ids::generate_session_id);

    let index = TraceIndex::open(&storage.index_path)?;

    // Ensure the session is recorded with the Claude Code agent.
    let repo_id = tellur_core::schema::ids::hash_content(&storage.root.to_string_lossy());
    let mut session = Session::new(
        repo_id,
        current_actor(),
        AgentInfo {
            id: "claude-code".to_string(),
            name: "Claude Code".to_string(),
            version: None,
        },
    );
    session.id = session_id.clone();
    index.index_session(&session)?;

    // SessionStart just records the session; tool events trigger capture.
    let mut writer = EventWriter::new(&storage.traces_dir);
    writer.open()?;

    if payload.hook_event_name.as_deref() == Some("SessionStart") {
        let event = writer.write_event(
            &session_id,
            "session.start",
            "agent",
            serde_json::json!({"tool": "claude-code"}),
            None,
        )?;
        index.index_event(&event)?;
        writer.close();
        return Ok(());
    }

    let policy = load_policy(&storage);
    let ctx = CaptureContext::recorded_ai(&session_id, "claude-code");
    if let Some(file_path) = payload.file_path() {
        let _ = capture_working_changes_for_paths(
            &storage,
            &mut writer,
            &index,
            policy.as_ref(),
            &ctx,
            &[file_path],
        )?;
    }
    writer.close();
    Ok(())
}

#[derive(Debug, Default)]
struct AgentHookPayload {
    session_id: Option<String>,
    hook_event_name: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    cwd: Option<String>,
    model: Option<String>,
    prompt: Option<String>,
    message: Option<String>,
    raw: serde_json::Value,
}

impl AgentHookPayload {
    fn parse(input: &str) -> Result<Self> {
        let raw = serde_json::from_str::<serde_json::Value>(input).context("invalid hook JSON")?;
        let tool_input = first_object_value(
            &raw,
            &[
                &["tool_input"],
                &["toolInput"],
                &["input"],
                &["tool", "input"],
                &["tool_use", "input"],
                &["toolUse", "input"],
            ],
        )
        .cloned();
        Ok(Self {
            session_id: first_string(
                &raw,
                &[
                    &["session_id"],
                    &["sessionId"],
                    &["session", "id"],
                    &["conversation_id"],
                    &["conversationId"],
                ],
            )
            .map(ToString::to_string),
            hook_event_name: first_string(
                &raw,
                &[
                    &["hook_event_name"],
                    &["hookEventName"],
                    &["event_name"],
                    &["eventName"],
                    &["event"],
                    &["type"],
                ],
            )
            .map(ToString::to_string),
            tool_name: first_string(
                &raw,
                &[
                    &["tool_name"],
                    &["toolName"],
                    &["tool", "name"],
                    &["tool"],
                    &["name"],
                ],
            )
            .map(ToString::to_string),
            tool_input,
            cwd: first_string(&raw, &[&["cwd"], &["working_dir"], &["workingDir"]])
                .map(ToString::to_string),
            model: first_string(&raw, &[&["model"], &["model_id"], &["modelId"]])
                .map(ToString::to_string),
            prompt: first_string(
                &raw,
                &[
                    &["prompt"],
                    &["user_prompt"],
                    &["userPrompt"],
                    &["input", "prompt"],
                    &["message", "content"],
                ],
            )
            .map(ToString::to_string),
            message: first_string(&raw, &[&["message"]]).map(ToString::to_string),
            raw,
        })
    }

    fn event_name(&self) -> Option<String> {
        self.hook_event_name.clone()
    }

    fn file_path(&self) -> Option<String> {
        self.tool_input
            .as_ref()
            .and_then(|v| find_first_string_key(v, &["file_path", "filePath", "path"], 4))
            .or_else(|| {
                first_string(
                    &self.raw,
                    &[
                        &["file_path"],
                        &["filePath"],
                        &["tool", "file_path"],
                        &["tool", "filePath"],
                        &["tool_use", "file_path"],
                        &["toolUse", "filePath"],
                    ],
                )
            })
            .map(ToString::to_string)
    }

    fn command(&self) -> Option<String> {
        self.tool_input
            .as_ref()
            .and_then(|v| find_first_string_key(v, &["command", "cmd"], 3))
            .or_else(|| first_string(&self.raw, &[&["command"], &["cmd"]]))
            .map(ToString::to_string)
    }

    fn prompt_text(&self) -> Option<&str> {
        self.prompt.as_deref().or(self.message.as_deref())
    }
}

fn first_object_value<'a>(
    value: &'a serde_json::Value,
    paths: &[&[&str]],
) -> Option<&'a serde_json::Value> {
    paths
        .iter()
        .filter_map(|path| json_path(value, path))
        .find(|value| value.is_object())
}

fn first_string<'a>(value: &'a serde_json::Value, paths: &[&[&str]]) -> Option<&'a str> {
    paths
        .iter()
        .filter_map(|path| json_path(value, path))
        .find_map(|value| value.as_str())
}

fn json_path<'a>(mut value: &'a serde_json::Value, path: &[&str]) -> Option<&'a serde_json::Value> {
    for key in path {
        value = value.get(*key)?;
    }
    Some(value)
}

fn find_first_string_key<'a>(
    value: &'a serde_json::Value,
    keys: &[&str],
    max_depth: usize,
) -> Option<&'a str> {
    if max_depth == 0 {
        return None;
    }
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(|value| value.as_str()) {
                    return Some(found);
                }
            }
            map.values()
                .find_map(|value| find_first_string_key(value, keys, max_depth - 1))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|value| find_first_string_key(value, keys, max_depth - 1)),
        _ => None,
    }
}

/// Generic hook ingestion entrypoint used by user-level Codex and Claude Code
/// hooks. It is deliberately no-op friendly so global hooks can be installed
/// once and safely run in unrelated directories.
fn cmd_hooks_ingest(source: &str, auto_init: bool, json_response: bool) -> Result<()> {
    use std::io::Read;

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload = match AgentHookPayload::parse(&input) {
        Ok(payload) => payload,
        Err(err) => {
            eprintln!("tellur hook ingest ignored invalid payload: {err:#}");
            if json_response {
                println!("{{}}");
            }
            return Ok(());
        }
    };

    if let Some(cwd) = payload.cwd.as_deref() {
        let _ = std::env::set_current_dir(cwd);
    }

    let storage = match RepoStorage::discover() {
        Ok(storage) => storage,
        Err(_) => {
            if json_response {
                println!("{{}}");
            }
            return Ok(());
        }
    };
    if storage.tellur_dir.join("disable").exists() {
        if json_response {
            println!("{{}}");
        }
        return Ok(());
    }
    if !storage.is_initialized() {
        if auto_init {
            storage.init()?;
        } else {
            if json_response {
                println!("{{}}");
            }
            return Ok(());
        }
    }

    let session_id = payload
        .session_id
        .clone()
        .unwrap_or_else(tellur_core::schema::ids::generate_session_id);
    let source = normalize_hook_source(source);
    let agent_name = match source {
        "codex" => "Codex",
        "claude-code" => "Claude Code",
        "windsurf" => "Windsurf / Cascade",
        "jetbrains" => "JetBrains AI / Junie",
        "devin" => "Devin",
        "continue" => "Continue",
        "cline" => "Cline / Roo Code",
        other => other,
    };

    let index = TraceIndex::open(&storage.index_path)?;
    let repo_id = tellur_core::schema::ids::hash_content(&storage.root.to_string_lossy());
    let mut session = Session::new(
        repo_id,
        current_actor(),
        AgentInfo {
            id: source.to_string(),
            name: agent_name.to_string(),
            version: None,
        },
    );
    session.id = session_id.clone();
    if let Some(model) = payload.model.as_deref() {
        session.model = Some(ModelInfo {
            provider: source.to_string(),
            name: model.to_string(),
            version: None,
        });
    }
    index.index_session(&session)?;

    let mut writer = EventWriter::new(&storage.traces_dir);
    writer.open()?;
    let hook_event_owned = payload
        .event_name()
        .unwrap_or_else(|| "unknown".to_string());
    let hook_event_owned = normalize_hook_event_name(&hook_event_owned).to_string();
    let hook_event = hook_event_owned.as_str();
    match hook_event {
        "SessionStart" => {
            let event = writer.write_event(
                &session_id,
                "session.start",
                "agent",
                serde_json::json!({
                    "tool": source,
                    "hook_event_name": hook_event,
                    "model": payload.model,
                }),
                None,
            )?;
            index.index_event(&event)?;
        }
        "UserPromptSubmit" => {
            let mut event_payload = serde_json::json!({
                "tool": source,
                "hook_event_name": hook_event,
                "model": payload.model,
            });
            if let Some(prompt) = payload.prompt_text() {
                event_payload["prompt_hash"] =
                    serde_json::Value::String(tellur_core::schema::ids::hash_content(prompt));
            }
            let event =
                writer.write_event(&session_id, "user.prompt", "agent", event_payload, None)?;
            index.index_event(&event)?;
        }
        "PreToolUse" => {
            let event = writer.write_event(
                &session_id,
                "tool.pre_call",
                "agent",
                hook_tool_payload(source, hook_event, &payload),
                None,
            )?;
            index.index_event(&event)?;
        }
        "PostToolUse" => {
            let event = writer.write_event(
                &session_id,
                "tool.post_call",
                "agent",
                hook_tool_payload(source, hook_event, &payload),
                None,
            )?;
            index.index_event(&event)?;

            let policy = load_policy(&storage);
            let ctx = CaptureContext::recorded_ai(&session_id, source);
            if let Some(file_path) = payload.file_path() {
                let _ = capture_working_changes_for_paths(
                    &storage,
                    &mut writer,
                    &index,
                    policy.as_ref(),
                    &ctx,
                    &[file_path],
                )?;
            }
        }
        "Stop" | "SessionEnd" => {
            let event = writer.write_event(
                &session_id,
                "session.end",
                "agent",
                serde_json::json!({
                    "tool": source,
                    "hook_event_name": hook_event,
                }),
                None,
            )?;
            index.index_event(&event)?;
        }
        _ => {
            let event = writer.write_event(
                &session_id,
                &format!("{}.hook.{}", source, sanitize_id(hook_event)),
                "agent",
                hook_tool_payload(source, hook_event, &payload),
                None,
            )?;
            index.index_event(&event)?;
        }
    }
    writer.close();
    if json_response {
        println!("{{}}");
    }
    Ok(())
}

fn normalize_hook_source(source: &str) -> &str {
    match source {
        "claude" | "claude-code" => "claude-code",
        "codex" | "codex-cli" => "codex",
        "gemini" | "gemini-cli" => "gemini-cli",
        "antigravity" | "google-antigravity" => "antigravity",
        "windsurf" | "cascade" => "windsurf",
        "jetbrains" | "junie" | "jetbrains-ai" => "jetbrains",
        "devin" => "devin",
        "continue" | "continue-dev" => "continue",
        "cline" | "roo" | "roo-code" => "cline",
        other => other,
    }
}

fn normalize_hook_event_name(event: &str) -> &str {
    match event {
        "BeforeTool" => "PreToolUse",
        "AfterTool" => "PostToolUse",
        "BeforeAgent" | "BeforeModel" => "UserPromptSubmit",
        "AfterAgent" => "SessionEnd",
        other => other,
    }
}

fn hook_tool_payload(
    source: &str,
    hook_event: &str,
    payload: &AgentHookPayload,
) -> serde_json::Value {
    let mut out = serde_json::json!({
        "tool": source,
        "hook_event_name": hook_event,
        "tool_name": payload.tool_name,
        "model": payload.model,
    });
    if let Some(file_path) = payload.file_path() {
        out["file_path"] = serde_json::Value::String(file_path);
    }
    if let Some(command) = payload.command() {
        out["command"] = serde_json::Value::String(redact_hook_string(&command));
    }
    out
}

fn redact_hook_string(value: &str) -> String {
    tellur_core::redaction::RedactionEngine::default_engine()
        .scan_and_redact(value)
        .redacted_content
        .unwrap_or_else(|| "[REDACTED]".to_string())
}
