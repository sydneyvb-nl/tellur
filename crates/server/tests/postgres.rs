//! Postgres backend integration tests.
//!
//! These run only when `TELLUR_TEST_DATABASE_URL` points at a disposable
//! Postgres database (CI provides one; locally export it yourself). The test
//! resets the `public` schema first, so **never** point it at real data.
//!
//! ```bash
//! TELLUR_TEST_DATABASE_URL=postgres://postgres@127.0.0.1:5432/tellur_test \
//!   cargo test -p tellur-server --test postgres
//! ```

use r2d2_postgres::postgres::{Client, NoTls};
use tellur_server::auth::Role;
use tellur_server::storage::{AuditEntry, IngestEvent, PostgresStore, Store};

use tellur_core::schema::types::{
    AttributionRange, AttributionState, EvidenceStrength, FileAttribution, Origin,
};

/// Serializes the schema-reset tests: they all wipe the shared `public` schema,
/// so they must not run concurrently (cargo runs tests in parallel by default).
static DB_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A configured store plus the held lock; dropping the guard frees the DB for
/// the next test. Returns `None` when no test DB is configured (so the suite is
/// a no-op outside CI / a local PG).
fn store() -> Option<(PostgresStore, std::sync::MutexGuard<'static, ()>)> {
    let url = std::env::var("TELLUR_TEST_DATABASE_URL").ok()?;
    let guard = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Wipe the schema for a clean, deterministic slate (test DB only).
    let mut admin = Client::connect(&url, NoTls).expect("connect to test DB");
    admin
        .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .expect("reset schema");
    let store = PostgresStore::connect(&url).expect("build pool");
    store.migrate().expect("migrate");
    Some((store, guard))
}

fn ev(session: &str, kind: &str, actor: &str) -> IngestEvent {
    IngestEvent {
        session_id: session.to_string(),
        timestamp: "2026-06-05T00:00:00Z".to_string(),
        event_type: kind.to_string(),
        actor: actor.to_string(),
        payload: serde_json::json!({ "file": "src/main.rs" }),
    }
}

#[test]
fn full_store_surface() {
    let Some((store, _guard)) = store() else {
        eprintln!("skipping: TELLUR_TEST_DATABASE_URL not set");
        return;
    };

    // ── Identity & tenancy ──────────────────────────────────────────────────
    let org_a = store.create_org("Org A").unwrap();
    let org_b = store.create_org("Org B").unwrap();
    let admin = store
        .create_member(&org_a.id, "alice", Role::Admin)
        .unwrap();
    let token = store.create_token(&admin).unwrap();

    let principal = store.authenticate(&token.plaintext).unwrap().unwrap();
    assert_eq!(principal.org_id, org_a.id);
    assert_eq!(principal.member_id, admin);
    assert_eq!(principal.role, Role::Admin);
    assert!(store.authenticate("tlr_bogus_token").unwrap().is_none());

    // ── Repos: get-or-create + lookup by id and by name ─────────────────────
    let repo = store.ensure_repo(&org_a.id, "service").unwrap();
    let again = store.ensure_repo(&org_a.id, "service").unwrap();
    assert_eq!(repo.id, again.id, "ensure_repo must be idempotent");
    assert_eq!(
        store.find_repo(&org_a.id, &repo.id).unwrap().unwrap().id,
        repo.id
    );
    assert_eq!(
        store.find_repo(&org_a.id, "service").unwrap().unwrap().id,
        repo.id
    );
    assert!(store.find_repo(&org_b.id, "service").unwrap().is_none());

    // ── Event chain ─────────────────────────────────────────────────────────
    let ids = store
        .append_events(
            &org_a.id,
            &repo.id,
            &[
                ev("s1", "edit", "claude"),
                ev("s1", "edit", "human"),
                ev("s2", "review", "human"),
            ],
        )
        .unwrap();
    assert_eq!(ids.len(), 3);
    assert_eq!(store.event_count(&org_a.id, &repo.id).unwrap(), 3);
    assert!(store.verify_event_chain(&org_a.id, &repo.id).unwrap());

    // Cross-tenant append must be refused.
    assert!(
        store
            .append_events(&org_b.id, &repo.id, &[ev("x", "edit", "mallory")])
            .is_err()
    );

    // Pagination: newest first, cursor by seq.
    let page1 = store.list_events(&org_a.id, &repo.id, 2, None).unwrap();
    assert_eq!(page1.len(), 2);
    assert!(page1[0].seq > page1[1].seq);
    let page2 = store
        .list_events(&org_a.id, &repo.id, 2, Some(page1[1].seq))
        .unwrap();
    assert_eq!(page2.len(), 1);

    // ── Attribution ─────────────────────────────────────────────────────────
    let attr = FileAttribution {
        schema: "tellur.attribution.v1".to_string(),
        file_path: "src/main.rs".to_string(),
        git_blob_sha: "abc123".to_string(),
        ranges: vec![AttributionRange {
            range_id: "rng1".to_string(),
            start_line: 1,
            end_line: 10,
            origin: Origin::Ai,
            evidence_strength: EvidenceStrength::Recorded,
            confidence: 0.95,
            state: AttributionState::Exact,
            session_id: "s1".to_string(),
            event_ids: vec![],
            agent_id: "claude-code".to_string(),
            model_id: None,
            prompt_hash: None,
            context_set_id: None,
            policy_tags: vec![],
            risk_tags: vec![],
            risk_level: None,
            tests_run: vec![],
            tests_passed: false,
            reviewer: None,
            reviewed_at: None,
        }],
        updated_at: "2026-06-05T00:00:00Z".to_string(),
    };
    assert_eq!(
        store
            .put_attributions(&org_a.id, &repo.id, &[attr])
            .unwrap(),
        1
    );
    let listed = store.list_attributions(&org_a.id, &repo.id).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].git_blob_sha, "abc123");
    assert_eq!(listed[0].ranges[0].end_line, 10);

    // ── Listings & rollup ───────────────────────────────────────────────────
    let repos = store.list_repos(&org_a.id).unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].event_count, 3);

    let report = store.org_report(&org_a.id).unwrap();
    assert_eq!(report.total_events, 3);
    assert_eq!(report.distinct_sessions, 2);
    assert_eq!(report.by_type.get("edit"), Some(&2));
    assert_eq!(report.by_actor.get("human"), Some(&2));

    // ── Policy distribution ─────────────────────────────────────────────────
    assert_eq!(
        store.put_policy(&org_a.id, "default", "rules: []").unwrap(),
        1
    );
    assert_eq!(
        store
            .put_policy(&org_a.id, "default", "rules: [x]")
            .unwrap(),
        2
    );
    let pol = store.get_policy(&org_a.id, "default").unwrap().unwrap();
    assert_eq!(pol.version, 2);
    assert_eq!(pol.content, "rules: [x]");
    assert_eq!(store.list_policies(&org_a.id).unwrap().len(), 1);
    assert!(store.get_policy(&org_b.id, "default").unwrap().is_none());

    // ── Export portal ───────────────────────────────────────────────────────
    assert_eq!(store.export_events(&org_a.id).unwrap().len(), 3);

    // ── Audit log ───────────────────────────────────────────────────────────
    store
        .append_audit(&AuditEntry {
            org_id: Some(org_a.id.clone()),
            actor_member_id: Some(admin.clone()),
            action: "test.action".to_string(),
            detail: "hello".to_string(),
        })
        .unwrap();
    assert_eq!(store.audit_len().unwrap(), 1);
    assert!(store.verify_audit_chain().unwrap());
    assert_eq!(store.export_audit(&org_a.id).unwrap().len(), 1);
}

#[test]
fn tampering_breaks_event_chain() {
    let Some((store, _guard)) = store() else {
        eprintln!("skipping: TELLUR_TEST_DATABASE_URL not set");
        return;
    };
    let org = store.create_org("Org").unwrap();
    let repo = store.ensure_repo(&org.id, "r").unwrap();
    store
        .append_events(&org.id, &repo.id, &[ev("s", "edit", "claude")])
        .unwrap();
    assert!(store.verify_event_chain(&org.id, &repo.id).unwrap());

    // Mutate a stored payload out from under the chain.
    let url = std::env::var("TELLUR_TEST_DATABASE_URL").unwrap();
    let mut admin = Client::connect(&url, NoTls).unwrap();
    admin
        .execute(
            "UPDATE event SET payload = '{\"file\":\"tampered\"}' WHERE repo_id = $1",
            &[&repo.id],
        )
        .unwrap();

    assert!(
        !store.verify_event_chain(&org.id, &repo.id).unwrap(),
        "tampered payload must break the chain"
    );
}
