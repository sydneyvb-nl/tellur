//! Durable background-job queue.

use super::*;

impl SqliteStore {
    pub(crate) fn prune_finished_jobs(&self, older_than_rfc3339: &str) -> Result<u64> {
        let n = self.conn()?.execute(
            "DELETE FROM job
             WHERE status IN ('completed', 'failed') AND updated_at < ?1",
            params![older_than_rfc3339],
        )?;
        Ok(n as u64)
    }


    pub(crate) fn enqueue_job(&self, org_id: &str, kind: &str, job_params: Option<&str>) -> Result<String> {
        let id = ids::generate_id("job");
        let now = chrono::Utc::now().to_rfc3339();
        self.conn()?
            .execute(
                "INSERT INTO job (id, org_id, kind, status, params, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?5)",
                params![id, org_id, kind, job_params, now],
            )
            .context("failed to enqueue job")?;
        Ok(id)
    }


    pub(crate) fn claim_next_job(&self) -> Result<Option<Job>> {
        let mut guard = self.conn()?;
        // IMMEDIATE so the select-then-update is atomic against other workers.
        let tx = guard.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = tx
            .query_row(
                "SELECT id, org_id, kind, params, created_at FROM job
                 WHERE status = 'queued' ORDER BY created_at ASC, id ASC LIMIT 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((id, org_id, kind, job_params, created_at)) = row else {
            return Ok(None);
        };
        let now = chrono::Utc::now().to_rfc3339();
        tx.execute(
            "UPDATE job SET status = 'running', updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        tx.commit()?;
        Ok(Some(Job {
            id,
            org_id,
            kind,
            status: "running".to_string(),
            result: None,
            error: None,
            params: job_params,
            created_at,
            updated_at: now,
        }))
    }


    pub(crate) fn complete_job(&self, job_id: &str, result_json: &str) -> Result<()> {
        self.conn()?.execute(
            "UPDATE job SET status = 'completed', result = ?2, updated_at = ?3 WHERE id = ?1",
            params![job_id, result_json, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }


    pub(crate) fn fail_job(&self, job_id: &str, error: &str) -> Result<()> {
        self.conn()?.execute(
            "UPDATE job SET status = 'failed', error = ?2, updated_at = ?3 WHERE id = ?1",
            params![job_id, error, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }


    pub(crate) fn get_job(&self, org_id: &str, job_id: &str) -> Result<Option<Job>> {
        let conn = self.conn()?;
        let row = conn
            .query_row(
                "SELECT id, org_id, kind, status, result, error, params, created_at, updated_at
                 FROM job WHERE org_id = ?1 AND id = ?2",
                params![org_id, job_id],
                |r| {
                    Ok(Job {
                        id: r.get(0)?,
                        org_id: r.get(1)?,
                        kind: r.get(2)?,
                        status: r.get(3)?,
                        result: r.get(4)?,
                        error: r.get(5)?,
                        params: r.get(6)?,
                        created_at: r.get(7)?,
                        updated_at: r.get(8)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }


    pub(crate) fn list_jobs(&self, org_id: &str, limit: u32) -> Result<Vec<Job>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, org_id, kind, status, result, error, params, created_at, updated_at
             FROM job WHERE org_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![org_id, limit as i64], |r| {
            Ok(Job {
                id: r.get(0)?,
                org_id: r.get(1)?,
                kind: r.get(2)?,
                status: r.get(3)?,
                result: r.get(4)?,
                error: r.get(5)?,
                params: r.get(6)?,
                created_at: r.get(7)?,
                updated_at: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }


    pub(crate) fn requeue_running_jobs(&self) -> Result<u64> {
        let n = self.conn()?.execute(
            "UPDATE job SET status = 'queued', updated_at = ?1 WHERE status = 'running'",
            params![chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(n as u64)
    }

}
