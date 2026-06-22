//! Schema migration and health check.

use super::*;

impl PostgresStore {
    pub(crate) fn migrate(&self) -> Result<()> {
        let mut client = self.client()?;
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS schema_meta (
                     key TEXT PRIMARY KEY, value TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS org (
                     id TEXT PRIMARY KEY, name TEXT NOT NULL, created_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS member (
                     id TEXT PRIMARY KEY,
                     org_id TEXT NOT NULL REFERENCES org(id),
                     display_name TEXT NOT NULL, role TEXT NOT NULL, created_at TEXT NOT NULL,
                     active BOOLEAN NOT NULL DEFAULT TRUE
                 );
                 CREATE INDEX IF NOT EXISTS idx_member_org ON member(org_id);
                 CREATE TABLE IF NOT EXISTS api_token (
                     token_id TEXT PRIMARY KEY,
                     member_id TEXT NOT NULL REFERENCES member(id),
                     secret_hash TEXT NOT NULL, created_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS audit_log (
                     seq BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                     ts TEXT NOT NULL, org_id TEXT, actor_member_id TEXT,
                     action TEXT NOT NULL, detail TEXT NOT NULL,
                     prev_hash TEXT NOT NULL, entry_hash TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS audit_head (
                     id INTEGER PRIMARY KEY CHECK (id = 1),
                     head_hash TEXT NOT NULL, entry_count BIGINT NOT NULL,
                     sealed_hash TEXT NOT NULL DEFAULT '',
                     sealed_count BIGINT NOT NULL DEFAULT 0
                 );
                 CREATE TABLE IF NOT EXISTS repo (
                     id TEXT PRIMARY KEY,
                     org_id TEXT NOT NULL REFERENCES org(id),
                     name TEXT NOT NULL, created_at TEXT NOT NULL,
                     UNIQUE (org_id, name)
                 );
                 CREATE TABLE IF NOT EXISTS event (
                     seq BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                     id TEXT NOT NULL UNIQUE,
                     org_id TEXT NOT NULL,
                     repo_id TEXT NOT NULL REFERENCES repo(id),
                     session_id TEXT NOT NULL, ts TEXT NOT NULL,
                     event_type TEXT NOT NULL, actor TEXT NOT NULL, payload TEXT NOT NULL,
                     prev_hash TEXT NOT NULL, entry_hash TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_event_repo ON event(repo_id, seq);
                 CREATE INDEX IF NOT EXISTS idx_event_org ON event(org_id);
                 CREATE TABLE IF NOT EXISTS event_head (
                     repo_id TEXT PRIMARY KEY REFERENCES repo(id),
                     head_hash TEXT NOT NULL, entry_count BIGINT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS policy (
                     org_id TEXT NOT NULL REFERENCES org(id),
                     name TEXT NOT NULL, content TEXT NOT NULL,
                     version BIGINT NOT NULL, updated_at TEXT NOT NULL,
                     PRIMARY KEY (org_id, name)
                 );
                 CREATE TABLE IF NOT EXISTS attribution (
                     org_id TEXT NOT NULL, repo_id TEXT NOT NULL REFERENCES repo(id),
                     file_path TEXT NOT NULL, git_blob_sha TEXT NOT NULL,
                     ranges_json TEXT NOT NULL, updated_at TEXT NOT NULL,
                     PRIMARY KEY (org_id, repo_id, file_path)
                 );
                 CREATE TABLE IF NOT EXISTS repo_source (
                     repo_id TEXT PRIMARY KEY REFERENCES repo(id),
                     org_id TEXT NOT NULL REFERENCES org(id),
                     template TEXT, raw_template TEXT, source_token TEXT,
                     updated_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS github_installation (
                     installation_id BIGINT PRIMARY KEY,
                     org_id TEXT NOT NULL REFERENCES org(id),
                     account_login TEXT NOT NULL,
                     updated_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS github_note_harvest (
                     org_id TEXT NOT NULL,
                     repo_id TEXT NOT NULL REFERENCES repo(id),
                     commit_sha TEXT NOT NULL,
                     note_sha TEXT NOT NULL,
                     harvested_at TEXT NOT NULL,
                     PRIMARY KEY (org_id, repo_id, commit_sha)
                 );
                 CREATE TABLE IF NOT EXISTS repo_role (
                     org_id TEXT NOT NULL REFERENCES org(id),
                     repo_id TEXT NOT NULL REFERENCES repo(id),
                     member_id TEXT NOT NULL REFERENCES member(id),
                     role TEXT NOT NULL, updated_at TEXT NOT NULL,
                     PRIMARY KEY (repo_id, member_id)
                 );
                 CREATE INDEX IF NOT EXISTS idx_repo_role_repo ON repo_role(repo_id);
                 CREATE TABLE IF NOT EXISTS member_identity (
                     member_id TEXT PRIMARY KEY REFERENCES member(id),
                     email TEXT UNIQUE, oidc_issuer TEXT, oidc_subject TEXT,
                     external_id TEXT,
                     UNIQUE (oidc_issuer, oidc_subject)
                 );
                 CREATE TABLE IF NOT EXISTS scim_token (
                     token_id TEXT PRIMARY KEY,
                     org_id TEXT NOT NULL REFERENCES org(id),
                     secret_hash TEXT NOT NULL, created_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS oidc_login (
                     state TEXT PRIMARY KEY, pkce_verifier TEXT NOT NULL,
                     nonce TEXT NOT NULL, browser_binding TEXT NOT NULL,
                     created_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS session (
                     id TEXT PRIMARY KEY, member_id TEXT NOT NULL REFERENCES member(id),
                     created_at TEXT NOT NULL, expires_at TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_session_member ON session(member_id);
                 CREATE TABLE IF NOT EXISTS device_auth (
                     device_code TEXT PRIMARY KEY, user_code TEXT NOT NULL UNIQUE,
                     status TEXT NOT NULL, member_id TEXT, created_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS job (
                     id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES org(id),
                     kind TEXT NOT NULL, status TEXT NOT NULL,
                     result TEXT, error TEXT, params TEXT,
                     created_at TEXT NOT NULL, updated_at TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_job_status ON job(status, created_at);
                 CREATE TABLE IF NOT EXISTS scim_group (
                     id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES org(id),
                     display_name TEXT NOT NULL, external_id TEXT,
                     created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                     UNIQUE (org_id, display_name)
                 );
                 CREATE TABLE IF NOT EXISTS scim_group_member (
                     group_id TEXT NOT NULL REFERENCES scim_group(id),
                     member_id TEXT NOT NULL REFERENCES member(id),
                     PRIMARY KEY (group_id, member_id)
                 );
                 CREATE INDEX IF NOT EXISTS idx_group_member ON scim_group_member(member_id);
                 CREATE TABLE IF NOT EXISTS compliance_snapshot (
                     id TEXT PRIMARY KEY, org_id TEXT NOT NULL REFERENCES org(id),
                     repo_id TEXT NOT NULL, repo_name TEXT NOT NULL,
                     policy_name TEXT NOT NULL, policy_version BIGINT NOT NULL,
                     evaluated_at TEXT NOT NULL, ai_ranges BIGINT NOT NULL,
                     violations BIGINT NOT NULL, high BIGINT NOT NULL,
                     medium BIGINT NOT NULL, low BIGINT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_compliance_latest
                     ON compliance_snapshot(org_id, repo_id, evaluated_at);",
            )
            .context("failed to create schema")?;
        // Additive migrations for columns introduced after a table's first
        // version (CREATE TABLE IF NOT EXISTS no-ops on existing tables, so an
        // upgraded DB would otherwise be missing e.g. member.active).
        client
            .batch_execute(
                "ALTER TABLE audit_head ADD COLUMN IF NOT EXISTS sealed_hash TEXT NOT NULL DEFAULT '';
                 ALTER TABLE audit_head ADD COLUMN IF NOT EXISTS sealed_count BIGINT NOT NULL DEFAULT 0;
                 ALTER TABLE repo_source ADD COLUMN IF NOT EXISTS raw_template TEXT;
                 ALTER TABLE repo_source ADD COLUMN IF NOT EXISTS source_token TEXT;
                 ALTER TABLE repo_source ALTER COLUMN template DROP NOT NULL;
                 CREATE TABLE IF NOT EXISTS github_installation (
                     installation_id BIGINT PRIMARY KEY,
                     org_id TEXT NOT NULL REFERENCES org(id),
                     account_login TEXT NOT NULL,
                     updated_at TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS github_note_harvest (
                     org_id TEXT NOT NULL,
                     repo_id TEXT NOT NULL REFERENCES repo(id),
                     commit_sha TEXT NOT NULL,
                     note_sha TEXT NOT NULL,
                     harvested_at TEXT NOT NULL,
                     PRIMARY KEY (org_id, repo_id, commit_sha)
                 );
                 ALTER TABLE job ADD COLUMN IF NOT EXISTS params TEXT;
                 ALTER TABLE member ADD COLUMN IF NOT EXISTS active BOOLEAN NOT NULL DEFAULT TRUE;
                 ALTER TABLE member_identity ADD COLUMN IF NOT EXISTS oidc_issuer TEXT;
                 ALTER TABLE member_identity ADD COLUMN IF NOT EXISTS external_id TEXT;
                 ALTER TABLE oidc_login ADD COLUMN IF NOT EXISTS browser_binding TEXT NOT NULL DEFAULT '';
                 CREATE UNIQUE INDEX IF NOT EXISTS idx_member_identity_oidc
                     ON member_identity(oidc_issuer, oidc_subject);",
            )
            .context("failed to apply column migrations")?;
        client
            .execute(
                "INSERT INTO schema_meta (key, value) VALUES ('schema_version', '17')
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                &[],
            )
            .context("failed to record schema version")?;
        Ok(())
    }

    pub(crate) fn health_check(&self) -> Result<()> {
        // Verify the schema is migrated (not just that the DB is reachable), so
        // a connected-but-unmigrated backend reports not-ready instead of
        // failing real requests with missing-table errors.
        let mut client = self.client()?;
        client
            .query_one(
                "SELECT value FROM schema_meta WHERE key = 'schema_version'",
                &[],
            )
            .context("database health check failed")?;
        Ok(())
    }
}
