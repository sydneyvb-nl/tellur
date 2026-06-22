//! Data-maintenance commands: `export`, `import`, `gc`, and `redact`.

use anyhow::Result;

use tellur_core::storage::{EventWriter, RepoStorage, TraceIndex};

use crate::util::{read_retention_days, rebuild_index};

pub(crate) fn cmd_export(format: &str, output: Option<&std::path::Path>) -> Result<()> {
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

pub(crate) async fn cmd_import(adapter: &str, source: &std::path::Path) -> Result<()> {
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

pub(crate) fn cmd_gc(dry_run: bool) -> Result<()> {
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

pub(crate) fn cmd_redact() -> Result<()> {
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
