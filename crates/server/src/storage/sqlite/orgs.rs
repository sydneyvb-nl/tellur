//! Orgs, members, tokens, and authentication.

use super::*;

impl SqliteStore {
    pub(crate) fn create_org(&self, name: &str) -> Result<Org> {
        let org = Org {
            id: ids::generate_id("org"),
            name: name.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO org (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![org.id, org.name, org.created_at],
        )
        .context("failed to create org")?;
        Ok(org)
    }


    pub(crate) fn create_member(&self, org_id: &str, display_name: &str, role: Role) -> Result<String> {
        let member_id = ids::generate_id("mbr");
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO member (id, org_id, display_name, role, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                member_id,
                org_id,
                display_name,
                role.as_str(),
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .context("failed to create member (does the org exist?)")?;
        Ok(member_id)
    }


    pub(crate) fn create_token(&self, member_id: &str) -> Result<GeneratedToken> {
        let token = auth::generate_token()?;
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO api_token (token_id, member_id, secret_hash, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                token.token_id,
                member_id,
                token.secret_hash,
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .context("failed to create token (does the member exist?)")?;
        Ok(token)
    }


    pub(crate) fn authenticate(&self, token: &str) -> Result<Option<Principal>> {
        let Some((token_id, secret)) = auth::parse_token(token) else {
            return Ok(None);
        };

        // Look up the row, then release the DB lock *before* the (intentionally
        // expensive) Argon2 verification so it does not serialize other store
        // work (audits, readiness, other users' auth).
        let row = {
            let conn = self.conn()?;
            conn.query_row(
                "SELECT t.secret_hash, m.id, m.org_id, m.role
                 FROM api_token t JOIN member m ON m.id = t.member_id
                 WHERE t.token_id = ?1 AND m.active = 1",
                [token_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .context("token lookup failed")?
        };

        let Some((secret_hash, member_id, org_id, role_str)) = row else {
            return Ok(None);
        };
        if !auth::verify_secret(&secret, &secret_hash) {
            return Ok(None);
        }
        Ok(Some(Principal {
            org_id,
            member_id,
            role: Role::parse(&role_str)?,
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
        let mut guard = self.conn()?;
        // Atomic: a failed identity insert (e.g. duplicate email) must roll back
        // the member row, so we never leave a half-provisioned account.
        let tx = guard.transaction()?;
        tx.execute(
            "INSERT INTO member (id, org_id, display_name, role, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                member_id,
                org_id,
                display_name,
                role.as_str(),
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .context("failed to create member (does the org exist?)")?;
        tx.execute(
            "INSERT INTO member_identity (member_id, email) VALUES (?1, ?2)",
            params![member_id, email],
        )
        .context("failed to set member email (already in use?)")?;
        tx.commit()?;
        Ok(member_id)
    }


    pub(crate) fn find_member_by_email(&self, email: &str) -> Result<Option<Principal>> {
        let conn = self.conn()?;
        principal_row(
            &conn,
            "SELECT m.id, m.org_id, m.role FROM member m
             JOIN member_identity i ON i.member_id = m.id WHERE i.email = ?1 AND m.active = 1",
            email,
        )
    }


    pub(crate) fn find_member_by_oidc_subject(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<Principal>> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT m.id, m.org_id, m.role FROM member m
                 JOIN member_identity i ON i.member_id = m.id
                 WHERE i.oidc_issuer = ?1 AND i.oidc_subject = ?2 AND m.active = 1",
                params![issuer, subject],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        match row {
            Some((member_id, org_id, role)) => Ok(Some(Principal {
                org_id,
                member_id,
                role: Role::parse(&role)?,
            })),
            None => Ok(None),
        }
    }


    pub(crate) fn bind_oidc_subject(&self, member_id: &str, issuer: &str, subject: &str) -> Result<bool> {
        // Only bind when no subject is set yet; never overwrite an existing
        // binding (that would let a different IdP account on the same email take
        // over the member).
        let n = self.conn()?.execute(
            "UPDATE member_identity SET oidc_issuer = ?2, oidc_subject = ?3
             WHERE member_id = ?1 AND oidc_subject IS NULL",
            params![member_id, issuer, subject],
        )?;
        Ok(n > 0)
    }


    pub(crate) fn member_principal(&self, member_id: &str) -> Result<Option<Principal>> {
        let conn = self.conn()?;
        principal_row(
            &conn,
            "SELECT id, org_id, role FROM member WHERE id = ?1 AND active = 1",
            member_id,
        )
    }


    pub(crate) fn list_members(&self, org_id: &str) -> Result<Vec<MemberInfo>> {
        let conn = self.conn()?;
        // LEFT JOIN so members without an SSO identity (e.g. CLI-created) still
        // appear; `sso_bound` is whether a verified OIDC subject is bound.
        let mut stmt = conn.prepare(
            "SELECT m.id, m.display_name, m.role, m.active, i.email, i.oidc_subject
             FROM member m
             LEFT JOIN member_identity i ON i.member_id = m.id
             WHERE m.org_id = ?1
             ORDER BY m.display_name",
        )?;
        let rows = stmt.query_map([org_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, display_name, role, active, email, oidc_subject) = row?;
            out.push(MemberInfo {
                id,
                display_name,
                role,
                email,
                sso_bound: oidc_subject.is_some(),
                active: active != 0,
            });
        }
        Ok(out)
    }

}
