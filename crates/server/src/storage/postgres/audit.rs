//! Audit log: append, export, and tamper-evident verification.

use super::*;

impl PostgresStore {
    pub(crate) fn export_audit(&self, org_id: &str) -> Result<Vec<AuditRecord>> {
        let rows = self.client()?.query(
            "SELECT seq, ts, org_id, actor_member_id, action, detail, entry_hash
             FROM audit_log WHERE org_id = $1 ORDER BY seq ASC",
            &[&org_id],
        )?;
        Ok(rows
            .iter()
            .map(|r| AuditRecord {
                seq: r.get(0),
                ts: r.get(1),
                org_id: r.get(2),
                actor_member_id: r.get(3),
                action: r.get(4),
                detail: r.get(5),
                entry_hash: r.get(6),
            })
            .collect())
    }


    pub(crate) fn list_audit(
        &self,
        org_id: &str,
        actor: Option<&str>,
        action: Option<&str>,
        since_rfc3339: Option<&str>,
        before_seq: Option<i64>,
        limit: u32,
    ) -> Result<Vec<AuditRecord>> {
        // Dynamic filter; every clause is a bound parameter (no injection
        // surface). `org_id = $1` keeps it tenant-scoped (NULL-org rows excluded).
        let org = org_id.to_string();
        let lim = limit as i64;
        let mut sql = String::from(
            "SELECT seq, ts, org_id, actor_member_id, action, detail, entry_hash
             FROM audit_log WHERE org_id = $1",
        );
        let mut params: Vec<Box<dyn postgres::types::ToSql + Sync>> = vec![Box::new(org)];
        if let Some(a) = actor {
            params.push(Box::new(a.to_string()));
            sql.push_str(&format!(" AND actor_member_id = ${}", params.len()));
        }
        if let Some(a) = action {
            params.push(Box::new(a.to_string()));
            sql.push_str(&format!(" AND action = ${}", params.len()));
        }
        if let Some(s) = since_rfc3339 {
            params.push(Box::new(s.to_string()));
            sql.push_str(&format!(" AND ts >= ${}", params.len()));
        }
        if let Some(c) = before_seq {
            params.push(Box::new(c));
            sql.push_str(&format!(" AND seq < ${}", params.len()));
        }
        params.push(Box::new(lim));
        sql.push_str(&format!(" ORDER BY seq DESC LIMIT ${}", params.len()));

        let refs: Vec<&(dyn postgres::types::ToSql + Sync)> =
            params.iter().map(|b| b.as_ref()).collect();
        let rows = self.client()?.query(&sql, refs.as_slice())?;
        Ok(rows
            .iter()
            .map(|r| AuditRecord {
                seq: r.get(0),
                ts: r.get(1),
                org_id: r.get(2),
                actor_member_id: r.get(3),
                action: r.get(4),
                detail: r.get(5),
                entry_hash: r.get(6),
            })
            .collect())
    }


    pub(crate) fn append_audit(&self, entry: &AuditEntry) -> Result<()> {
        let ts = chrono::Utc::now().to_rfc3339();
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&advisory_key("audit")],
        )?;
        let (prev, count) = read_head(
            &mut tx,
            "SELECT head_hash, entry_count FROM audit_head WHERE id = $1",
            &1i32,
        )?;
        let entry_hash = audit_hash(
            &prev,
            &ts,
            entry.org_id.as_deref(),
            entry.actor_member_id.as_deref(),
            &entry.action,
            &entry.detail,
        );
        tx.execute(
            "INSERT INTO audit_log
                 (ts, org_id, actor_member_id, action, detail, prev_hash, entry_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &ts,
                &entry.org_id,
                &entry.actor_member_id,
                &entry.action,
                &entry.detail,
                &prev,
                &entry_hash,
            ],
        )
        .context("failed to append audit entry")?;
        tx.execute(
            "INSERT INTO audit_head (id, head_hash, entry_count) VALUES (1, $1, $2)
             ON CONFLICT (id) DO UPDATE SET head_hash = excluded.head_hash,
                 entry_count = excluded.entry_count",
            &[&entry_hash, &(count + 1)],
        )?;
        tx.commit().context("failed to commit audit entry")?;
        Ok(())
    }


    pub(crate) fn audit_len(&self) -> Result<u64> {
        let n: i64 = self
            .client()?
            .query_one("SELECT COUNT(*) FROM audit_log", &[])?
            .get(0);
        Ok(n as u64)
    }


    pub(crate) fn verify_audit_chain(&self) -> Result<bool> {
        let mut client = self.client()?;
        // Seed from the sealed checkpoint (genesis `("", 0)` when nothing sealed).
        let (sealed_hash, sealed_count): (String, i64) = match client.query_opt(
            "SELECT sealed_hash, sealed_count FROM audit_head WHERE id = 1",
            &[],
        )? {
            Some(r) => (r.get(0), r.get(1)),
            None => (String::new(), 0),
        };
        let rows = client.query(
            "SELECT ts, org_id, actor_member_id, action, detail, prev_hash, entry_hash
             FROM audit_log ORDER BY seq ASC",
            &[],
        )?;
        let mut expected_prev = sealed_hash;
        let mut counted: i64 = sealed_count;
        for r in &rows {
            let prev_hash: String = r.get(5);
            let entry_hash: String = r.get(6);
            if prev_hash != expected_prev {
                return Ok(false);
            }
            let org_id: Option<String> = r.get(1);
            let actor: Option<String> = r.get(2);
            let recomputed = audit_hash(
                &prev_hash,
                &r.get::<_, String>(0),
                org_id.as_deref(),
                actor.as_deref(),
                &r.get::<_, String>(3),
                &r.get::<_, String>(4),
            );
            if recomputed != entry_hash {
                return Ok(false);
            }
            expected_prev = entry_hash;
            counted += 1;
        }
        let head = client.query_opt(
            "SELECT head_hash, entry_count FROM audit_head WHERE id = 1",
            &[],
        )?;
        match head {
            Some(row) => {
                Ok(counted == row.get::<_, i64>(1) && expected_prev == row.get::<_, String>(0))
            }
            None => Ok(counted == 0),
        }
    }


    pub(crate) fn seal_audit_before(&self, cutoff_rfc3339: &str) -> Result<u64> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        // Take the same advisory lock as append_audit so sealing and appends are
        // serialized: otherwise a concurrent append between the entry_count read
        // and the COUNT(seq > boundary) below would skew sealed_count and make
        // verify_audit_chain report a false break.
        tx.execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&advisory_key("audit")],
        )?;

        // Newest entry older than the cutoff becomes the new checkpoint boundary.
        let boundary = tx.query_opt(
            "SELECT seq, entry_hash FROM audit_log WHERE ts < $1 ORDER BY seq DESC LIMIT 1",
            &[&cutoff_rfc3339],
        )?;
        let Some(brow) = boundary else {
            return Ok(0);
        };
        let bseq: i64 = brow.get(0);
        let bhash: String = brow.get(1);

        let entry_count: i64 = tx
            .query_one("SELECT entry_count FROM audit_head WHERE id = 1", &[])?
            .get(0);
        let retained_after: i64 = tx
            .query_one("SELECT COUNT(*) FROM audit_log WHERE seq > $1", &[&bseq])?
            .get(0);
        let sealed_count = entry_count - retained_after;

        let pruned = tx.execute("DELETE FROM audit_log WHERE seq <= $1", &[&bseq])?;
        tx.execute(
            "UPDATE audit_head SET sealed_hash = $1, sealed_count = $2 WHERE id = 1",
            &[&bhash, &sealed_count],
        )?;
        tx.commit()?;
        Ok(pruned)
    }

}
