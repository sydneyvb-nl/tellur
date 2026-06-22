//! Read-only inspection commands: `status`, `explain`, `blame`, `pr-report`,
//! `verify`, and `sessions`.

use anyhow::{Context, Result};

use tellur_core::storage::{RepoStorage, TraceIndex};

pub(crate) fn cmd_status() -> Result<()> {
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

pub(crate) fn cmd_explain(target: &str, json: bool) -> Result<()> {
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

pub(crate) fn cmd_blame(file: &str, json: bool) -> Result<()> {
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

pub(crate) fn cmd_pr_report(base: &str, head: &str) -> Result<()> {
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

pub(crate) fn cmd_verify() -> Result<()> {
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

pub(crate) fn cmd_sessions(session_id: Option<&str>, json: bool) -> Result<()> {
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
