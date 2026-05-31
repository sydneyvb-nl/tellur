//! TraceGit CLI — AI Code Provenance from the terminal
//!
//! Commands:
//!   tracegit init       — Initialize TraceGit in a repository
//!   tracegit doctor     — Check setup and detect AI tools
//!   tracegit status     — Show current status
//!   tracegit explain    — Explain who/what changed a line
//!   tracegit blame      — Show AI attribution for a file
//!   tracegit pr-report  — Generate a PR risk report
//!   tracegit policy     — Check policy compliance
//!   tracegit export     — Export provenance data
//!   tracegit watch      — Start capturing AI development activity
//!   tracegit event      — Emit a single event (generic adapter)
//!   tracegit gc         — Garbage collect expired data
//!   tracegit verify     — Verify provenance integrity

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use tracegit_core::capture::{capture_working_changes, CaptureContext};
use tracegit_core::policy::PolicyEngine;
use tracegit_core::schema::types::{Actor, AgentInfo, EventActor, Session};
use tracegit_core::storage::{EventWriter, RepoStorage, TraceIndex};

#[derive(Parser)]
#[command(name = "tracegit")]
#[command(version, about = "AI Code Provenance — line-level attribution, session replay, PR risk reports")]
#[command(long_about = "TraceGit records, attributes, and reports on AI-assisted development.\n\n\
Git tells you what changed. TraceGit tells you how AI participated.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize TraceGit in the current repository
    Init {
        /// Setup profile: default | team | oss-maintainer
        #[arg(long, default_value = "default")]
        profile: String,
    },

    /// Check TraceGit setup and detect AI tools
    Doctor,

    /// Show current TraceGit status
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
        /// Adapter to import from: claude-code | aider | cursor | generic
        adapter: String,
        /// Source path
        source: PathBuf,
    },

    /// Start watching for AI development activity
    Watch,

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

    /// Manage editor/agent hook integrations
    Hooks {
        #[command(subcommand)]
        action: HookActions,
    },
}

#[derive(Subcommand)]
enum HookActions {
    /// Install TraceGit hooks into Claude Code settings (.claude/settings.json)
    Install {
        /// Which tool's hooks to install
        #[arg(default_value = "claude-code")]
        tool: String,
    },
    /// Internal: handle a Claude Code hook payload from stdin
    #[command(hide = true)]
    Claude,
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
        Commands::Watch => cmd_watch().await,
        Commands::Event { event_type, session, file, command, exit_code } => {
            cmd_event(&event_type, &session, file.as_deref(), command.as_deref(), exit_code)
        }
        Commands::Gc { dry_run } => cmd_gc(dry_run),
        Commands::Verify => cmd_verify(),
        Commands::Redact => cmd_redact(),
        Commands::Sessions { session_id, json } => cmd_sessions(session_id.as_deref(), json),
        Commands::Daemon { host, port } => cmd_daemon(&host, port).await,
        Commands::Mcp => cmd_mcp(),
        Commands::Hooks { action } => match action {
            HookActions::Install { tool } => cmd_hooks_install(&tool),
            HookActions::Claude => cmd_hooks_claude(),
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
        println!("TraceGit already initialized. Run `tracegit doctor` to check setup.");
        return Ok(());
    }

    storage.init()?;
    println!("✓ TraceGit initialized (profile: {})", profile);
    println!("  Config: {}", storage.config_path.display());
    println!("  Policies: {}", storage.policies_dir.display());
    println!("  Traces: {}", storage.traces_dir.display());
    println!();
    println!("Next: run `tracegit doctor` to verify setup");
    Ok(())
}

async fn cmd_doctor() -> Result<()> {
    let storage = RepoStorage::discover()?;

    println!("TraceGit Doctor");
    println!("═══════════════");
    println!();

    // Check config
    if storage.is_initialized() {
        println!("✓ Config found");
    } else {
        println!("✗ Config not found — run `tracegit init` first");
    }

    // Check policies
    let policies: Vec<_> = match std::fs::read_dir(&storage.policies_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "yml"))
            .collect(),
        Err(_) => Vec::new(),
    };
    println!("✓ {} polic{} found", policies.len(), if policies.len() == 1 { "y" } else { "ies" });
    for p in &policies {
        println!("  - {}", p.path().file_name().unwrap_or_default().to_string_lossy());
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
    if std::path::Path::new(&std::env::var("HOME").unwrap_or_default()).join(".claude").exists() {
        detected += 1;
        println!("  ✓ Claude Code (~/.claude found)");
    }

    // Check for Cursor
    if storage.root.join(".cursor").exists() {
        detected += 1;
        println!("  ✓ Cursor (.cursor/ found)");
    }

    // Check for Aider
    if std::process::Command::new("which").arg("aider").output().map(|o| o.status.success()).unwrap_or(false) {
        detected += 1;
        println!("  ✓ Aider (installed)");
    }

    if detected == 0 {
        println!("  No AI coding tools detected");
    }

    println!();
    if storage.is_initialized() {
        println!("Setup looks good. Run `tracegit watch` to start capturing.");
    }

    Ok(())
}

fn cmd_status() -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }

    let index = TraceIndex::open(&storage.index_path)?;
    let events = index.event_count()?;
    let sessions = index.session_count()?;

    println!("Sessions: {}", sessions);
    println!("Events: {}", events);

    if events == 0 {
        println!();
        println!("No events recorded yet. Run `tracegit watch` to start capturing.");
    }

    Ok(())
}

fn cmd_explain(target: &str, json: bool) -> Result<()> {
    // Parse file:line format
    let (file, line) = if let Some((f, l)) = target.rsplit_once(':') {
        let line_num: u32 = l.parse().context("Invalid line number")?;
        (f, line_num)
    } else {
        anyhow::bail!("Usage: tracegit explain <file>:<line>");
    };

    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("TraceGit not initialized. Run `tracegit init` first.");
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
            println!("Run `tracegit watch` (or install hooks) to start capturing AI activity.");
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
        println!("TraceGit not initialized. Run `tracegit init` first.");
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
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }

    let report = tracegit_core::report::build_repo_pr_report(&storage, base, head)?;
    println!("{}", tracegit_core::report::PRReportGenerator::to_markdown(&report));
    Ok(())
}

fn cmd_policy_check() -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }

    let policy_path = storage.policies_dir.join("default.yml");
    if !policy_path.exists() {
        println!("No policy file found.");
        return Ok(());
    }

    let engine = tracegit_core::policy::PolicyEngine::load_from_file(&policy_path)?;
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

    let engine = tracegit_core::policy::PolicyEngine::load_from_file(&policy_path)?;
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
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }

    let events = tracegit_core::storage::read_events(&storage.traces_dir)?;
    if events.is_empty() {
        println!("No events to export.");
        return Ok(());
    }

    let result = match format {
        "json" => serde_json::to_string_pretty(&events)?,
        "jsonl" => events.iter()
            .map(|e| serde_json::to_string(e).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n"),
        "markdown" | "md" => {
            let mut md = String::from("# TraceGit Export\n\n");
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
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }

    println!("Importing from {} adapter: {}", adapter, source.display());

    let events: Vec<tracegit_core::schema::types::TraceEvent> = match adapter {
        "claude-code" | "claude" => {
            let a = tracegit_adapters::ClaudeCodeAdapter::new();
            a.parse_transcript(source, "imported")?
        }
        "aider" => {
            let a = tracegit_adapters::AiderAdapter::new();
            let repo_root = std::env::current_dir()?;
            a.parse_git_log(&repo_root, "2020-01-01")?
        }
        "cursor" => {
            let a = tracegit_adapters::CursorAdapter::new();
            a.parse_trace_file(source, "imported")?
        }
        "generic" => {
            let a = tracegit_adapters::GenericAdapter::new();
            a.import_jsonl(source)?
        }
        _ => {
            println!("Unknown adapter: {}. Supported: claude-code, aider, cursor, generic", adapter);
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
    for e in &events {
        // Re-write through EventWriter for a proper, server-side hash chain.
        let actor = serde_json::to_value(&e.actor)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "agent".to_string());
        let event = writer.write_event(
            &e.session_id,
            &e.event_type.as_wire(),
            &actor,
            e.payload.clone(),
            None,
        )?;
        index.index_event(&event)?;
        count += 1;
    }
    writer.close();

    println!("Imported {} events from {}", count, adapter);
    Ok(())
}

async fn cmd_watch() -> Result<()> {
    use notify::{RecursiveMode, Watcher};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{channel, RecvTimeoutError};
    use std::sync::Arc;
    use std::time::Duration;

    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }

    println!("TraceGit Watch");
    println!("══════════════");
    println!("Watching {} for changes...", storage.root.display());
    println!("Press Ctrl+C to stop.");
    println!();

    // Create and index a watch session.
    let repo_id = tracegit_core::schema::ids::hash_content(&storage.root.to_string_lossy());
    let session = Session::new(
        repo_id,
        current_actor(),
        AgentInfo {
            id: "watch".to_string(),
            name: "TraceGit Watch".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        },
    );
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
        serde_json::json!({"mode": "watch", "tool": "tracegit-cli"}),
        None,
    )?;

    let policy = load_policy(&storage);
    let ctx = CaptureContext::inferred_watch(&session_id);

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
                let relevant = event.paths.iter().any(|p| {
                    tracegit_core::storage::file_watcher::should_track(p, &storage.root)
                });
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
) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("TraceGit not initialized. Run `tracegit init` first.");
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
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }
    println!("Garbage collection{}", if dry_run { " (dry run)" } else { "" });

    // Retention window from config (default 90 days).
    let keep_days = read_retention_days(&storage).unwrap_or(90);
    let cutoff = chrono::Utc::now() - chrono::Duration::days(keep_days as i64);
    println!("  Keeping events newer than {} ({} days)", cutoff.to_rfc3339(), keep_days);

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
            let keep = serde_json::from_str::<tracegit_core::schema::types::TraceEvent>(line)
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
            std::fs::write(path, surviving.join("\n") + if surviving.is_empty() { "" } else { "\n" })?;
        }
    }

    println!("  {} event(s) kept, {} event(s) {}",
        kept, removed, if dry_run { "would be removed" } else { "removed" });

    if !dry_run && removed > 0 {
        // Rebuild the index from the surviving logs so it stays consistent.
        rebuild_index(&storage)?;
        println!("  Index rebuilt from surviving events.");
    }

    Ok(())
}

/// Read `retention.keep_days` from `.tracegit/config.yml`.
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
    let events = tracegit_core::storage::read_events(&storage.traces_dir)?;
    for event in &events {
        index.index_event(event)?;
    }
    Ok(())
}

fn cmd_verify() -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }

    let events = tracegit_core::storage::read_events(&storage.traces_dir)?;
    if events.is_empty() {
        println!("No events to verify.");
        return Ok(());
    }

    println!("Verifying {} events...", events.len());

    let result = tracegit_core::storage::event_log::verify_chain(&events);
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
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }

    let events = tracegit_core::storage::read_events(&storage.traces_dir)?;
    if events.is_empty() {
        println!("No events to redact.");
        return Ok(());
    }

    let engine = tracegit_core::redaction::RedactionEngine::new(
        tracegit_core::redaction::RedactionConfig::default(),
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
            match serde_json::from_str::<tracegit_core::schema::types::TraceEvent>(line) {
                Ok(mut event) => {
                    let payload_str = serde_json::to_string(&event.payload)?;
                    let result = engine.scan_and_redact(&payload_str);
                    if result.has_secrets {
                        if let Some(red) = result.redacted_content
                            && let Ok(new_payload) = serde_json::from_str(&red) {
                                event.payload = new_payload;
                            }
                        event.redaction = Some(tracegit_core::schema::types::RedactionInfo {
                            applied: true,
                            mode: tracegit_core::schema::types::RedactionMode::Automatic,
                            rules_applied: Some(
                                result.findings.iter().map(|f| f.pattern_name.clone()).collect(),
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
        let resealed = tracegit_core::storage::event_log::reseal_chain(&storage.traces_dir)?;
        rebuild_index(&storage)?;
        println!("Redacted secrets in {} of {} events.", redacted_events, events.len());
        println!("Re-sealed hash chain over {} events; run `tracegit verify` to confirm.", resealed);
    }

    Ok(())
}

fn cmd_sessions(session_id: Option<&str>, json: bool) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("TraceGit not initialized. Run `tracegit init` first.");
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
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }
    let config = tracegit_core::daemon::DaemonConfig {
        host: host.to_string(),
        port,
        repo_root: storage.root.clone(),
    };
    tracegit_core::daemon::run_daemon(config).await
}

fn cmd_mcp() -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        eprintln!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }
    tracegit_core::mcp::serve_stdio(&storage.root)
}

fn cmd_hooks_install(tool: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }
    match tool {
        "claude-code" | "claude" => {
            tracegit_adapters::ClaudeCodeAdapter::install_hooks(&storage.root)?;
            println!("✓ Installed Claude Code hooks into {}/.claude/settings.json", storage.root.display());
            println!("  PostToolUse (Write|Edit|MultiEdit) and SessionStart now record provenance.");
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
        // Never fail a hook — just no-op if TraceGit isn't set up here.
        _ => return Ok(()),
    };

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload = tracegit_adapters::claude_code::HookPayload::parse(&input)?;
    let session_id = payload
        .session_id
        .clone()
        .unwrap_or_else(tracegit_core::schema::ids::generate_session_id);

    let index = TraceIndex::open(&storage.index_path)?;

    // Ensure the session is recorded with the Claude Code agent.
    let repo_id = tracegit_core::schema::ids::hash_content(&storage.root.to_string_lossy());
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
        writer.write_event(
            &session_id,
            "session.start",
            "agent",
            serde_json::json!({"tool": "claude-code"}),
            None,
        )?;
        writer.close();
        return Ok(());
    }

    let policy = load_policy(&storage);
    let ctx = CaptureContext::recorded_ai(&session_id, "claude-code");
    let _ = capture_working_changes(&storage, &mut writer, &index, policy.as_ref(), &ctx)?;
    writer.close();
    Ok(())
}
