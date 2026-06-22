//! Login transactions, browser sessions, and device-authorization.

use super::*;

impl SqliteStore {
    pub(crate) fn put_login(
        &self,
        state: &str,
        pkce_verifier: &str,
        nonce: &str,
        browser_binding: &str,
    ) -> Result<()> {
        self.conn()?.execute(
            "INSERT INTO oidc_login (state, pkce_verifier, nonce, browser_binding, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                state,
                pkce_verifier,
                nonce,
                browser_binding,
                chrono::Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }


    pub(crate) fn count_logins(&self) -> Result<u64> {
        let n: i64 = self
            .conn()?
            .query_row("SELECT COUNT(*) FROM oidc_login", [], |r| r.get(0))?;
        Ok(n as u64)
    }


    pub(crate) fn prune_expired_logins(&self, ttl_secs: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(ttl_secs)).to_rfc3339();
        let n = self.conn()?.execute(
            "DELETE FROM oidc_login WHERE created_at < ?1",
            params![cutoff],
        )?;
        Ok(n as u64)
    }


    pub(crate) fn take_login(&self, state: &str) -> Result<Option<LoginTx>> {
        let conn = self.conn()?;
        let tx = conn
            .query_row(
                "SELECT pkce_verifier, nonce, browser_binding, created_at
                 FROM oidc_login WHERE state = ?1",
                params![state],
                |r| {
                    Ok(LoginTx {
                        pkce_verifier: r.get(0)?,
                        nonce: r.get(1)?,
                        browser_binding: r.get(2)?,
                        created_at: r.get(3)?,
                    })
                },
            )
            .optional()?;
        if tx.is_some() {
            conn.execute("DELETE FROM oidc_login WHERE state = ?1", params![state])?;
        }
        Ok(tx)
    }


    pub(crate) fn create_session(&self, member_id: &str, ttl_secs: i64) -> Result<String> {
        let id = ids::generate_id("sess");
        let now = chrono::Utc::now();
        let expires = now + chrono::Duration::seconds(ttl_secs);
        self.conn()?.execute(
            "INSERT INTO session (id, member_id, created_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, member_id, now.to_rfc3339(), expires.to_rfc3339()],
        )?;
        Ok(id)
    }


    pub(crate) fn session_principal(&self, session_id: &str) -> Result<Option<Principal>> {
        let conn = self.conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        let row = conn
            .query_row(
                "SELECT m.id, m.org_id, m.role FROM session s
                 JOIN member m ON m.id = s.member_id
                 WHERE s.id = ?1 AND s.expires_at > ?2 AND m.active = 1",
                params![session_id, now],
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


    pub(crate) fn delete_session(&self, session_id: &str) -> Result<bool> {
        let n = self
            .conn()?
            .execute("DELETE FROM session WHERE id = ?1", params![session_id])?;
        Ok(n > 0)
    }


    pub(crate) fn create_device_auth(&self, device_code: &str, user_code: &str) -> Result<()> {
        self.conn()?.execute(
            "INSERT INTO device_auth (device_code, user_code, status, created_at)
             VALUES (?1, ?2, 'pending', ?3)",
            params![device_code, user_code, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }


    pub(crate) fn count_device_auths(&self) -> Result<u64> {
        let n: i64 = self
            .conn()?
            .query_row("SELECT COUNT(*) FROM device_auth", [], |r| r.get(0))?;
        Ok(n as u64)
    }


    pub(crate) fn prune_expired_device_auths(&self, ttl_secs: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(ttl_secs)).to_rfc3339();
        let n = self.conn()?.execute(
            "DELETE FROM device_auth WHERE created_at < ?1",
            params![cutoff],
        )?;
        Ok(n as u64)
    }


    pub(crate) fn find_device_by_user_code(&self, user_code: &str) -> Result<Option<DeviceAuth>> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT user_code, status, member_id, created_at
                 FROM device_auth WHERE user_code = ?1",
                params![user_code],
                |r| {
                    Ok(DeviceAuth {
                        user_code: r.get(0)?,
                        status: r.get(1)?,
                        member_id: r.get(2)?,
                        created_at: r.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }


    pub(crate) fn set_device_decision(&self, user_code: &str, member_id: &str, approve: bool) -> Result<bool> {
        // Only a still-pending request may transition, so a second click (or a
        // race) cannot flip an already-decided request.
        let status = if approve { "approved" } else { "denied" };
        let n = self.conn()?.execute(
            "UPDATE device_auth SET status = ?2, member_id = ?3
             WHERE user_code = ?1 AND status = 'pending'",
            params![user_code, status, member_id],
        )?;
        Ok(n > 0)
    }


    pub(crate) fn poll_device(&self, device_code: &str, ttl_secs: i64) -> Result<DevicePoll> {
        // Serialize the read + terminal delete so a token is delivered at most
        // once even if the CLI polls concurrently.
        let mut guard = self.conn()?;
        let tx = guard.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = tx
            .query_row(
                "SELECT status, member_id, created_at FROM device_auth WHERE device_code = ?1",
                params![device_code],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((status, member_id, created_at)) = row else {
            return Ok(DevicePoll::NotFound);
        };
        if super::device_expired(&created_at, ttl_secs) {
            tx.execute(
                "DELETE FROM device_auth WHERE device_code = ?1",
                params![device_code],
            )?;
            tx.commit()?;
            return Ok(DevicePoll::NotFound);
        }
        let outcome = match status.as_str() {
            "approved" => DevicePoll::Approved(member_id.unwrap_or_default()),
            "denied" => DevicePoll::Denied,
            _ => DevicePoll::Pending,
        };
        // Terminal outcomes consume the row (deliver-once); pending leaves it.
        if !matches!(outcome, DevicePoll::Pending) {
            tx.execute(
                "DELETE FROM device_auth WHERE device_code = ?1",
                params![device_code],
            )?;
        }
        tx.commit()?;
        Ok(outcome)
    }


    pub(crate) fn prune_expired_sessions(&self) -> Result<u64> {
        let now = chrono::Utc::now().to_rfc3339();
        let n = self
            .conn()?
            .execute("DELETE FROM session WHERE expires_at < ?1", params![now])?;
        Ok(n as u64)
    }

}
