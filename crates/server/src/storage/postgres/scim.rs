//! SCIM 2.0 provisioning: users, groups, and SCIM tokens.

use super::*;

impl PostgresStore {
    pub(crate) fn scim_token_created_at(&self, org_id: &str) -> Result<Option<String>> {
        let row = self.client()?.query_opt(
            "SELECT created_at FROM scim_token WHERE org_id = $1
             ORDER BY created_at DESC LIMIT 1",
            &[&org_id],
        )?;
        Ok(row.map(|r| r.get(0)))
    }


    pub(crate) fn scim_create_group(
        &self,
        org_id: &str,
        display_name: &str,
        external_id: Option<&str>,
        members: &[String],
    ) -> Result<ScimGroup> {
        let id = ids::generate_id("grp");
        let now = chrono::Utc::now().to_rfc3339();
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "INSERT INTO scim_group (id, org_id, display_name, external_id, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $5)",
            &[&id, &org_id, &display_name, &external_id, &now],
        )
        .context("failed to create group (duplicate displayName?)")?;
        let members = pg_set_group_members(&mut tx, org_id, &id, members)?;
        tx.commit()?;
        Ok(ScimGroup {
            id,
            org_id: org_id.to_string(),
            display_name: display_name.to_string(),
            external_id: external_id.map(str::to_string),
            members,
        })
    }


    pub(crate) fn scim_list_groups(&self, org_id: &str, name_filter: Option<&str>) -> Result<Vec<ScimGroup>> {
        let mut client = self.client()?;
        let rows = client.query(
            "SELECT id, display_name, external_id FROM scim_group
             WHERE org_id = $1 AND ($2::text IS NULL OR display_name = $2) ORDER BY display_name",
            &[&org_id, &name_filter],
        )?;
        let metas: Vec<(String, String, Option<String>)> = rows
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect();
        let mut out = Vec::new();
        for (id, display_name, external_id) in metas {
            let members = pg_group_member_ids(&mut *client, &id)?;
            out.push(ScimGroup {
                id,
                org_id: org_id.to_string(),
                display_name,
                external_id,
                members,
            });
        }
        Ok(out)
    }


    pub(crate) fn scim_get_group(&self, org_id: &str, group_id: &str) -> Result<Option<ScimGroup>> {
        let mut client = self.client()?;
        let row = client.query_opt(
            "SELECT display_name, external_id FROM scim_group WHERE org_id = $1 AND id = $2",
            &[&org_id, &group_id],
        )?;
        match row {
            Some(r) => {
                let display_name: String = r.get(0);
                let external_id: Option<String> = r.get(1);
                let members = pg_group_member_ids(&mut *client, group_id)?;
                Ok(Some(ScimGroup {
                    id: group_id.to_string(),
                    org_id: org_id.to_string(),
                    display_name,
                    external_id,
                    members,
                }))
            }
            None => Ok(None),
        }
    }


    pub(crate) fn scim_update_group(
        &self,
        org_id: &str,
        group_id: &str,
        display_name: Option<&str>,
        external_id: Option<&str>,
        members: Option<&[String]>,
    ) -> Result<Option<ScimGroup>> {
        {
            let mut client = self.client()?;
            let mut tx = client.transaction()?;
            let exists = tx
                .query_opt(
                    "SELECT 1 FROM scim_group WHERE org_id = $1 AND id = $2",
                    &[&org_id, &group_id],
                )?
                .is_some();
            if !exists {
                return Ok(None);
            }
            if let Some(name) = display_name {
                tx.execute(
                    "UPDATE scim_group SET display_name = $2 WHERE id = $1",
                    &[&group_id, &name],
                )?;
            }
            if let Some(ext) = external_id {
                tx.execute(
                    "UPDATE scim_group SET external_id = $2 WHERE id = $1",
                    &[&group_id, &ext],
                )?;
            }
            if let Some(new_members) = members {
                let old = pg_group_member_ids(&mut tx, group_id)?;
                tx.execute(
                    "DELETE FROM scim_group_member WHERE group_id = $1",
                    &[&group_id],
                )?;
                pg_set_group_members(&mut tx, org_id, group_id, new_members)?;
                let mut affected: std::collections::BTreeSet<String> = old.into_iter().collect();
                affected.extend(new_members.iter().cloned());
                for m in affected {
                    pg_recompute_member_role(&mut tx, &m)?;
                }
            } else if display_name.is_some() {
                for m in pg_group_member_ids(&mut tx, group_id)? {
                    pg_recompute_member_role(&mut tx, &m)?;
                }
            }
            tx.execute(
                "UPDATE scim_group SET updated_at = $2 WHERE id = $1",
                &[&group_id, &chrono::Utc::now().to_rfc3339()],
            )?;
            tx.commit()?;
        }
        self.scim_get_group(org_id, group_id)
    }


    pub(crate) fn scim_delete_group(&self, org_id: &str, group_id: &str) -> Result<bool> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        let exists = tx
            .query_opt(
                "SELECT 1 FROM scim_group WHERE org_id = $1 AND id = $2",
                &[&org_id, &group_id],
            )?
            .is_some();
        if !exists {
            return Ok(false);
        }
        let members = pg_group_member_ids(&mut tx, group_id)?;
        tx.execute(
            "DELETE FROM scim_group_member WHERE group_id = $1",
            &[&group_id],
        )?;
        tx.execute("DELETE FROM scim_group WHERE id = $1", &[&group_id])?;
        for m in members {
            pg_recompute_member_role(&mut tx, &m)?;
        }
        tx.commit()?;
        Ok(true)
    }


    pub(crate) fn create_scim_token(&self, org_id: &str) -> Result<GeneratedToken> {
        let token = auth::generate_token()?;
        self.client()?
            .execute(
                "INSERT INTO scim_token (token_id, org_id, secret_hash, created_at)
                 VALUES ($1, $2, $3, $4)",
                &[
                    &token.token_id,
                    &org_id,
                    &token.secret_hash,
                    &chrono::Utc::now().to_rfc3339(),
                ],
            )
            .context("failed to create SCIM token (does the org exist?)")?;
        Ok(token)
    }


    pub(crate) fn authenticate_scim(&self, token: &str) -> Result<Option<String>> {
        let Some((token_id, secret)) = auth::parse_token(token) else {
            return Ok(None);
        };
        let row = self.client()?.query_opt(
            "SELECT secret_hash, org_id FROM scim_token WHERE token_id = $1",
            &[&token_id],
        )?;
        let Some(row) = row else { return Ok(None) };
        let secret_hash: String = row.get(0);
        if !auth::verify_secret(&secret, &secret_hash) {
            return Ok(None);
        }
        Ok(Some(row.get(1)))
    }


    pub(crate) fn scim_create_user(
        &self,
        org_id: &str,
        email: &str,
        display_name: &str,
        role: Role,
        external_id: Option<&str>,
    ) -> Result<ScimUser> {
        let member_id = ids::generate_id("mbr");
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "INSERT INTO member (id, org_id, display_name, role, created_at, active)
             VALUES ($1, $2, $3, $4, $5, TRUE)",
            &[
                &member_id,
                &org_id,
                &display_name,
                &role.as_str(),
                &chrono::Utc::now().to_rfc3339(),
            ],
        )
        .context("failed to create member")?;
        tx.execute(
            "INSERT INTO member_identity (member_id, email, external_id) VALUES ($1, $2, $3)",
            &[&member_id, &email, &external_id],
        )
        .context("failed to set member email (already in use?)")?;
        tx.commit()?;
        Ok(ScimUser {
            member_id,
            email: email.to_string(),
            display_name: display_name.to_string(),
            role,
            active: true,
            external_id: external_id.map(str::to_string),
        })
    }


    pub(crate) fn scim_list_users(&self, org_id: &str, email_filter: Option<&str>) -> Result<Vec<ScimUser>> {
        let rows = self.client()?.query(
            "SELECT m.id, m.display_name, m.role, m.active, i.email, i.external_id
             FROM member m JOIN member_identity i ON i.member_id = m.id
             WHERE m.org_id = $1 AND ($2::text IS NULL OR i.email = $2)
             ORDER BY i.email",
            &[&org_id, &email_filter],
        )?;
        let mut out = Vec::new();
        for r in &rows {
            out.push(ScimUser {
                member_id: r.get(0),
                display_name: r.get(1),
                role: Role::parse(&r.get::<_, String>(2))?,
                active: r.get(3),
                email: r.get(4),
                external_id: r.get(5),
            });
        }
        Ok(out)
    }


    pub(crate) fn scim_get_user(&self, org_id: &str, member_id: &str) -> Result<Option<ScimUser>> {
        let row = self.client()?.query_opt(
            "SELECT m.display_name, m.role, m.active, i.email, i.external_id
             FROM member m JOIN member_identity i ON i.member_id = m.id
             WHERE m.org_id = $1 AND m.id = $2",
            &[&org_id, &member_id],
        )?;
        match row {
            Some(r) => Ok(Some(ScimUser {
                member_id: member_id.to_string(),
                display_name: r.get(0),
                role: Role::parse(&r.get::<_, String>(1))?,
                active: r.get(2),
                email: r.get(3),
                external_id: r.get(4),
            })),
            None => Ok(None),
        }
    }


    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scim_update_user(
        &self,
        org_id: &str,
        member_id: &str,
        email: Option<&str>,
        display_name: Option<&str>,
        role: Option<Role>,
        active: Option<bool>,
        external_id: Option<&str>,
    ) -> Result<Option<ScimUser>> {
        {
            let mut client = self.client()?;
            let exists = client
                .query_opt(
                    "SELECT 1 FROM member WHERE id = $1 AND org_id = $2",
                    &[&member_id, &org_id],
                )?
                .is_some();
            if !exists {
                return Ok(None);
            }
            if let Some(addr) = email {
                client.execute(
                    "UPDATE member_identity SET email = $2 WHERE member_id = $1",
                    &[&member_id, &addr],
                )?;
            }
            if let Some(name) = display_name {
                client.execute(
                    "UPDATE member SET display_name = $2 WHERE id = $1",
                    &[&member_id, &name],
                )?;
            }
            if let Some(r) = role {
                client.execute(
                    "UPDATE member SET role = $2 WHERE id = $1",
                    &[&member_id, &r.as_str()],
                )?;
            }
            if let Some(a) = active {
                client.execute(
                    "UPDATE member SET active = $2 WHERE id = $1",
                    &[&member_id, &a],
                )?;
            }
            if let Some(ext) = external_id {
                client.execute(
                    "UPDATE member_identity SET external_id = $2 WHERE member_id = $1",
                    &[&member_id, &ext],
                )?;
            }
        }
        self.scim_get_user(org_id, member_id)
    }
}
