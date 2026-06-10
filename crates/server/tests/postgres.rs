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

    // ── Per-repo RBAC (additive grants) ──────────────────────────────────────
    let viewer = store.create_member(&org_a.id, "vic", Role::Viewer).unwrap();
    assert!(
        store
            .get_repo_role(&org_a.id, &repo.id, &viewer)
            .unwrap()
            .is_none()
    );
    store
        .set_repo_role(&org_a.id, &repo.id, &viewer, Role::Contributor)
        .unwrap();
    assert_eq!(
        store.get_repo_role(&org_a.id, &repo.id, &viewer).unwrap(),
        Some(Role::Contributor)
    );
    // Upsert: re-granting changes the role in place.
    store
        .set_repo_role(&org_a.id, &repo.id, &viewer, Role::Admin)
        .unwrap();
    let grants = store.list_repo_roles(&org_a.id, &repo.id).unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].role, "admin");
    // Cross-tenant grants are refused (repo not in org_b).
    assert!(
        store
            .set_repo_role(&org_b.id, &repo.id, &viewer, Role::Admin)
            .is_err()
    );
    assert!(
        store
            .remove_repo_role(&org_a.id, &repo.id, &viewer)
            .unwrap()
    );
    assert!(
        !store
            .remove_repo_role(&org_a.id, &repo.id, &viewer)
            .unwrap()
    );

    // ── Activity time-series + repo facts (dashboard D1) ─────────────────────
    use tellur_server::storage::ActivityGroup;
    let since = "2000-01-01T00:00:00Z";
    let by_type = store
        .activity_by_day(&org_a.id, since, ActivityGroup::Type)
        .unwrap();
    let typed: u64 = by_type.iter().map(|b| b.count).sum();
    assert_eq!(typed, 3);
    assert!(by_type.iter().all(|b| b.day.len() == 10)); // YYYY-MM-DD
    let by_actor = store
        .activity_by_day(&org_a.id, since, ActivityGroup::Actor)
        .unwrap();
    assert!(by_actor.iter().any(|b| b.key == "human" && b.count == 2));
    // Future cutoff yields nothing.
    assert!(
        store
            .activity_by_day(&org_a.id, "2999-01-01T00:00:00Z", ActivityGroup::Type)
            .unwrap()
            .is_empty()
    );

    let facts = store.repo_facts(&org_a.id, &repo.id).unwrap();
    assert_eq!(facts.event_count, 3);
    assert!(facts.contributors.iter().any(|c| c == "human"));
    assert!(facts.last_activity.is_some());

    // ── Sessions list + detail (dashboard D2) ────────────────────────────────
    let sessions = store
        .list_sessions(&org_a.id, None, None, None, 50)
        .unwrap();
    assert_eq!(sessions.len(), 2); // s1 (2 events) + s2 (1)
    let s1 = sessions.iter().find(|s| s.session_id == "s1").unwrap();
    assert_eq!(s1.event_count, 2);
    assert!(s1.actors.iter().any(|a| a == "human"));
    assert!(s1.actors.iter().any(|a| a == "claude"));
    assert_eq!(s1.repos, vec![repo.id.clone()]);
    // Actor filter narrows the set (claude only appears in s1).
    assert_eq!(
        store
            .list_sessions(&org_a.id, None, Some("claude"), None, 50)
            .unwrap()
            .len(),
        1
    );
    let s1_events = store.session_events(&org_a.id, "s1", 50).unwrap();
    assert_eq!(s1_events.len(), 2);
    assert!(s1_events[0].seq < s1_events[1].seq);
    assert!(
        store
            .session_events(&org_a.id, "ghost", 50)
            .unwrap()
            .is_empty()
    );

    // ── SSO: identity, login tx, sessions ────────────────────────────────────
    let sso = store
        .provision_member(&org_a.id, "Sso User", Role::Contributor, "sso@corp.test")
        .unwrap();
    let by_email = store
        .find_member_by_email("sso@corp.test")
        .unwrap()
        .unwrap();
    assert_eq!(by_email.member_id, sso);
    assert_eq!(by_email.role, Role::Contributor);
    assert!(
        store
            .find_member_by_email("nobody@corp.test")
            .unwrap()
            .is_none()
    );
    let issuer = "https://idp.test";
    assert!(
        store
            .find_member_by_oidc_subject(issuer, "idp-1")
            .unwrap()
            .is_none()
    );
    assert!(store.bind_oidc_subject(&sso, issuer, "idp-1").unwrap());
    // Re-binding a different subject must be refused (no takeover).
    assert!(!store.bind_oidc_subject(&sso, issuer, "idp-2").unwrap());
    assert_eq!(
        store
            .find_member_by_oidc_subject(issuer, "idp-1")
            .unwrap()
            .unwrap()
            .member_id,
        sso
    );
    // The same subject at a *different* issuer is not the same identity.
    assert!(
        store
            .find_member_by_oidc_subject("https://other.test", "idp-1")
            .unwrap()
            .is_none()
    );
    // Login tx carries the browser binding and is counted + TTL-pruned.
    store.put_login("old-state", "v", "n", "bind-1").unwrap();
    assert_eq!(store.count_logins().unwrap(), 1);
    let tx = store.take_login("old-state").unwrap().unwrap();
    assert_eq!(tx.browser_binding, "bind-1");
    store.put_login("s2", "v", "n", "b2").unwrap();
    assert_eq!(store.prune_expired_logins(-1).unwrap(), 1);
    assert_eq!(store.count_logins().unwrap(), 0);

    // Login transaction is consumed exactly once.
    store
        .put_login("state-1", "verifier-1", "nonce-1", "bind-x")
        .unwrap();
    let tx = store.take_login("state-1").unwrap().unwrap();
    assert_eq!(tx.pkce_verifier, "verifier-1");
    assert_eq!(tx.nonce, "nonce-1");
    assert!(store.take_login("state-1").unwrap().is_none());

    // Sessions resolve to a principal until deleted.
    let sid = store.create_session(&sso, 3600).unwrap();
    let sp = store.session_principal(&sid).unwrap().unwrap();
    assert_eq!(sp.member_id, sso);
    assert!(store.delete_session(&sid).unwrap());
    assert!(store.session_principal(&sid).unwrap().is_none());
    // An already-expired session does not resolve.
    let expired = store.create_session(&sso, -10).unwrap();
    assert!(store.session_principal(&expired).unwrap().is_none());

    // ── SCIM provisioning ─────────────────────────────────────────────────────
    let scim_tok = store.create_scim_token(&org_a.id).unwrap().plaintext;
    assert_eq!(
        store.authenticate_scim(&scim_tok).unwrap().as_deref(),
        Some(org_a.id.as_str())
    );
    assert!(store.authenticate_scim("tlr_bogus_x").unwrap().is_none());
    let su = store
        .scim_create_user(
            &org_a.id,
            "scim@corp.test",
            "Scim U",
            Role::Viewer,
            Some("ext-9"),
        )
        .unwrap();
    assert!(su.active);
    assert_eq!(
        store
            .scim_list_users(&org_a.id, Some("scim@corp.test"))
            .unwrap()
            .len(),
        1
    );
    // An active SCIM user resolves for SSO; deactivating hides it from auth.
    assert!(
        store
            .find_member_by_email("scim@corp.test")
            .unwrap()
            .is_some()
    );
    let updated = store
        .scim_update_user(
            &org_a.id,
            &su.member_id,
            None,
            None,
            Some(Role::Admin),
            Some(false),
            None,
        )
        .unwrap()
        .unwrap();
    assert!(!updated.active);
    assert_eq!(updated.role, Role::Admin);
    assert!(
        store
            .find_member_by_email("scim@corp.test")
            .unwrap()
            .is_none()
    );
    // Email change via SCIM PUT is persisted (and reactivation).
    let renamed = store
        .scim_update_user(
            &org_a.id,
            &su.member_id,
            Some("scim2@corp.test"),
            None,
            None,
            Some(true),
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(renamed.email, "scim2@corp.test");
    assert!(
        store
            .find_member_by_email("scim2@corp.test")
            .unwrap()
            .is_some()
    );
    // Cross-org update is refused (tenant scoping).
    assert!(
        store
            .scim_update_user(&org_b.id, &su.member_id, None, None, None, Some(true), None)
            .unwrap()
            .is_none()
    );

    // ── Durable jobs ──────────────────────────────────────────────────────────
    let job_id = store.enqueue_job(&org_a.id, "export.events").unwrap();
    let claimed = store.claim_next_job().unwrap().unwrap();
    assert_eq!(claimed.id, job_id);
    assert_eq!(claimed.status, "running");
    // Requeue reclaims the in-flight job; then it can be claimed again.
    assert_eq!(store.requeue_running_jobs().unwrap(), 1);
    assert_eq!(
        store.get_job(&org_a.id, &job_id).unwrap().unwrap().status,
        "queued"
    );
    store.complete_job(&job_id, "{\"ok\":true}").unwrap();
    assert_eq!(
        store.get_job(&org_a.id, &job_id).unwrap().unwrap().status,
        "completed"
    );
    assert!(store.get_job(&org_b.id, &job_id).unwrap().is_none());
    // list_jobs: tenant-scoped, newest first.
    let jobs = store.list_jobs(&org_a.id, 50).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, job_id);
    assert!(store.list_jobs(&org_b.id, 50).unwrap().is_empty());

    // ── SCIM groups: membership drives + revokes roles ───────────────────────
    let gm = store
        .scim_create_user(&org_a.id, "grp@corp.test", "Grp", Role::Viewer, None)
        .unwrap();
    let group = store
        .scim_create_group(
            &org_a.id,
            "tellur-admin",
            None,
            std::slice::from_ref(&gm.member_id),
        )
        .unwrap();
    assert_eq!(
        store
            .find_member_by_email("grp@corp.test")
            .unwrap()
            .unwrap()
            .role,
        Role::Admin
    );
    // Deleting the only mapping group revokes the elevated role.
    assert!(store.scim_delete_group(&org_a.id, &group.id).unwrap());
    assert_eq!(
        store
            .find_member_by_email("grp@corp.test")
            .unwrap()
            .unwrap()
            .role,
        Role::Viewer
    );
    assert_eq!(store.recent_org_events(&org_a.id, 10).unwrap().len(), 3);

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
    // list_audit: paginated/filtered parity (newest first, tenant-scoped).
    store
        .append_audit(&AuditEntry {
            org_id: Some(org_a.id.clone()),
            actor_member_id: Some(admin.clone()),
            action: "test.other".to_string(),
            detail: "world".to_string(),
        })
        .unwrap();
    let recs = store
        .list_audit(&org_a.id, None, None, None, None, 1)
        .unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].detail, "world"); // newest first
    let cursor = recs[0].seq;
    let page2 = store
        .list_audit(&org_a.id, None, None, None, Some(cursor), 10)
        .unwrap();
    assert_eq!(page2.len(), 1);
    assert_eq!(page2[0].detail, "hello");
    // Action filter.
    let filtered = store
        .list_audit(&org_a.id, None, Some("test.other"), None, None, 10)
        .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].action, "test.other");
    // Tenant scope: org B sees none of A's audit rows.
    assert!(
        store
            .list_audit(&org_b.id, None, None, None, None, 10)
            .unwrap()
            .is_empty()
    );
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

#[test]
fn health_check_requires_migrated_schema() {
    let url = match std::env::var("TELLUR_TEST_DATABASE_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("skipping: TELLUR_TEST_DATABASE_URL not set");
            return;
        }
    };
    let _guard = DB_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Reachable but unmigrated: readiness must report not-ready, otherwise the
    // backend would later fail real requests with missing-table errors.
    let mut admin = Client::connect(&url, NoTls).expect("connect");
    admin
        .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .expect("reset schema");
    let store = PostgresStore::connect(&url).expect("pool");
    assert!(
        store.health_check().is_err(),
        "unmigrated schema must fail the health check"
    );

    // After migration the same check passes.
    store.migrate().unwrap();
    assert!(store.health_check().is_ok());
}
