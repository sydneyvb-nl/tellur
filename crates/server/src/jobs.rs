//! Durable background-job worker.
//!
//! Heavy org exports are enqueued as persistent jobs (survive restarts) and run
//! by a worker that claims `queued` jobs, executes them, and stores the JSON
//! result. [`process_one`] runs a single job and is used both by the background
//! loop and by tests (deterministic, no timing).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::storage::{Job, Store};

/// Job kinds the worker knows how to run.
pub const KIND_EXPORT_EVENTS: &str = "export.events";
pub const KIND_EXPORT_AUDIT: &str = "export.audit";

/// Claim and run a single queued job. Returns `true` if a job was processed,
/// `false` if the queue was empty.
pub fn process_one(store: &Arc<dyn Store>) -> Result<bool> {
    let Some(job) = store.claim_next_job()? else {
        return Ok(false);
    };
    match run_job(store, &job) {
        Ok(value) => {
            let text = serde_json::to_string(&value)?;
            store.complete_job(&job.id, &text)?;
        }
        Err(e) => {
            store.fail_job(&job.id, &e.to_string())?;
        }
    }
    Ok(true)
}

/// Execute a job's work and return its JSON result.
fn run_job(store: &Arc<dyn Store>, job: &Job) -> Result<Value> {
    match job.kind.as_str() {
        KIND_EXPORT_EVENTS => {
            let events = store.export_events(&job.org_id)?;
            Ok(json!({
                "schema": "tellur.server.export.events.v1",
                "org_id": job.org_id,
                "generated_at": chrono::Utc::now().to_rfc3339(),
                "count": events.len(),
                "events": events,
            }))
        }
        KIND_EXPORT_AUDIT => {
            let entries = store.export_audit(&job.org_id)?;
            let chain_intact = store.verify_audit_chain()?;
            Ok(json!({
                "schema": "tellur.server.export.audit.v1",
                "org_id": job.org_id,
                "generated_at": chrono::Utc::now().to_rfc3339(),
                "chain_intact": chain_intact,
                "count": entries.len(),
                "entries": entries,
            }))
        }
        other => bail!("unknown job kind: {other}"),
    }
}

/// Spawn the background worker loop. Drains the queue, then idles between polls.
pub fn spawn_worker(store: Arc<dyn Store>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Reclaim jobs left `running` by a previous process (crash/restart).
        match store.requeue_running_jobs() {
            Ok(n) if n > 0 => tracing::info!(requeued = n, "reclaimed in-flight jobs on startup"),
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "failed to requeue running jobs on startup"),
        }
        loop {
            // Drain all ready jobs, each on a blocking thread.
            loop {
                let store = store.clone();
                match tokio::task::spawn_blocking(move || process_one(&store)).await {
                    Ok(Ok(true)) => continue,
                    Ok(Ok(false)) => break,
                    Ok(Err(e)) => {
                        tracing::error!(error = %e, "job processing failed");
                        break;
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "job worker task panicked");
                        break;
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    })
}
