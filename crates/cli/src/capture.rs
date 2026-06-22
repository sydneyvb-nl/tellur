//! Activity-capture commands: `watch` (filesystem-driven capture loop) and
//! `event` (emit a single event for the generic adapter / CI).

use anyhow::{Context, Result};

use tellur_core::capture::{CaptureContext, capture_working_changes};
use tellur_core::policy::PolicyEngine;
use tellur_core::schema::types::{AgentInfo, ModelInfo, Session};
use tellur_core::storage::{EventWriter, RepoStorage, TraceIndex};

use crate::util::{current_actor, load_policy};

pub(crate) async fn cmd_watch(
    agent_id: &str,
    agent_name: &str,
    model_id: Option<String>,
) -> Result<()> {
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

pub(crate) fn cmd_event(
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
