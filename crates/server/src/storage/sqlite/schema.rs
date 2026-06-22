//! Schema migration and health check.

use super::*;

impl SqliteStore {
    pub(crate) fn migrate(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_meta (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS org (
                 id         TEXT PRIMARY KEY,
                 name       TEXT NOT NULL,
                 created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS member (
                 id           TEXT PRIMARY KEY,
                 org_id       TEXT NOT NULL REFERENCES org(id),
                 display_name TEXT NOT NULL,
                 role         TEXT NOT NULL,
                 created_at   TEXT NOT NULL,
                 -- Deactivated members cannot authenticate (SCIM deprovisioning).
                 active       INTEGER NOT NULL DEFAULT 1
             );
             CREATE INDEX IF NOT EXISTS idx_member_org ON member(org_id);
             CREATE TABLE IF NOT EXISTS api_token (
                 token_id    TEXT PRIMARY KEY,
                 member_id   TEXT NOT NULL REFERENCES member(id),
                 secret_hash TEXT NOT NULL,
                 created_at  TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS audit_log (
                 seq             INTEGER PRIMARY KEY AUTOINCREMENT,
                 ts              TEXT NOT NULL,
                 org_id          TEXT,
                 actor_member_id TEXT,
                 action          TEXT NOT NULL,
                 detail          TEXT NOT NULL,
                 prev_hash       TEXT NOT NULL,
                 entry_hash      TEXT NOT NULL
             );
             -- Single-row checkpoint of the audit chain head + length, so tail
             -- truncation / rollback to an earlier prefix is detectable.
             CREATE TABLE IF NOT EXISTS audit_head (
                 id           INTEGER PRIMARY KEY CHECK (id = 1),
                 head_hash    TEXT NOT NULL,
                 entry_count  INTEGER NOT NULL,
                 sealed_hash  TEXT NOT NULL DEFAULT '',
                 sealed_count INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE IF NOT EXISTS repo (
                 id         TEXT PRIMARY KEY,
                 org_id     TEXT NOT NULL REFERENCES org(id),
                 name       TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 UNIQUE (org_id, name)
             );
             CREATE TABLE IF NOT EXISTS event (
                 seq        INTEGER PRIMARY KEY AUTOINCREMENT,
                 id         TEXT NOT NULL UNIQUE,
                 org_id     TEXT NOT NULL,
                 repo_id    TEXT NOT NULL REFERENCES repo(id),
                 session_id TEXT NOT NULL,
                 ts         TEXT NOT NULL,
                 event_type TEXT NOT NULL,
                 actor      TEXT NOT NULL,
                 payload    TEXT NOT NULL,
                 prev_hash  TEXT NOT NULL,
                 entry_hash TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_event_repo ON event(repo_id, seq);
             -- Speeds up org-scoped aggregates in org_report.
             CREATE INDEX IF NOT EXISTS idx_event_org ON event(org_id);
             -- Per-repo chain head + length checkpoint, so tail truncation /
             -- rollback of a repo's event log is detectable.
             CREATE TABLE IF NOT EXISTS event_head (
                 repo_id     TEXT PRIMARY KEY REFERENCES repo(id),
                 head_hash   TEXT NOT NULL,
                 entry_count INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS policy (
                 org_id     TEXT NOT NULL REFERENCES org(id),
                 name       TEXT NOT NULL,
                 content    TEXT NOT NULL,
                 version    INTEGER NOT NULL,
                 updated_at TEXT NOT NULL,
                 PRIMARY KEY (org_id, name)
             );
             CREATE TABLE IF NOT EXISTS attribution (
                 org_id       TEXT NOT NULL,
                 repo_id      TEXT NOT NULL REFERENCES repo(id),
                 file_path    TEXT NOT NULL,
                 git_blob_sha TEXT NOT NULL,
                 ranges_json  TEXT NOT NULL,
                 updated_at   TEXT NOT NULL,
                 PRIMARY KEY (org_id, repo_id, file_path)
             );
             -- Fine-grained per-repo role grants. Additive over the member's org
             -- role: effective role on a repo is max(org_role, repo grant).
             -- Opt-in source link template per repo (A12). Stores only a URL
             -- template — never source code. {path}/{start}/{end} are
             -- substituted client-side to deep-link the provider.
             CREATE TABLE IF NOT EXISTS repo_source (
                 repo_id      TEXT PRIMARY KEY REFERENCES repo(id),
                 org_id       TEXT NOT NULL REFERENCES org(id),
                 template     TEXT,
                 raw_template TEXT,
                 source_token TEXT,
                 updated_at   TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS github_installation (
                 installation_id INTEGER PRIMARY KEY,
                 org_id          TEXT NOT NULL REFERENCES org(id),
                 account_login   TEXT NOT NULL,
                 updated_at      TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS github_note_harvest (
                 org_id      TEXT NOT NULL,
                 repo_id     TEXT NOT NULL REFERENCES repo(id),
                 commit_sha  TEXT NOT NULL,
                 note_sha    TEXT NOT NULL,
                 harvested_at TEXT NOT NULL,
                 PRIMARY KEY (org_id, repo_id, commit_sha)
             );
             CREATE TABLE IF NOT EXISTS repo_role (
                 org_id     TEXT NOT NULL REFERENCES org(id),
                 repo_id    TEXT NOT NULL REFERENCES repo(id),
                 member_id  TEXT NOT NULL REFERENCES member(id),
                 role       TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 PRIMARY KEY (repo_id, member_id)
             );
             CREATE INDEX IF NOT EXISTS idx_repo_role_repo ON repo_role(repo_id);
             -- SSO identity: maps a member to a (globally unique) email and an
             -- optional OIDC subject bound on first login. Separate from member
             -- so the core identity table is untouched.
             CREATE TABLE IF NOT EXISTS member_identity (
                 member_id    TEXT PRIMARY KEY REFERENCES member(id),
                 email        TEXT UNIQUE,
                 oidc_issuer  TEXT,
                 oidc_subject TEXT,
                 external_id  TEXT,
                 -- A subject is only unique within an issuer.
                 UNIQUE (oidc_issuer, oidc_subject)
             );
             -- Org-scoped SCIM provisioning tokens (secret stored hashed).
             CREATE TABLE IF NOT EXISTS scim_token (
                 token_id    TEXT PRIMARY KEY,
                 org_id      TEXT NOT NULL REFERENCES org(id),
                 secret_hash TEXT NOT NULL,
                 created_at  TEXT NOT NULL
             );
             -- Pending OIDC login transactions (CSRF state -> PKCE/nonce +
             -- a browser-binding secret matched against a login cookie).
             CREATE TABLE IF NOT EXISTS oidc_login (
                 state           TEXT PRIMARY KEY,
                 pkce_verifier   TEXT NOT NULL,
                 nonce           TEXT NOT NULL,
                 browser_binding TEXT NOT NULL,
                 created_at      TEXT NOT NULL
             );
             -- Browser sessions (opaque id -> member, with expiry).
             CREATE TABLE IF NOT EXISTS session (
                 id         TEXT PRIMARY KEY,
                 member_id  TEXT NOT NULL REFERENCES member(id),
                 created_at TEXT NOT NULL,
                 expires_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_session_member ON session(member_id);
             -- Device-authorization requests for CLI `tellur login` (RFC 8628).
             -- Polled by the secret device_code; approved by the typed user_code.
             CREATE TABLE IF NOT EXISTS device_auth (
                 device_code TEXT PRIMARY KEY,
                 user_code   TEXT NOT NULL UNIQUE,
                 status      TEXT NOT NULL,
                 member_id   TEXT,
                 created_at  TEXT NOT NULL
             );
             -- Durable background jobs (e.g. large org exports).
             CREATE TABLE IF NOT EXISTS job (
                 id         TEXT PRIMARY KEY,
                 org_id     TEXT NOT NULL REFERENCES org(id),
                 kind       TEXT NOT NULL,
                 status     TEXT NOT NULL,
                 result     TEXT,
                 error      TEXT,
                 params     TEXT,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_job_status ON job(status, created_at);
             -- SCIM groups + membership (group displayName drives org roles).
             CREATE TABLE IF NOT EXISTS scim_group (
                 id           TEXT PRIMARY KEY,
                 org_id       TEXT NOT NULL REFERENCES org(id),
                 display_name TEXT NOT NULL,
                 external_id  TEXT,
                 created_at   TEXT NOT NULL,
                 updated_at   TEXT NOT NULL,
                 UNIQUE (org_id, display_name)
             );
             CREATE TABLE IF NOT EXISTS scim_group_member (
                 group_id  TEXT NOT NULL REFERENCES scim_group(id),
                 member_id TEXT NOT NULL REFERENCES member(id),
                 PRIMARY KEY (group_id, member_id)
             );
             CREATE INDEX IF NOT EXISTS idx_group_member ON scim_group_member(member_id);
             -- Policy-compliance snapshots (A8): append-only, timestamped per
             -- (org, repo, policy version); the dashboard reads the latest.
             CREATE TABLE IF NOT EXISTS compliance_snapshot (
                 id             TEXT PRIMARY KEY,
                 org_id         TEXT NOT NULL REFERENCES org(id),
                 repo_id        TEXT NOT NULL,
                 repo_name      TEXT NOT NULL,
                 policy_name    TEXT NOT NULL,
                 policy_version INTEGER NOT NULL,
                 evaluated_at   TEXT NOT NULL,
                 ai_ranges      INTEGER NOT NULL,
                 violations     INTEGER NOT NULL,
                 high           INTEGER NOT NULL,
                 medium         INTEGER NOT NULL,
                 low            INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_compliance_latest
                 ON compliance_snapshot(org_id, repo_id, evaluated_at);",
        )
        .context("failed to create schema")?;

        // Additive migrations for columns introduced after a table's first
        // version. `CREATE TABLE IF NOT EXISTS` is a no-op on an already-created
        // table, so columns added later must be applied with guarded ALTERs or
        // an upgraded database would be missing them (e.g. `member.active`, which
        // every auth lookup now queries).
        ensure_column(&conn, "member", "active", "INTEGER NOT NULL DEFAULT 1")?;
        ensure_column(&conn, "job", "params", "TEXT")?;
        ensure_column(&conn, "repo_source", "raw_template", "TEXT")?;
        // v16 created repo_source.template as NOT NULL; A12's raw-only configs
        // store a NULL there, so drop the constraint by rebuilding the table
        // (SQLite can't ALTER it away). Guarded so it runs only on old DBs.
        let template_notnull: i64 = conn
            .query_row(
                "SELECT \"notnull\" FROM pragma_table_info('repo_source') WHERE name = 'template'",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if template_notnull == 1 {
            conn.execute_batch(
                "CREATE TABLE repo_source_new (
                     repo_id      TEXT PRIMARY KEY REFERENCES repo(id),
                     org_id       TEXT NOT NULL REFERENCES org(id),
                     template     TEXT,
                     raw_template TEXT,
                     updated_at   TEXT NOT NULL
                 );
                 INSERT INTO repo_source_new (repo_id, org_id, template, raw_template, updated_at)
                     SELECT repo_id, org_id, template, raw_template, updated_at FROM repo_source;
                 DROP TABLE repo_source;
                 ALTER TABLE repo_source_new RENAME TO repo_source;",
            )?;
        }
        // v19: optional provider token for the private-repo blob proxy (A12).
        // After the rebuild above, so the column survives that narrow upgrade.
        ensure_column(&conn, "repo_source", "source_token", "TEXT")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS github_installation (
                 installation_id INTEGER PRIMARY KEY,
                 org_id          TEXT NOT NULL REFERENCES org(id),
                 account_login   TEXT NOT NULL,
                 updated_at      TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS github_note_harvest (
                 org_id      TEXT NOT NULL,
                 repo_id     TEXT NOT NULL REFERENCES repo(id),
                 commit_sha  TEXT NOT NULL,
                 note_sha    TEXT NOT NULL,
                 harvested_at TEXT NOT NULL,
                 PRIMARY KEY (org_id, repo_id, commit_sha)
             );",
        )?;
        ensure_column(
            &conn,
            "audit_head",
            "sealed_hash",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        ensure_column(
            &conn,
            "audit_head",
            "sealed_count",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column(&conn, "member_identity", "oidc_issuer", "TEXT")?;
        ensure_column(&conn, "member_identity", "external_id", "TEXT")?;
        ensure_column(
            &conn,
            "oidc_login",
            "browser_binding",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        // Composite uniqueness for (issuer, subject) also applies to upgraded
        // DBs (inline UNIQUE in CREATE only covers fresh installs).
        conn.execute_batch(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_member_identity_oidc
                 ON member_identity(oidc_issuer, oidc_subject);",
        )?;

        conn.execute(
            "INSERT INTO schema_meta (key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [SCHEMA_VERSION],
        )?;
        Ok(())
    }

    pub(crate) fn health_check(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT value FROM schema_meta WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .context("database health check failed")?;
        Ok(())
    }
}
