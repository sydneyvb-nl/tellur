//! Org policies and compliance snapshots.

use super::*;

impl PostgresStore {
    pub(crate) fn put_policy(&self, org_id: &str, name: &str, content: &str) -> Result<i64> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&advisory_key(&format!("policy:{org_id}:{name}"))],
        )?;
        let current: i64 = tx
            .query_opt(
                "SELECT version FROM policy WHERE org_id = $1 AND name = $2",
                &[&org_id, &name],
            )?
            .map(|r| r.get(0))
            .unwrap_or(0);
        let version = current + 1;
        tx.execute(
            "INSERT INTO policy (org_id, name, content, version, updated_at)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (org_id, name) DO UPDATE SET content = excluded.content,
                 version = excluded.version, updated_at = excluded.updated_at",
            &[
                &org_id,
                &name,
                &content,
                &version,
                &chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        tx.commit()?;
        Ok(version)
    }

    pub(crate) fn list_policies(&self, org_id: &str) -> Result<Vec<PolicySummary>> {
        let rows = self.client()?.query(
            "SELECT name, version, updated_at FROM policy WHERE org_id = $1 ORDER BY name",
            &[&org_id],
        )?;
        Ok(rows
            .iter()
            .map(|r| PolicySummary {
                name: r.get(0),
                version: r.get(1),
                updated_at: r.get(2),
            })
            .collect())
    }

    pub(crate) fn get_policy(&self, org_id: &str, name: &str) -> Result<Option<PolicyDoc>> {
        let row = self.client()?.query_opt(
            "SELECT name, content, version, updated_at FROM policy
             WHERE org_id = $1 AND name = $2",
            &[&org_id, &name],
        )?;
        Ok(row.map(|r| PolicyDoc {
            name: r.get(0),
            content: r.get(1),
            version: r.get(2),
            updated_at: r.get(3),
        }))
    }

    pub(crate) fn put_compliance_snapshots(
        &self,
        org_id: &str,
        snaps: &[ComplianceSnapshot],
    ) -> Result<()> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        for snap in snaps {
            tx.execute(
                "INSERT INTO compliance_snapshot
                     (id, org_id, repo_id, repo_name, policy_name, policy_version,
                      evaluated_at, ai_ranges, violations, high, medium, low)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                &[
                    &ids::generate_id("cmp"),
                    &org_id,
                    &snap.repo_id,
                    &snap.repo_name,
                    &snap.policy_name,
                    &snap.policy_version,
                    &snap.evaluated_at,
                    &snap.ai_ranges,
                    &snap.violations,
                    &snap.high,
                    &snap.medium,
                    &snap.low,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn latest_compliance(&self, org_id: &str) -> Result<Vec<ComplianceSnapshot>> {
        let rows = self.client()?.query(
            "SELECT cs.repo_id, cs.repo_name, cs.policy_name, cs.policy_version,
                    cs.evaluated_at, cs.ai_ranges, cs.violations, cs.high, cs.medium, cs.low
             FROM compliance_snapshot cs
             JOIN (
                 SELECT repo_id, MAX(evaluated_at || '|' || id) AS mk
                 FROM compliance_snapshot WHERE org_id = $1 GROUP BY repo_id
             ) latest
               ON cs.repo_id = latest.repo_id
              AND (cs.evaluated_at || '|' || cs.id) = latest.mk
             WHERE cs.org_id = $1
             ORDER BY cs.evaluated_at DESC, cs.repo_name",
            &[&org_id],
        )?;
        Ok(rows
            .iter()
            .map(|r| ComplianceSnapshot {
                repo_id: r.get(0),
                repo_name: r.get(1),
                policy_name: r.get(2),
                policy_version: r.get(3),
                evaluated_at: r.get(4),
                ai_ranges: r.get(5),
                violations: r.get(6),
                high: r.get(7),
                medium: r.get(8),
                low: r.get(9),
            })
            .collect())
    }
}
