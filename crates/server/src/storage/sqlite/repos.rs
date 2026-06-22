//! Repos, repo sources, GitHub installations, and per-repo roles.

use super::*;

impl SqliteStore {
    pub(crate) fn ensure_repo(&self, org_id: &str, name: &str) -> Result<Repo> {
        let conn = self.conn()?;
        let id = ids::generate_id("repo");
        // Race-safe get-or-create: insert if absent, then read the canonical id.
        conn.execute(
            "INSERT INTO repo (id, org_id, name, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(org_id, name) DO NOTHING",
            params![id, org_id, name, chrono::Utc::now().to_rfc3339()],
        )
        .context("failed to create repo")?;
        let real_id: String = conn.query_row(
            "SELECT id FROM repo WHERE org_id = ?1 AND name = ?2",
            params![org_id, name],
            |r| r.get(0),
        )?;
        Ok(Repo {
            id: real_id,
            org_id: org_id.to_string(),
            name: name.to_string(),
        })
    }

    pub(crate) fn find_repo(&self, org_id: &str, repo: &str) -> Result<Option<Repo>> {
        let conn = self.conn()?;
        let found: Option<(String, String)> = conn
            .query_row(
                "SELECT id, name FROM repo WHERE org_id = ?1 AND id = ?2",
                params![org_id, repo],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let found = match found {
            Some(repo) => Some(repo),
            None => conn
                .query_row(
                    "SELECT id, name FROM repo WHERE org_id = ?1 AND name = ?2",
                    params![org_id, repo],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?,
        };
        Ok(found.map(|(id, name)| Repo {
            id,
            org_id: org_id.to_string(),
            name,
        }))
    }

    pub(crate) fn get_repo_source(&self, org_id: &str, repo_id: &str) -> Result<RepoSource> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT template, raw_template, source_token FROM repo_source
                 WHERE org_id = ?1 AND repo_id = ?2",
                params![org_id, repo_id],
                |r| {
                    Ok(RepoSource {
                        link: r.get::<_, Option<String>>(0)?,
                        raw: r.get::<_, Option<String>>(1)?,
                        token: r.get::<_, Option<String>>(2)?,
                    })
                },
            )
            .optional()?;
        Ok(row.unwrap_or_default())
    }

    pub(crate) fn set_repo_source(
        &self,
        org_id: &str,
        repo_id: &str,
        link: Option<&str>,
        raw: Option<&str>,
        token: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn()?;
        if link.is_none() && raw.is_none() && token.is_none() {
            conn.execute(
                "DELETE FROM repo_source WHERE org_id = ?1 AND repo_id = ?2",
                params![org_id, repo_id],
            )?;
        } else {
            let now = chrono::Utc::now().to_rfc3339();
            conn.execute(
                "INSERT INTO repo_source (repo_id, org_id, template, raw_template, source_token, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(repo_id) DO UPDATE SET template = excluded.template,
                                                    raw_template = excluded.raw_template,
                                                    source_token = excluded.source_token,
                                                    updated_at = excluded.updated_at",
                params![repo_id, org_id, link, raw, token, now],
            )?;
        }
        Ok(())
    }

    pub(crate) fn set_github_installation(
        &self,
        org_id: &str,
        installation_id: i64,
        account_login: &str,
    ) -> Result<()> {
        let conn = self.conn()?;
        let org_exists = conn
            .query_row("SELECT 1 FROM org WHERE id = ?1", params![org_id], |_| {
                Ok(())
            })
            .optional()?
            .is_some();
        if !org_exists {
            bail!("org {org_id} not found");
        }
        conn.execute(
            "INSERT INTO github_installation
                 (installation_id, org_id, account_login, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(installation_id) DO UPDATE SET
                 org_id = excluded.org_id,
                 account_login = excluded.account_login,
                 updated_at = excluded.updated_at",
            params![
                installation_id,
                org_id,
                account_login,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub(crate) fn github_installation(
        &self,
        installation_id: i64,
    ) -> Result<Option<GithubInstallation>> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT org_id, installation_id, account_login, updated_at
             FROM github_installation WHERE installation_id = ?1",
            params![installation_id],
            |r| {
                Ok(GithubInstallation {
                    org_id: r.get(0)?,
                    installation_id: r.get(1)?,
                    account_login: r.get(2)?,
                    updated_at: r.get(3)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
    }

    pub(crate) fn mark_github_note_harvested(
        &self,
        org_id: &str,
        repo_id: &str,
        commit_sha: &str,
        note_sha: &str,
    ) -> Result<bool> {
        let conn = self.conn()?;
        let n = conn.execute(
            "INSERT OR IGNORE INTO github_note_harvest
                 (org_id, repo_id, commit_sha, note_sha, harvested_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                org_id,
                repo_id,
                commit_sha,
                note_sha,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(n == 1)
    }

    pub(crate) fn set_repo_role(
        &self,
        org_id: &str,
        repo_id: &str,
        member_id: &str,
        role: Role,
    ) -> Result<()> {
        let conn = self.conn()?;
        // Both the repo and the member must belong to the org (no cross-tenant
        // grants).
        let repo_ok: bool = conn
            .query_row(
                "SELECT 1 FROM repo WHERE id = ?1 AND org_id = ?2",
                params![repo_id, org_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !repo_ok {
            bail!("repo {repo_id} not found in org {org_id}");
        }
        let member_ok: bool = conn
            .query_row(
                "SELECT 1 FROM member WHERE id = ?1 AND org_id = ?2",
                params![member_id, org_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !member_ok {
            bail!("member {member_id} not found in org {org_id}");
        }
        conn.execute(
            "INSERT INTO repo_role (org_id, repo_id, member_id, role, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(repo_id, member_id) DO UPDATE SET role = excluded.role,
                                                           updated_at = excluded.updated_at",
            params![
                org_id,
                repo_id,
                member_id,
                role.as_str(),
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub(crate) fn remove_repo_role(
        &self,
        org_id: &str,
        repo_id: &str,
        member_id: &str,
    ) -> Result<bool> {
        let conn = self.conn()?;
        let n = conn.execute(
            "DELETE FROM repo_role WHERE org_id = ?1 AND repo_id = ?2 AND member_id = ?3",
            params![org_id, repo_id, member_id],
        )?;
        Ok(n > 0)
    }

    pub(crate) fn get_repo_role(
        &self,
        org_id: &str,
        repo_id: &str,
        member_id: &str,
    ) -> Result<Option<Role>> {
        let conn = self.conn()?;
        let role: Option<String> = conn
            .query_row(
                "SELECT role FROM repo_role WHERE org_id = ?1 AND repo_id = ?2 AND member_id = ?3",
                params![org_id, repo_id, member_id],
                |r| r.get(0),
            )
            .optional()?;
        role.map(|r| Role::parse(&r)).transpose()
    }

    pub(crate) fn list_repo_roles(
        &self,
        org_id: &str,
        repo_id: &str,
    ) -> Result<Vec<RepoRoleGrant>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT member_id, role, updated_at FROM repo_role
             WHERE org_id = ?1 AND repo_id = ?2 ORDER BY member_id",
        )?;
        let rows = stmt.query_map(params![org_id, repo_id], |r| {
            Ok(RepoRoleGrant {
                member_id: r.get(0)?,
                role: r.get(1)?,
                updated_at: r.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub(crate) fn list_repos(&self, org_id: &str) -> Result<Vec<RepoSummary>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT r.id, r.name, COUNT(e.seq)
             FROM repo r LEFT JOIN event e ON e.repo_id = r.id
             WHERE r.org_id = ?1
             GROUP BY r.id, r.name
             ORDER BY r.name",
        )?;
        let rows = stmt.query_map([org_id], |r| {
            Ok(RepoSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                event_count: r.get::<_, i64>(2)? as u64,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
