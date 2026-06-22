//! SCIM 2.0 provisioning: users, groups, and SCIM tokens.

use super::*;

impl SqliteStore {
    pub(crate) fn scim_token_created_at(&self, org_id: &str) -> Result<Option<String>> {
        let conn = self.conn()?;
        let ts = conn
            .query_row(
                "SELECT created_at FROM scim_token WHERE org_id = ?1
                 ORDER BY created_at DESC LIMIT 1",
                [org_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(ts)
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
        let mut guard = self.conn()?;
        let tx = guard.transaction()?;
        tx.execute(
            "INSERT INTO scim_group (id, org_id, display_name, external_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, org_id, display_name, external_id, now],
        )
        .context("failed to create group (duplicate displayName?)")?;
        let members = set_group_members(&tx, org_id, &id, members)?;
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
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, display_name, external_id FROM scim_group
             WHERE org_id = ?1 AND (?2 IS NULL OR display_name = ?2) ORDER BY display_name",
        )?;
        let rows: Vec<(String, String, Option<String>)> = stmt
            .query_map(params![org_id, name_filter], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .filter_map(std::result::Result::ok)
            .collect();
        let mut out = Vec::new();
        for (id, display_name, external_id) in rows {
            let members = group_member_ids(&conn, &id)?;
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
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT display_name, external_id FROM scim_group WHERE org_id = ?1 AND id = ?2",
                params![org_id, group_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        match row {
            Some((display_name, external_id)) => {
                let members = group_member_ids(&conn, group_id)?;
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
            let mut guard = self.conn()?;
            let tx = guard.transaction()?;
            let exists = tx
                .query_row(
                    "SELECT 1 FROM scim_group WHERE org_id = ?1 AND id = ?2",
                    params![org_id, group_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !exists {
                return Ok(None);
            }
            if let Some(name) = display_name {
                tx.execute(
                    "UPDATE scim_group SET display_name = ?2 WHERE id = ?1",
                    params![group_id, name],
                )?;
            }
            if let Some(ext) = external_id {
                tx.execute(
                    "UPDATE scim_group SET external_id = ?2 WHERE id = ?1",
                    params![group_id, ext],
                )?;
            }
            if let Some(new_members) = members {
                // Recompute the union of old + new members after the swap.
                let old = group_member_ids(&tx, group_id)?;
                tx.execute(
                    "DELETE FROM scim_group_member WHERE group_id = ?1",
                    params![group_id],
                )?;
                set_group_members(&tx, org_id, group_id, new_members)?;
                let mut affected: std::collections::BTreeSet<String> = old.into_iter().collect();
                affected.extend(new_members.iter().cloned());
                for m in affected {
                    recompute_member_role(&tx, &m)?;
                }
            } else if display_name.is_some() {
                // Renaming may change the group's role mapping.
                for m in group_member_ids(&tx, group_id)? {
                    recompute_member_role(&tx, &m)?;
                }
            }
            tx.execute(
                "UPDATE scim_group SET updated_at = ?2 WHERE id = ?1",
                params![group_id, chrono::Utc::now().to_rfc3339()],
            )?;
            tx.commit()?;
        }
        self.scim_get_group(org_id, group_id)
    }


    pub(crate) fn scim_delete_group(&self, org_id: &str, group_id: &str) -> Result<bool> {
        let mut guard = self.conn()?;
        let tx = guard.transaction()?;
        let exists = tx
            .query_row(
                "SELECT 1 FROM scim_group WHERE org_id = ?1 AND id = ?2",
                params![org_id, group_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Ok(false);
        }
        let members = group_member_ids(&tx, group_id)?;
        tx.execute(
            "DELETE FROM scim_group_member WHERE group_id = ?1",
            params![group_id],
        )?;
        tx.execute("DELETE FROM scim_group WHERE id = ?1", params![group_id])?;
        for m in members {
            recompute_member_role(&tx, &m)?;
        }
        tx.commit()?;
        Ok(true)
    }


    pub(crate) fn create_scim_token(&self, org_id: &str) -> Result<GeneratedToken> {
        let token = auth::generate_token()?;
        self.conn()?
            .execute(
                "INSERT INTO scim_token (token_id, org_id, secret_hash, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    token.token_id,
                    org_id,
                    token.secret_hash,
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .context("failed to create SCIM token (does the org exist?)")?;
        Ok(token)
    }


    pub(crate) fn authenticate_scim(&self, token: &str) -> Result<Option<String>> {
        let Some((token_id, secret)) = auth::parse_token(token) else {
            return Ok(None);
        };
        let row: Option<(String, String)> = {
            let conn = self.conn()?;
            conn.query_row(
                "SELECT secret_hash, org_id FROM scim_token WHERE token_id = ?1",
                [token_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .context("SCIM token lookup failed")?
        };
        let Some((secret_hash, org_id)) = row else {
            return Ok(None);
        };
        if !auth::verify_secret(&secret, &secret_hash) {
            return Ok(None);
        }
        Ok(Some(org_id))
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
        let mut guard = self.conn()?;
        let tx = guard.transaction()?;
        tx.execute(
            "INSERT INTO member (id, org_id, display_name, role, created_at, active)
             VALUES (?1, ?2, ?3, ?4, ?5, 1)",
            params![
                member_id,
                org_id,
                display_name,
                role.as_str(),
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .context("failed to create member")?;
        tx.execute(
            "INSERT INTO member_identity (member_id, email, external_id) VALUES (?1, ?2, ?3)",
            params![member_id, email, external_id],
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
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT m.id, m.display_name, m.role, m.active, i.email, i.external_id
             FROM member m JOIN member_identity i ON i.member_id = m.id
             WHERE m.org_id = ?1 AND (?2 IS NULL OR i.email = ?2)
             ORDER BY i.email",
        )?;
        let rows = stmt.query_map(params![org_id, email_filter], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (member_id, display_name, role, active, email, external_id) = row?;
            out.push(ScimUser {
                member_id,
                email,
                display_name,
                role: Role::parse(&role)?,
                active: active != 0,
                external_id,
            });
        }
        Ok(out)
    }


    pub(crate) fn scim_get_user(&self, org_id: &str, member_id: &str) -> Result<Option<ScimUser>> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT m.display_name, m.role, m.active, i.email, i.external_id
                 FROM member m JOIN member_identity i ON i.member_id = m.id
                 WHERE m.org_id = ?1 AND m.id = ?2",
                params![org_id, member_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()?;
        match row {
            Some((display_name, role, active, email, external_id)) => Ok(Some(ScimUser {
                member_id: member_id.to_string(),
                email,
                display_name,
                role: Role::parse(&role)?,
                active: active != 0,
                external_id,
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
            let conn = self.conn()?;
            // Verify the member exists in this org first (tenant scoping).
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM member WHERE id = ?1 AND org_id = ?2",
                    params![member_id, org_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !exists {
                return Ok(None);
            }
            if let Some(addr) = email {
                conn.execute(
                    "UPDATE member_identity SET email = ?2 WHERE member_id = ?1",
                    params![member_id, addr],
                )?;
            }
            if let Some(name) = display_name {
                conn.execute(
                    "UPDATE member SET display_name = ?2 WHERE id = ?1",
                    params![member_id, name],
                )?;
            }
            if let Some(r) = role {
                conn.execute(
                    "UPDATE member SET role = ?2 WHERE id = ?1",
                    params![member_id, r.as_str()],
                )?;
            }
            if let Some(a) = active {
                conn.execute(
                    "UPDATE member SET active = ?2 WHERE id = ?1",
                    params![member_id, a as i64],
                )?;
            }
            if let Some(ext) = external_id {
                conn.execute(
                    "UPDATE member_identity SET external_id = ?2 WHERE member_id = ?1",
                    params![member_id, ext],
                )?;
            }
        }
        self.scim_get_user(org_id, member_id)
    }
}
