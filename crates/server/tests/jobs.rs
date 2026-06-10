//! Durable job queue tests (storage + worker semantics).

use std::sync::Arc;

use tellur_server::jobs;
use tellur_server::storage::{SqliteStore, Store};

fn store() -> Arc<dyn Store> {
    let s = Arc::new(SqliteStore::open_in_memory().unwrap());
    s.migrate().unwrap();
    s
}

#[test]
fn enqueue_process_and_complete() {
    let store = store();
    let org = store.create_org("A").unwrap().id;
    let job_id = store
        .enqueue_job(&org, jobs::KIND_EXPORT_EVENTS, None)
        .unwrap();

    // A queued job is visible and pending.
    assert_eq!(
        store.get_job(&org, &job_id).unwrap().unwrap().status,
        "queued"
    );

    // The worker processes exactly one job, then the queue is empty.
    assert!(jobs::process_one(&store).unwrap());
    assert!(!jobs::process_one(&store).unwrap());

    let job = store.get_job(&org, &job_id).unwrap().unwrap();
    assert_eq!(job.status, "completed");
    assert!(job.error.is_none());
}

#[test]
fn unknown_kind_fails_the_job() {
    let store = store();
    let org = store.create_org("A").unwrap().id;
    let job_id = store.enqueue_job(&org, "bogus.kind", None).unwrap();
    assert!(jobs::process_one(&store).unwrap());
    let job = store.get_job(&org, &job_id).unwrap().unwrap();
    assert_eq!(job.status, "failed");
    assert!(job.error.unwrap().contains("unknown job kind"));
}

#[test]
fn jobs_are_tenant_scoped() {
    let store = store();
    let org_a = store.create_org("A").unwrap().id;
    let org_b = store.create_org("B").unwrap().id;
    let job_id = store
        .enqueue_job(&org_a, jobs::KIND_EXPORT_AUDIT, None)
        .unwrap();
    // Another org cannot read the job.
    assert!(store.get_job(&org_b, &job_id).unwrap().is_none());
    assert!(store.get_job(&org_a, &job_id).unwrap().is_some());
}

#[test]
fn running_jobs_are_requeued_on_startup() {
    let store = store();
    let org = store.create_org("A").unwrap().id;
    let job_id = store
        .enqueue_job(&org, jobs::KIND_EXPORT_EVENTS, None)
        .unwrap();
    // Simulate a crash mid-flight: claimed (running) but never completed.
    let claimed = store.claim_next_job().unwrap().unwrap();
    assert_eq!(claimed.id, job_id);
    assert!(store.claim_next_job().unwrap().is_none());

    // On startup the worker requeues it, so it can be claimed and run again.
    assert_eq!(store.requeue_running_jobs().unwrap(), 1);
    assert_eq!(
        store.get_job(&org, &job_id).unwrap().unwrap().status,
        "queued"
    );
    assert!(jobs::process_one(&store).unwrap());
    assert_eq!(
        store.get_job(&org, &job_id).unwrap().unwrap().status,
        "completed"
    );
}

#[test]
fn claim_is_fifo_and_exclusive() {
    let store = store();
    let org = store.create_org("A").unwrap().id;
    let first = store
        .enqueue_job(&org, jobs::KIND_EXPORT_EVENTS, None)
        .unwrap();
    let _second = store
        .enqueue_job(&org, jobs::KIND_EXPORT_AUDIT, None)
        .unwrap();

    let claimed = store.claim_next_job().unwrap().unwrap();
    assert_eq!(claimed.id, first);
    assert_eq!(claimed.status, "running");
    // The claimed job is no longer queued, so the next claim returns the second.
    let claimed2 = store.claim_next_job().unwrap().unwrap();
    assert_ne!(claimed2.id, first);
    // Nothing else queued.
    assert!(store.claim_next_job().unwrap().is_none());
}

#[test]
fn retention_prunes_expired_sessions_and_finished_jobs() {
    let store = store();
    let org = store.create_org("A").unwrap().id;
    let member = store
        .create_member(&org, "alice", tellur_server::auth::Role::Admin)
        .unwrap();

    // An already-expired session (negative ttl) and a live one.
    let _expired = store.create_session(&member, -60).unwrap();
    let live = store.create_session(&member, 3600).unwrap();

    // A completed job and a still-queued job.
    let done = store
        .enqueue_job(&org, jobs::KIND_EXPORT_EVENTS, None)
        .unwrap();
    assert!(jobs::process_one(&store).unwrap());
    assert_eq!(
        store.get_job(&org, &done).unwrap().unwrap().status,
        "completed"
    );
    let queued = store
        .enqueue_job(&org, jobs::KIND_EXPORT_AUDIT, None)
        .unwrap();

    // Maintenance with retention off: expired session pruned, jobs kept (cutoff
    // is in the past, the job was just completed).
    let counts = jobs::run_maintenance_once(&store, 0).unwrap();
    assert_eq!(counts.sessions, 1);
    assert_eq!(counts.jobs, 0);
    assert!(store.session_principal(&live).unwrap().is_some());
    assert!(store.get_job(&org, &done).unwrap().is_some());

    // An absurd retention value must clamp, not wrap negative and delete fresh
    // jobs (the cutoff would otherwise land in the future).
    let counts = jobs::run_maintenance_once(&store, u64::MAX).unwrap();
    assert_eq!(counts.jobs, 0);
    assert!(store.get_job(&org, &done).unwrap().is_some());

    // Direct finished-job prune with a future cutoff removes completed/failed
    // but keeps queued.
    let future = (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339();
    assert_eq!(store.prune_finished_jobs(&future).unwrap(), 1);
    assert!(store.get_job(&org, &done).unwrap().is_none());
    assert_eq!(
        store.get_job(&org, &queued).unwrap().unwrap().status,
        "queued"
    );
}
