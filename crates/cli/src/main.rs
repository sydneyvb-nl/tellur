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

use tracegit_core::adapter::builtin::all_adapters;
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
    },

    /// Show AI attribution for a file
    Blame {
        /// File path
        file: String,
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
        Commands::Explain { target } => cmd_explain(&target),
        Commands::Blame { file } => cmd_blame(&file),
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
        Commands::Sessions { session_id } => cmd_sessions(session_id.as_deref()),
    }
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
    let policies: Vec<_> = std::fs::read_dir(&storage.policies_dir)
        .unwrap_or_else(|_| panic!("No policies dir"))
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "yml"))
        .collect();
    println!("✓ {} polic{} found", policies.len(), if policies.len() == 1 { "y" } else { "ies" });
    for p in &policies {
        println!("  - {}", p.path().file_name().unwrap().to_string_lossy());
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
        let trace_files: Vec<_> = std::fs::read_dir(&storage.traces_dir)
            .unwrap_or_else(|_| panic!("No traces dir"))
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |ext| ext == "jsonl"))
            .collect();
        println!("✓ Traces directory ({} log files)", trace_files.len());
    }

    // Detect AI tools
    println!();
    println!("AI Tool Detection:");
    let workspace = &storage.root;
    let adapters = all_adapters();
    let mut detected = 0;
    for adapter in adapters {
        let result = adapter.detect(workspace).await;
        if result.detected {
            detected += 1;
            println!(
                "  ✓ {}{} ({})",
                result.tool_name,
                result.version.map(|v| format!(" v{}", v)).unwrap_or_default(),
                result.config_path.as_deref().unwrap_or("detected")
            );
        }
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

fn cmd_explain(target: &str) -> Result<()> {
    // Parse file:line format
    let (file, line) = if let Some((f, l)) = target.rsplit_once(':') {
        let line_num: u32 = l.parse().context("Invalid line number")?;
        (f, line_num)
    } else {
        println!("Usage: tracegit explain <file>:<line>");
        std::process::exit(1);
    };

    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }

    let index = TraceIndex::open(&storage.index_path)?;
    let attributions = index.get_file_attributions(file)?;

    if attributions.is_empty() {
        println!("No attribution data for {}", file);
        println!("Run `tracegit watch` to start capturing AI activity.");
        return Ok(());
    }

    // Find the range that contains this line
    for (_blob_sha, attr) in &attributions {
        if line >= attr.start_line && line <= attr.end_line {
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
            return Ok(());
        }
    }

    println!("Line {} in {} — no AI attribution recorded", line, file);
    Ok(())
}

fn cmd_blame(file: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }

    let index = TraceIndex::open(&storage.index_path)?;
    let attributions = index.get_file_attributions(file)?;

    if attributions.is_empty() {
        println!("No attribution data for {}", file);
        return Ok(());
    }

    println!("Attribution for {}", file);
    println!("─────────────────────────────────────────────");
    for (_blob_sha, attr) in &attributions {
        println!(
            "  L{:3}-{:<3} {:?} {} conf={:.0}% [{}]",
            attr.start_line,
            attr.end_line,
            attr.origin,
            attr.agent_id,
            attr.confidence * 100.0,
            format!("{:?}", attr.state),
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

    println!("PR Risk Report: {}..{}", base, head);
    println!("══════════════════════════════════════════════");
    println!();
    println!("⚠ PR report generation requires git diff analysis.");
    println!("  This feature is under development.");
    println!();
    println!("  To generate a full report, TraceGit needs:");
    println!("  1. The diff between {} and {}", base, head);
    println!("  2. Attributed ranges from the index");
    println!("  3. Policy evaluation results");

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

    println!("Exporting provenance data (format: {})...", format);
    println!("⚠ Export is under development.");
    Ok(())
}

async fn cmd_import(adapter: &str, source: &std::path::Path) -> Result<()> {
    println!("Importing from {} adapter: {}", adapter, source.display());
    println!("⚠ Adapter imports are under development.");
    Ok(())
}

async fn cmd_watch() -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }

    println!("TraceGit Watch");
    println!("══════════════");
    println!("Watching for AI development activity...");
    println!("Press Ctrl+C to stop.");
    println!();

    // Create a session
    let session_id = tracegit_core::schema::ids::generate_session_id();
    println!("Session: {}", session_id);
    println!();

    // Open event writer
    let mut writer = EventWriter::new(&storage.traces_dir);
    writer.open()?;

    let event = writer.write_event(
        &session_id,
        "session.start",
        "agent",
        serde_json::json!({"mode": "watch", "tool": "tracegit-cli"}),
        None,
    )?;

    println!("Session started. Event: {}", event.id);
    println!();
    println!("⚠ File watching and adapter integration coming soon.");
    println!("  For now, use `tracegit event` to emit events manually.");

    writer.close();
    Ok(())
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
    let event = writer.write_event(session, event_type, "agent", payload, None)?;
    writer.close();

    println!("Event recorded: {} ({})", event.id, event_type);
    Ok(())
}

fn cmd_gc(dry_run: bool) -> Result<()> {
    let storage = RepoStorage::discover()?;
    println!("Garbage collection{}", if dry_run { " (dry run)" } else { "" });
    println!("⚠ GC is under development.");
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

    let mut prev_hash: Option<&str> = None;
    let mut valid = 0;
    let mut broken = 0;

    for event in &events {
        // Verify hash chain
        if let Some(prev) = prev_hash {
            if event.prev_hash.as_deref() != Some(prev) {
                println!("✗ Chain broken at event {}", event.id);
                broken += 1;
            } else {
                valid += 1;
            }
        } else {
            valid += 1;
        }
        prev_hash = event.event_hash.as_deref();
    }

    println!();
    if broken == 0 {
        println!("✓ All {} events verified — hash chain intact", events.len());
    } else {
        println!("✗ {} valid, {} broken", valid, broken);
    }

    Ok(())
}

fn cmd_redact() -> Result<()> {
    println!("⚠ Redaction command is under development.");
    Ok(())
}

fn cmd_sessions(session_id: Option<&str>) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("TraceGit not initialized. Run `tracegit init` first.");
        return Ok(());
    }

    let index = TraceIndex::open(&storage.index_path)?;

    if let Some(sid) = session_id {
        let events = index.get_session_events(sid)?;
        if events.is_empty() {
            println!("No events found for session {}", sid);
            return Ok(());
        }

        println!("Session: {}", sid);
        println!("Events: {}", events.len());
        println!("─────────────────────────────────");
        for event in &events {
            let event_type_str = serde_json::to_string(&event.event_type)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string();
            println!(
                "  {} {} {}",
                &event.timestamp[..19.min(event.timestamp.len())],
                event_type_str,
                format!("{:?}", event.actor),
            );
        }
    } else {
        let count = index.session_count()?;
        println!("{} session(s) recorded", count);
    }

    Ok(())
}
