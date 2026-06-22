//! Org policies and compliance snapshots.

use super::*;

impl SqliteStore {
    pub(crate) fn put_policy(&self, org_id: &str, name: &str, content: &str) -> Result<i64> {
        let mut guard = self.conn()?;
        let tx = guard
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to begin policy transaction")?;
        let current: i64 = tx
            .query_row(
                "SELECT version FROM policy WHERE org_id = ?1 AND name = ?2",
                params![org_id, name],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let version = current + 1;
        tx.execute(
            "INSERT INTO policy (org_id, name, content, version, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(org_id, name) DO UPDATE SET content = excluded.content,
                                                     version = excluded.version,
                                                     updated_at = excluded.updated_at",
            params![
                org_id,
                name,
                content,
                version,
                chrono::Utc::now().to_rfc3339()
            ],
        )
        .context("failed to write policy")?;
        tx.commit()?;
        Ok(version)
    }

    pub(crate) fn list_policies(&self, org_id: &str) -> Result<Vec<PolicySummary>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT name, version, updated_at FROM policy WHERE org_id = ?1 ORDER BY name",
        )?;
        let rows = stmt.query_map([org_id], |r| {
            Ok(PolicySummary {
                name: r.get(0)?,
                version: r.get(1)?,
                updated_at: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub(crate) fn get_policy(&self, org_id: &str, name: &str) -> Result<Option<PolicyDoc>> {
        let conn = self.conn()?;
        let doc = conn
            .query_row(
                "SELECT name, content, version, updated_at FROM policy
                 WHERE org_id = ?1 AND name = ?2",
                params![org_id, name],
                |r| {
                    Ok(PolicyDoc {
                        name: r.get(0)?,
                        content: r.get(1)?,
                        version: r.get(2)?,
                        updated_at: r.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(doc)
    }

    pub(crate) fn put_compliance_snapshots(
        &self,
        org_id: &str,
        snaps: &[ComplianceSnapshot],
    ) -> Result<()> {
        let mut guard = self.conn()?;
        let tx = guard.transaction()?;
        for snap in snaps {
            tx.execute(
                "INSERT INTO compliance_snapshot
                     (id, org_id, repo_id, repo_name, policy_name, policy_version,
                      evaluated_at, ai_ranges, violations, high, medium, low)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    ids::generate_id("cmp"),
                    org_id,
                    snap.repo_id,
                    snap.repo_name,
                    snap.policy_name,
                    snap.policy_version,
                    snap.evaluated_at,
                    snap.ai_ranges,
                    snap.violations,
                    snap.high,
                    snap.medium,
                    snap.low,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn latest_compliance(&self, org_id: &str) -> Result<Vec<ComplianceSnapshot>> {
        let conn = self.conn()?;
        // Newest snapshot per repo: pick the max (evaluated_at, id) so a tie on
        // the timestamp is still deterministic.
        let mut stmt = conn.prepare(
            "SELECT cs.repo_id, cs.repo_name, cs.policy_name, cs.policy_version,
                    cs.evaluated_at, cs.ai_ranges, cs.violations, cs.high, cs.medium, cs.low
             FROM compliance_snapshot cs
             JOIN (
                 SELECT repo_id, MAX(evaluated_at || '|' || id) AS mk
                 FROM compliance_snapshot WHERE org_id = ?1 GROUP BY repo_id
             ) latest
               ON cs.repo_id = latest.repo_id
              AND (cs.evaluated_at || '|' || cs.id) = latest.mk
             WHERE cs.org_id = ?1
             ORDER BY cs.evaluated_at DESC, cs.repo_name",
        )?;
        let rows = stmt.query_map([org_id], |r| {
            Ok(ComplianceSnapshot {
                repo_id: r.get(0)?,
                repo_name: r.get(1)?,
                policy_name: r.get(2)?,
                policy_version: r.get(3)?,
                evaluated_at: r.get(4)?,
                ai_ranges: r.get(5)?,
                violations: r.get(6)?,
                high: r.get(7)?,
                medium: r.get(8)?,
                low: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
