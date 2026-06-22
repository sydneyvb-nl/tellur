//! Repos, repo sources, GitHub installations, and per-repo roles.

use super::*;

impl PostgresStore {
    pub(crate) fn ensure_repo(&self, org_id: &str, name: &str) -> Result<Repo> {
        let id = ids::generate_id("repo");
        let mut client = self.client()?;
        client
            .execute(
                "INSERT INTO repo (id, org_id, name, created_at) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (org_id, name) DO NOTHING",
                &[&id, &org_id, &name, &chrono::Utc::now().to_rfc3339()],
            )
            .context("failed to create repo")?;
        let real_id: String = client
            .query_one(
                "SELECT id FROM repo WHERE org_id = $1 AND name = $2",
                &[&org_id, &name],
            )?
            .get(0);
        Ok(Repo {
            id: real_id,
            org_id: org_id.to_string(),
            name: name.to_string(),
        })
    }

    pub(crate) fn find_repo(&self, org_id: &str, repo: &str) -> Result<Option<Repo>> {
        let mut client = self.client()?;
        let row = client.query_opt(
            "SELECT id, name FROM repo WHERE org_id = $1 AND (id = $2 OR name = $2)
             ORDER BY (id = $2) DESC LIMIT 1",
            &[&org_id, &repo],
        )?;
        Ok(row.map(|r| Repo {
            id: r.get(0),
            org_id: org_id.to_string(),
            name: r.get(1),
        }))
    }

    pub(crate) fn get_repo_source(&self, org_id: &str, repo_id: &str) -> Result<RepoSource> {
        let row = self.client()?.query_opt(
            "SELECT template, raw_template, source_token FROM repo_source
             WHERE org_id = $1 AND repo_id = $2",
            &[&org_id, &repo_id],
        )?;
        Ok(row
            .map(|r| RepoSource {
                link: r.get(0),
                raw: r.get(1),
                token: r.get(2),
            })
            .unwrap_or_default())
    }

    pub(crate) fn set_repo_source(
        &self,
        org_id: &str,
        repo_id: &str,
        link: Option<&str>,
        raw: Option<&str>,
        token: Option<&str>,
    ) -> Result<()> {
        if link.is_none() && raw.is_none() && token.is_none() {
            self.client()?.execute(
                "DELETE FROM repo_source WHERE org_id = $1 AND repo_id = $2",
                &[&org_id, &repo_id],
            )?;
        } else {
            let now = chrono::Utc::now().to_rfc3339();
            self.client()?.execute(
                "INSERT INTO repo_source (repo_id, org_id, template, raw_template, source_token, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (repo_id) DO UPDATE SET template = excluded.template,
                                                     raw_template = excluded.raw_template,
                                                     source_token = excluded.source_token,
                                                     updated_at = excluded.updated_at",
                &[&repo_id, &org_id, &link, &raw, &token, &now],
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
        let mut client = self.client()?;
        let org_exists = client
            .query_opt("SELECT 1 FROM org WHERE id = $1", &[&org_id])?
            .is_some();
        if !org_exists {
            bail!("org {org_id} not found");
        }
        client.execute(
            "INSERT INTO github_installation
                 (installation_id, org_id, account_login, updated_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (installation_id) DO UPDATE SET
                 org_id = excluded.org_id,
                 account_login = excluded.account_login,
                 updated_at = excluded.updated_at",
            &[
                &installation_id,
                &org_id,
                &account_login,
                &chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub(crate) fn github_installation(
        &self,
        installation_id: i64,
    ) -> Result<Option<GithubInstallation>> {
        let row = self.client()?.query_opt(
            "SELECT org_id, installation_id, account_login, updated_at
             FROM github_installation WHERE installation_id = $1",
            &[&installation_id],
        )?;
        Ok(row.map(|r| GithubInstallation {
            org_id: r.get(0),
            installation_id: r.get(1),
            account_login: r.get(2),
            updated_at: r.get(3),
        }))
    }

    pub(crate) fn mark_github_note_harvested(
        &self,
        org_id: &str,
        repo_id: &str,
        commit_sha: &str,
        note_sha: &str,
    ) -> Result<bool> {
        let changed = self.client()?.execute(
            "INSERT INTO github_note_harvest
                 (org_id, repo_id, commit_sha, note_sha, harvested_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (org_id, repo_id, commit_sha) DO NOTHING",
            &[
                &org_id,
                &repo_id,
                &commit_sha,
                &note_sha,
                &chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(changed == 1)
    }

    pub(crate) fn set_repo_role(
        &self,
        org_id: &str,
        repo_id: &str,
        member_id: &str,
        role: Role,
    ) -> Result<()> {
        let mut client = self.client()?;
        let repo_ok = client
            .query_opt(
                "SELECT 1 FROM repo WHERE id = $1 AND org_id = $2",
                &[&repo_id, &org_id],
            )?
            .is_some();
        if !repo_ok {
            bail!("repo {repo_id} not found in org {org_id}");
        }
        let member_ok = client
            .query_opt(
                "SELECT 1 FROM member WHERE id = $1 AND org_id = $2",
                &[&member_id, &org_id],
            )?
            .is_some();
        if !member_ok {
            bail!("member {member_id} not found in org {org_id}");
        }
        client.execute(
            "INSERT INTO repo_role (org_id, repo_id, member_id, role, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (repo_id, member_id) DO UPDATE SET role = excluded.role,
                                                            updated_at = excluded.updated_at",
            &[
                &org_id,
                &repo_id,
                &member_id,
                &role.as_str(),
                &chrono::Utc::now().to_rfc3339(),
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
        let n = self.client()?.execute(
            "DELETE FROM repo_role WHERE org_id = $1 AND repo_id = $2 AND member_id = $3",
            &[&org_id, &repo_id, &member_id],
        )?;
        Ok(n > 0)
    }

    pub(crate) fn get_repo_role(
        &self,
        org_id: &str,
        repo_id: &str,
        member_id: &str,
    ) -> Result<Option<Role>> {
        let row = self.client()?.query_opt(
            "SELECT role FROM repo_role WHERE org_id = $1 AND repo_id = $2 AND member_id = $3",
            &[&org_id, &repo_id, &member_id],
        )?;
        row.map(|r| Role::parse(&r.get::<_, String>(0))).transpose()
    }

    pub(crate) fn list_repo_roles(
        &self,
        org_id: &str,
        repo_id: &str,
    ) -> Result<Vec<RepoRoleGrant>> {
        let rows = self.client()?.query(
            "SELECT member_id, role, updated_at FROM repo_role
             WHERE org_id = $1 AND repo_id = $2 ORDER BY member_id",
            &[&org_id, &repo_id],
        )?;
        Ok(rows
            .iter()
            .map(|r| RepoRoleGrant {
                member_id: r.get(0),
                role: r.get(1),
                updated_at: r.get(2),
            })
            .collect())
    }

    pub(crate) fn list_repos(&self, org_id: &str) -> Result<Vec<RepoSummary>> {
        let rows = self.client()?.query(
            "SELECT r.id, r.name, COUNT(e.seq)
             FROM repo r LEFT JOIN event e ON e.repo_id = r.id
             WHERE r.org_id = $1 GROUP BY r.id, r.name ORDER BY r.name",
            &[&org_id],
        )?;
        Ok(rows
            .iter()
            .map(|r| RepoSummary {
                id: r.get(0),
                name: r.get(1),
                event_count: r.get::<_, i64>(2) as u64,
            })
            .collect())
    }
}
