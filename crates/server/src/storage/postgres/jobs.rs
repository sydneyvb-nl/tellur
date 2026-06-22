//! Durable background-job queue.

use super::*;

impl PostgresStore {
    pub(crate) fn prune_finished_jobs(&self, older_than_rfc3339: &str) -> Result<u64> {
        let n = self.client()?.execute(
            "DELETE FROM job
             WHERE status IN ('completed', 'failed') AND updated_at < $1",
            &[&older_than_rfc3339],
        )?;
        Ok(n)
    }

    pub(crate) fn enqueue_job(
        &self,
        org_id: &str,
        kind: &str,
        job_params: Option<&str>,
    ) -> Result<String> {
        let id = ids::generate_id("job");
        let now = chrono::Utc::now().to_rfc3339();
        self.client()?
            .execute(
                "INSERT INTO job (id, org_id, kind, status, params, created_at, updated_at)
                 VALUES ($1, $2, $3, 'queued', $4, $5, $5)",
                &[&id, &org_id, &kind, &job_params, &now],
            )
            .context("failed to enqueue job")?;
        Ok(id)
    }

    pub(crate) fn claim_next_job(&self) -> Result<Option<Job>> {
        // FOR UPDATE SKIP LOCKED lets multiple workers claim distinct jobs.
        let now = chrono::Utc::now().to_rfc3339();
        let row = self.client()?.query_opt(
            "UPDATE job SET status = 'running', updated_at = $1
             WHERE id = (
                 SELECT id FROM job WHERE status = 'queued'
                 ORDER BY created_at ASC, id ASC
                 FOR UPDATE SKIP LOCKED LIMIT 1
             )
             RETURNING id, org_id, kind, status, result, error, params, created_at, updated_at",
            &[&now],
        )?;
        Ok(row.map(|r| Job {
            id: r.get(0),
            org_id: r.get(1),
            kind: r.get(2),
            status: r.get(3),
            result: r.get(4),
            error: r.get(5),
            params: r.get(6),
            created_at: r.get(7),
            updated_at: r.get(8),
        }))
    }

    pub(crate) fn complete_job(&self, job_id: &str, result_json: &str) -> Result<()> {
        self.client()?.execute(
            "UPDATE job SET status = 'completed', result = $2, updated_at = $3 WHERE id = $1",
            &[&job_id, &result_json, &chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub(crate) fn fail_job(&self, job_id: &str, error: &str) -> Result<()> {
        self.client()?.execute(
            "UPDATE job SET status = 'failed', error = $2, updated_at = $3 WHERE id = $1",
            &[&job_id, &error, &chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub(crate) fn get_job(&self, org_id: &str, job_id: &str) -> Result<Option<Job>> {
        let row = self.client()?.query_opt(
            "SELECT id, org_id, kind, status, result, error, params, created_at, updated_at
             FROM job WHERE org_id = $1 AND id = $2",
            &[&org_id, &job_id],
        )?;
        Ok(row.map(|r| Job {
            id: r.get(0),
            org_id: r.get(1),
            kind: r.get(2),
            status: r.get(3),
            result: r.get(4),
            error: r.get(5),
            params: r.get(6),
            created_at: r.get(7),
            updated_at: r.get(8),
        }))
    }

    pub(crate) fn list_jobs(&self, org_id: &str, limit: u32) -> Result<Vec<Job>> {
        let lim = limit as i64;
        let rows = self.client()?.query(
            "SELECT id, org_id, kind, status, result, error, params, created_at, updated_at
             FROM job WHERE org_id = $1 ORDER BY created_at DESC, id DESC LIMIT $2",
            &[&org_id, &lim],
        )?;
        Ok(rows
            .iter()
            .map(|r| Job {
                id: r.get(0),
                org_id: r.get(1),
                kind: r.get(2),
                status: r.get(3),
                result: r.get(4),
                error: r.get(5),
                params: r.get(6),
                created_at: r.get(7),
                updated_at: r.get(8),
            })
            .collect())
    }

    pub(crate) fn requeue_running_jobs(&self) -> Result<u64> {
        let n = self.client()?.execute(
            "UPDATE job SET status = 'queued', updated_at = $1 WHERE status = 'running'",
            &[&chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(n)
    }
}
