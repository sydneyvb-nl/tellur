//! Orgs, members, tokens, and authentication.

use super::*;

impl PostgresStore {
    pub(crate) fn create_org(&self, name: &str) -> Result<Org> {
        let org = Org {
            id: ids::generate_id("org"),
            name: name.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        self.client()?
            .execute(
                "INSERT INTO org (id, name, created_at) VALUES ($1, $2, $3)",
                &[&org.id, &org.name, &org.created_at],
            )
            .context("failed to create org")?;
        Ok(org)
    }


    pub(crate) fn create_member(&self, org_id: &str, display_name: &str, role: Role) -> Result<String> {
        let member_id = ids::generate_id("mbr");
        self.client()?
            .execute(
                "INSERT INTO member (id, org_id, display_name, role, created_at)
                 VALUES ($1, $2, $3, $4, $5)",
                &[
                    &member_id,
                    &org_id,
                    &display_name,
                    &role.as_str(),
                    &chrono::Utc::now().to_rfc3339(),
                ],
            )
            .context("failed to create member (does the org exist?)")?;
        Ok(member_id)
    }


    pub(crate) fn create_token(&self, member_id: &str) -> Result<GeneratedToken> {
        let token = auth::generate_token()?;
        self.client()?
            .execute(
                "INSERT INTO api_token (token_id, member_id, secret_hash, created_at)
                 VALUES ($1, $2, $3, $4)",
                &[
                    &token.token_id,
                    &member_id,
                    &token.secret_hash,
                    &chrono::Utc::now().to_rfc3339(),
                ],
            )
            .context("failed to create token (does the member exist?)")?;
        Ok(token)
    }


    pub(crate) fn authenticate(&self, token: &str) -> Result<Option<Principal>> {
        let Some((token_id, secret)) = auth::parse_token(token) else {
            return Ok(None);
        };
        let row = self.client()?.query_opt(
            "SELECT t.secret_hash, m.id, m.org_id, m.role
             FROM api_token t JOIN member m ON m.id = t.member_id
             WHERE t.token_id = $1 AND m.active = TRUE",
            &[&token_id],
        )?;
        let Some(row) = row else { return Ok(None) };
        let secret_hash: String = row.get(0);
        if !auth::verify_secret(&secret, &secret_hash) {
            return Ok(None);
        }
        Ok(Some(Principal {
            org_id: row.get(2),
            member_id: row.get(1),
            role: Role::parse(&row.get::<_, String>(3))?,
        }))
    }


    pub(crate) fn provision_member(
        &self,
        org_id: &str,
        display_name: &str,
        role: Role,
        email: &str,
    ) -> Result<String> {
        let member_id = ids::generate_id("mbr");
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "INSERT INTO member (id, org_id, display_name, role, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &member_id,
                &org_id,
                &display_name,
                &role.as_str(),
                &chrono::Utc::now().to_rfc3339(),
            ],
        )
        .context("failed to create member (does the org exist?)")?;
        tx.execute(
            "INSERT INTO member_identity (member_id, email) VALUES ($1, $2)",
            &[&member_id, &email],
        )
        .context("failed to set member email (already in use?)")?;
        tx.commit()?;
        Ok(member_id)
    }


    pub(crate) fn find_member_by_email(&self, email: &str) -> Result<Option<Principal>> {
        principal_row(
            &mut self.client()?,
            "SELECT m.id, m.org_id, m.role FROM member m
             JOIN member_identity i ON i.member_id = m.id WHERE i.email = $1 AND m.active = TRUE",
            email,
        )
    }


    pub(crate) fn find_member_by_oidc_subject(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<Principal>> {
        let row = self.client()?.query_opt(
            "SELECT m.id, m.org_id, m.role FROM member m
             JOIN member_identity i ON i.member_id = m.id
             WHERE i.oidc_issuer = $1 AND i.oidc_subject = $2 AND m.active = TRUE",
            &[&issuer, &subject],
        )?;
        match row {
            Some(r) => Ok(Some(Principal {
                member_id: r.get(0),
                org_id: r.get(1),
                role: Role::parse(&r.get::<_, String>(2))?,
            })),
            None => Ok(None),
        }
    }


    pub(crate) fn bind_oidc_subject(&self, member_id: &str, issuer: &str, subject: &str) -> Result<bool> {
        // Only bind when no subject is set yet (see SQLite impl for rationale).
        let n = self.client()?.execute(
            "UPDATE member_identity SET oidc_issuer = $2, oidc_subject = $3
             WHERE member_id = $1 AND oidc_subject IS NULL",
            &[&member_id, &issuer, &subject],
        )?;
        Ok(n > 0)
    }


    pub(crate) fn member_principal(&self, member_id: &str) -> Result<Option<Principal>> {
        let row = self.client()?.query_opt(
            "SELECT id, org_id, role FROM member WHERE id = $1 AND active = TRUE",
            &[&member_id],
        )?;
        match row {
            Some(r) => Ok(Some(Principal {
                member_id: r.get(0),
                org_id: r.get(1),
                role: Role::parse(&r.get::<_, String>(2))?,
            })),
            None => Ok(None),
        }
    }


    pub(crate) fn list_members(&self, org_id: &str) -> Result<Vec<MemberInfo>> {
        let rows = self.client()?.query(
            "SELECT m.id, m.display_name, m.role, m.active, i.email, i.oidc_subject
             FROM member m
             LEFT JOIN member_identity i ON i.member_id = m.id
             WHERE m.org_id = $1
             ORDER BY m.display_name",
            &[&org_id],
        )?;
        Ok(rows
            .iter()
            .map(|r| {
                let oidc_subject: Option<String> = r.get(5);
                MemberInfo {
                    id: r.get(0),
                    display_name: r.get(1),
                    role: r.get(2),
                    active: r.get(3),
                    email: r.get(4),
                    sso_bound: oidc_subject.is_some(),
                }
            })
            .collect())
    }

}
