//! Login transactions, browser sessions, and device-authorization.

use super::*;

impl PostgresStore {
    pub(crate) fn put_login(
        &self,
        state: &str,
        pkce_verifier: &str,
        nonce: &str,
        browser_binding: &str,
    ) -> Result<()> {
        self.client()?.execute(
            "INSERT INTO oidc_login (state, pkce_verifier, nonce, browser_binding, created_at)
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &state,
                &pkce_verifier,
                &nonce,
                &browser_binding,
                &chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }


    pub(crate) fn count_logins(&self) -> Result<u64> {
        let n: i64 = self
            .client()?
            .query_one("SELECT COUNT(*) FROM oidc_login", &[])?
            .get(0);
        Ok(n as u64)
    }


    pub(crate) fn prune_expired_logins(&self, ttl_secs: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(ttl_secs)).to_rfc3339();
        let n = self
            .client()?
            .execute("DELETE FROM oidc_login WHERE created_at < $1", &[&cutoff])?;
        Ok(n)
    }


    pub(crate) fn take_login(&self, state: &str) -> Result<Option<LoginTx>> {
        let row = self.client()?.query_opt(
            "DELETE FROM oidc_login WHERE state = $1
             RETURNING pkce_verifier, nonce, browser_binding, created_at",
            &[&state],
        )?;
        Ok(row.map(|r| LoginTx {
            pkce_verifier: r.get(0),
            nonce: r.get(1),
            browser_binding: r.get(2),
            created_at: r.get(3),
        }))
    }


    pub(crate) fn create_session(&self, member_id: &str, ttl_secs: i64) -> Result<String> {
        let id = ids::generate_id("sess");
        let now = chrono::Utc::now();
        let expires = now + chrono::Duration::seconds(ttl_secs);
        self.client()?.execute(
            "INSERT INTO session (id, member_id, created_at, expires_at)
             VALUES ($1, $2, $3, $4)",
            &[&id, &member_id, &now.to_rfc3339(), &expires.to_rfc3339()],
        )?;
        Ok(id)
    }


    pub(crate) fn session_principal(&self, session_id: &str) -> Result<Option<Principal>> {
        let now = chrono::Utc::now().to_rfc3339();
        let row = self.client()?.query_opt(
            "SELECT m.id, m.org_id, m.role FROM session s
             JOIN member m ON m.id = s.member_id
             WHERE s.id = $1 AND s.expires_at > $2 AND m.active = TRUE",
            &[&session_id, &now],
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


    pub(crate) fn delete_session(&self, session_id: &str) -> Result<bool> {
        let n = self
            .client()?
            .execute("DELETE FROM session WHERE id = $1", &[&session_id])?;
        Ok(n > 0)
    }


    pub(crate) fn create_device_auth(&self, device_code: &str, user_code: &str) -> Result<()> {
        self.client()?.execute(
            "INSERT INTO device_auth (device_code, user_code, status, created_at)
             VALUES ($1, $2, 'pending', $3)",
            &[&device_code, &user_code, &chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }


    pub(crate) fn count_device_auths(&self) -> Result<u64> {
        let n: i64 = self
            .client()?
            .query_one("SELECT COUNT(*) FROM device_auth", &[])?
            .get(0);
        Ok(n as u64)
    }


    pub(crate) fn prune_expired_device_auths(&self, ttl_secs: i64) -> Result<u64> {
        let cutoff = (chrono::Utc::now() - chrono::Duration::seconds(ttl_secs)).to_rfc3339();
        let n = self
            .client()?
            .execute("DELETE FROM device_auth WHERE created_at < $1", &[&cutoff])?;
        Ok(n)
    }


    pub(crate) fn find_device_by_user_code(&self, user_code: &str) -> Result<Option<DeviceAuth>> {
        let row = self.client()?.query_opt(
            "SELECT user_code, status, member_id, created_at
             FROM device_auth WHERE user_code = $1",
            &[&user_code],
        )?;
        Ok(row.map(|r| DeviceAuth {
            user_code: r.get(0),
            status: r.get(1),
            member_id: r.get(2),
            created_at: r.get(3),
        }))
    }


    pub(crate) fn set_device_decision(&self, user_code: &str, member_id: &str, approve: bool) -> Result<bool> {
        let status = if approve { "approved" } else { "denied" };
        let n = self.client()?.execute(
            "UPDATE device_auth SET status = $2, member_id = $3
             WHERE user_code = $1 AND status = 'pending'",
            &[&user_code, &status, &member_id],
        )?;
        Ok(n > 0)
    }


    pub(crate) fn poll_device(&self, device_code: &str, ttl_secs: i64) -> Result<DevicePoll> {
        // A short transaction with an advisory lock serializes the read +
        // terminal delete so an approved token is delivered at most once.
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtext($1))",
            &[&device_code],
        )?;
        let row = tx.query_opt(
            "SELECT status, member_id, created_at FROM device_auth WHERE device_code = $1",
            &[&device_code],
        )?;
        let Some(row) = row else {
            return Ok(DevicePoll::NotFound);
        };
        let status: String = row.get(0);
        let member_id: Option<String> = row.get(1);
        let created_at: String = row.get(2);
        if super::device_expired(&created_at, ttl_secs) {
            tx.execute(
                "DELETE FROM device_auth WHERE device_code = $1",
                &[&device_code],
            )?;
            tx.commit()?;
            return Ok(DevicePoll::NotFound);
        }
        let outcome = match status.as_str() {
            "approved" => DevicePoll::Approved(member_id.unwrap_or_default()),
            "denied" => DevicePoll::Denied,
            _ => DevicePoll::Pending,
        };
        if !matches!(outcome, DevicePoll::Pending) {
            tx.execute(
                "DELETE FROM device_auth WHERE device_code = $1",
                &[&device_code],
            )?;
        }
        tx.commit()?;
        Ok(outcome)
    }


    pub(crate) fn prune_expired_sessions(&self) -> Result<u64> {
        let now = chrono::Utc::now().to_rfc3339();
        let n = self
            .client()?
            .execute("DELETE FROM session WHERE expires_at < $1", &[&now])?;
        Ok(n)
    }

}
