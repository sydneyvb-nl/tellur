//! Audit log: append, export, and tamper-evident verification.

use super::*;

impl SqliteStore {
    pub(crate) fn export_audit(&self, org_id: &str) -> Result<Vec<AuditRecord>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT seq, ts, org_id, actor_member_id, action, detail, entry_hash
             FROM audit_log WHERE org_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map([org_id], |r| {
            Ok(AuditRecord {
                seq: r.get(0)?,
                ts: r.get(1)?,
                org_id: r.get(2)?,
                actor_member_id: r.get(3)?,
                action: r.get(4)?,
                detail: r.get(5)?,
                entry_hash: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
        let conn = self.conn()?;
        // Build the filter dynamically; every clause is a bound parameter so
        // there is no injection surface. `org_id = ?` keeps it tenant-scoped
        // (rows with a NULL org — e.g. pre-auth denials — are never returned).
        let mut sql = String::from(
            "SELECT seq, ts, org_id, actor_member_id, action, detail, entry_hash
             FROM audit_log WHERE org_id = ?1",
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(org_id.to_string())];
        if let Some(a) = actor {
            params.push(Box::new(a.to_string()));
            sql.push_str(&format!(" AND actor_member_id = ?{}", params.len()));
        }
        if let Some(a) = action {
            params.push(Box::new(a.to_string()));
            sql.push_str(&format!(" AND action = ?{}", params.len()));
        }
        if let Some(s) = since_rfc3339 {
            params.push(Box::new(s.to_string()));
            sql.push_str(&format!(" AND ts >= ?{}", params.len()));
        }
        if let Some(c) = before_seq {
            params.push(Box::new(c));
            sql.push_str(&format!(" AND seq < ?{}", params.len()));
        }
        params.push(Box::new(limit as i64));
        sql.push_str(&format!(" ORDER BY seq DESC LIMIT ?{}", params.len()));

        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let rows = stmt.query_map(refs.as_slice(), |r| {
            Ok(AuditRecord {
                seq: r.get(0)?,
                ts: r.get(1)?,
                org_id: r.get(2)?,
                actor_member_id: r.get(3)?,
                action: r.get(4)?,
                detail: r.get(5)?,
                entry_hash: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub(crate) fn append_audit(&self, entry: &AuditEntry) -> Result<()> {
        let ts = chrono::Utc::now().to_rfc3339();
        let mut guard = self.conn()?;
        // IMMEDIATE acquires the write lock up front, so the read of the current
        // head and the insert are atomic even across separate connections to the
        // same database (e.g. the server and the admin CLI). Without this, two
        // writers could read the same head and create siblings with identical
        // `prev_hash`, which would later look like tampering.
        let tx = guard
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to begin audit transaction")?;

        let audit_key: i64 = 1;
        let head = chain::HeadRef {
            table: "audit_head",
            key_col: "id",
            key: &audit_key,
        };
        let (prev, count) = chain::read_head(&tx, &head)?;

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
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                ts,
                entry.org_id,
                entry.actor_member_id,
                entry.action,
                entry.detail,
                prev,
                entry_hash
            ],
        )
        .context("failed to append audit entry")?;
        chain::write_head(&tx, &head, &entry_hash, count + 1)?;
        tx.commit().context("failed to commit audit entry")?;
        Ok(())
    }

    pub(crate) fn audit_len(&self) -> Result<u64> {
        let conn = self.conn()?;
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM audit_log", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    pub(crate) fn verify_audit_chain(&self) -> Result<bool> {
        let conn = self.conn()?;
        let audit_key: i64 = 1;
        let head = chain::HeadRef {
            table: "audit_head",
            key_col: "id",
            key: &audit_key,
        };
        // Seed the walk from the sealed checkpoint (genesis when nothing sealed).
        let (sealed_hash, sealed_count) = audit_checkpoint(&conn)?;
        chain::verify(
            &conn,
            "SELECT ts, org_id, actor_member_id, action, detail, prev_hash, entry_hash
             FROM audit_log ORDER BY seq ASC",
            params![],
            &head,
            (sealed_hash.as_str(), sealed_count),
            |r| {
                let ts: String = r.get(0)?;
                let org_id: Option<String> = r.get(1)?;
                let actor: Option<String> = r.get(2)?;
                let action: String = r.get(3)?;
                let detail: String = r.get(4)?;
                let prev_hash: String = r.get(5)?;
                let entry_hash: String = r.get(6)?;
                let recomputed = audit_hash(
                    &prev_hash,
                    &ts,
                    org_id.as_deref(),
                    actor.as_deref(),
                    &action,
                    &detail,
                );
                Ok((prev_hash, entry_hash, recomputed))
            },
        )
    }

    pub(crate) fn seal_audit_before(&self, cutoff_rfc3339: &str) -> Result<u64> {
        let mut guard = self.conn()?;
        // IMMEDIATE takes the write lock up front, serializing with append_audit
        // (also IMMEDIATE) so the entry_count read and the boundary count below
        // can't race a concurrent append and skew sealed_count.
        let tx = guard.transaction_with_behavior(TransactionBehavior::Immediate)?;

        // Newest entry older than the cutoff becomes the new checkpoint boundary.
        let boundary: Option<(i64, String)> = tx
            .query_row(
                "SELECT seq, entry_hash FROM audit_log WHERE ts < ?1
                 ORDER BY seq DESC LIMIT 1",
                params![cutoff_rfc3339],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((bseq, bhash)) = boundary else {
            return Ok(0);
        };

        // entry_count is the chain length (monotonic, survives pruning).
        let entry_count: i64 =
            tx.query_row("SELECT entry_count FROM audit_head WHERE id = 1", [], |r| {
                r.get(0)
            })?;
        let retained_after: i64 = tx.query_row(
            "SELECT COUNT(*) FROM audit_log WHERE seq > ?1",
            params![bseq],
            |r| r.get(0),
        )?;
        let sealed_count = entry_count - retained_after;

        let pruned = tx.execute("DELETE FROM audit_log WHERE seq <= ?1", params![bseq])?;
        tx.execute(
            "UPDATE audit_head SET sealed_hash = ?1, sealed_count = ?2 WHERE id = 1",
            params![bhash, sealed_count],
        )?;
        tx.commit()?;
        Ok(pruned as u64)
    }
}
