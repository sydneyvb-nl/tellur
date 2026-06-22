//! Events, attributions, reports, sessions, and activity.

use super::*;

impl PostgresStore {
    pub(crate) fn append_events(
        &self,
        org_id: &str,
        repo_id: &str,
        events: &[IngestEvent],
    ) -> Result<Vec<String>> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        tx.execute(
            "SELECT pg_advisory_xact_lock($1)",
            &[&advisory_key(&format!("event:{repo_id}"))],
        )?;

        let belongs = tx
            .query_opt(
                "SELECT 1 FROM repo WHERE id = $1 AND org_id = $2",
                &[&repo_id, &org_id],
            )?
            .is_some();
        if !belongs {
            bail!("repo {repo_id} not found in org {org_id}");
        }

        let (mut prev, mut count) = read_head(
            &mut tx,
            "SELECT head_hash, entry_count FROM event_head WHERE repo_id = $1",
            &repo_id,
        )?;

        let mut new_ids = Vec::with_capacity(events.len());
        for ev in events {
            let id = ids::generate_event_id();
            let prev_opt = (!prev.is_empty()).then_some(prev.as_str());
            let entry_hash = ids::hash_event(
                &id,
                &ev.session_id,
                &ev.timestamp,
                &ev.event_type,
                &ev.actor,
                &ev.payload,
                prev_opt,
            );
            let payload_str = serde_json::to_string(&ev.payload)?;
            tx.execute(
                "INSERT INTO event
                     (id, org_id, repo_id, session_id, ts, event_type, actor, payload,
                      prev_hash, entry_hash)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
                &[
                    &id,
                    &org_id,
                    &repo_id,
                    &ev.session_id,
                    &ev.timestamp,
                    &ev.event_type,
                    &ev.actor,
                    &payload_str,
                    &prev,
                    &entry_hash,
                ],
            )
            .context("failed to insert event")?;
            prev = entry_hash;
            count += 1;
            new_ids.push(id);
        }
        tx.execute(
            "INSERT INTO event_head (repo_id, head_hash, entry_count) VALUES ($1, $2, $3)
             ON CONFLICT (repo_id) DO UPDATE SET head_hash = excluded.head_hash,
                                                 entry_count = excluded.entry_count",
            &[&repo_id, &prev, &count],
        )?;
        tx.commit().context("failed to commit events")?;
        Ok(new_ids)
    }


    pub(crate) fn event_count(&self, org_id: &str, repo_id: &str) -> Result<u64> {
        let n: i64 = self
            .client()?
            .query_one(
                "SELECT COUNT(*) FROM event WHERE org_id = $1 AND repo_id = $2",
                &[&org_id, &repo_id],
            )?
            .get(0);
        Ok(n as u64)
    }


    pub(crate) fn verify_event_chain(&self, org_id: &str, repo_id: &str) -> Result<bool> {
        let mut client = self.client()?;
        let rows = client.query(
            "SELECT id, session_id, ts, event_type, actor, payload, prev_hash, entry_hash
             FROM event WHERE org_id = $1 AND repo_id = $2 ORDER BY seq ASC",
            &[&org_id, &repo_id],
        )?;
        let mut expected_prev = String::new();
        let mut counted: i64 = 0;
        for r in &rows {
            let id: String = r.get(0);
            let payload: String = r.get(5);
            let prev_hash: String = r.get(6);
            let entry_hash: String = r.get(7);
            if prev_hash != expected_prev {
                return Ok(false);
            }
            let payload_value: serde_json::Value = serde_json::from_str(&payload)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            let prev_opt = (!prev_hash.is_empty()).then_some(prev_hash.as_str());
            let recomputed = ids::hash_event(
                &id,
                &r.get::<_, String>(1),
                &r.get::<_, String>(2),
                &r.get::<_, String>(3),
                &r.get::<_, String>(4),
                &payload_value,
                prev_opt,
            );
            if recomputed != entry_hash {
                return Ok(false);
            }
            expected_prev = entry_hash;
            counted += 1;
        }
        let head = client.query_opt(
            "SELECT head_hash, entry_count FROM event_head WHERE repo_id = $1",
            &[&repo_id],
        )?;
        match head {
            Some(row) => {
                Ok(counted == row.get::<_, i64>(1) && expected_prev == row.get::<_, String>(0))
            }
            None => Ok(counted == 0),
        }
    }


    pub(crate) fn put_attributions(
        &self,
        org_id: &str,
        repo_id: &str,
        files: &[FileAttribution],
    ) -> Result<usize> {
        let mut client = self.client()?;
        let mut tx = client.transaction()?;
        let belongs = tx
            .query_opt(
                "SELECT 1 FROM repo WHERE id = $1 AND org_id = $2",
                &[&repo_id, &org_id],
            )?
            .is_some();
        if !belongs {
            bail!("repo {repo_id} not found in org {org_id}");
        }
        let now = chrono::Utc::now().to_rfc3339();
        for file in files {
            // Empty ranges = tombstone: drop the row (file lost its attribution,
            // e.g. deleted from the repo) instead of leaving stale ranges.
            if file.ranges.is_empty() {
                tx.execute(
                    "DELETE FROM attribution
                     WHERE org_id = $1 AND repo_id = $2 AND file_path = $3",
                    &[&org_id, &repo_id, &file.file_path],
                )
                .context("failed to delete attribution")?;
                continue;
            }
            let ranges_json = serde_json::to_string(&file.ranges)?;
            tx.execute(
                "INSERT INTO attribution
                     (org_id, repo_id, file_path, git_blob_sha, ranges_json, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (org_id, repo_id, file_path) DO UPDATE SET
                     git_blob_sha = excluded.git_blob_sha,
                     ranges_json = excluded.ranges_json,
                     updated_at = excluded.updated_at",
                &[
                    &org_id,
                    &repo_id,
                    &file.file_path,
                    &file.git_blob_sha,
                    &ranges_json,
                    &now,
                ],
            )
            .context("failed to upsert attribution")?;
        }
        tx.commit()?;
        Ok(files.len())
    }


    pub(crate) fn list_attributions(&self, org_id: &str, repo_id: &str) -> Result<Vec<FileAttribution>> {
        let rows = self.client()?.query(
            "SELECT file_path, git_blob_sha, ranges_json, updated_at
             FROM attribution WHERE org_id = $1 AND repo_id = $2 ORDER BY file_path",
            &[&org_id, &repo_id],
        )?;
        let mut out = Vec::new();
        for r in &rows {
            let file_path: String = r.get(0);
            let ranges_json: String = r.get(2);
            let ranges = serde_json::from_str(&ranges_json)
                .with_context(|| format!("corrupt attribution ranges for {file_path}"))?;
            out.push(FileAttribution {
                schema: "tellur.attribution.v1".to_string(),
                file_path,
                git_blob_sha: r.get(1),
                ranges,
                updated_at: r.get(3),
            });
        }
        Ok(out)
    }


    pub(crate) fn list_events(
        &self,
        org_id: &str,
        repo_id: &str,
        limit: u32,
        before_seq: Option<i64>,
    ) -> Result<Vec<StoredEvent>> {
        let cursor = before_seq.unwrap_or(i64::MAX);
        let rows = self.client()?.query(
            "SELECT seq, id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = $1 AND repo_id = $2 AND seq < $3
             ORDER BY seq DESC LIMIT $4",
            &[&org_id, &repo_id, &cursor, &(limit as i64)],
        )?;
        let mut out = Vec::new();
        for r in &rows {
            let id: String = r.get(1);
            let payload_str: String = r.get(6);
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq: r.get(0),
                id,
                repo_id: repo_id.to_string(),
                session_id: r.get(2),
                timestamp: r.get(3),
                event_type: r.get(4),
                actor: r.get(5),
                payload,
            });
        }
        Ok(out)
    }


    pub(crate) fn org_report(&self, org_id: &str) -> Result<OrgReport> {
        let mut client = self.client()?;
        let total_events: i64 = client
            .query_one("SELECT COUNT(*) FROM event WHERE org_id = $1", &[&org_id])?
            .get(0);
        let distinct_sessions: i64 = client
            .query_one(
                "SELECT COUNT(DISTINCT session_id) FROM event WHERE org_id = $1",
                &[&org_id],
            )?
            .get(0);
        let by_type = group_counts(&mut client, "event_type", org_id)?;
        let by_actor = group_counts(&mut client, "actor", org_id)?;
        // Return this connection to the pool before `list_repos` checks out a
        // second one; otherwise concurrent reports (>= pool size) could each
        // hold one connection while waiting for another, deadlocking the pool.
        drop(client);
        let repos = self.list_repos(org_id)?;
        Ok(OrgReport {
            org_id: org_id.to_string(),
            total_events: total_events as u64,
            distinct_sessions: distinct_sessions as u64,
            by_type,
            by_actor,
            repos,
        })
    }


    pub(crate) fn recent_org_events(&self, org_id: &str, limit: u32) -> Result<Vec<StoredEvent>> {
        let rows = self.client()?.query(
            "SELECT seq, id, repo_id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = $1 ORDER BY seq DESC LIMIT $2",
            &[&org_id, &(limit as i64)],
        )?;
        let mut out = Vec::new();
        for r in &rows {
            let id: String = r.get(1);
            let payload_str: String = r.get(7);
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq: r.get(0),
                id,
                repo_id: r.get(2),
                session_id: r.get(3),
                timestamp: r.get(4),
                event_type: r.get(5),
                actor: r.get(6),
                payload,
            });
        }
        Ok(out)
    }


    pub(crate) fn activity_by_day(
        &self,
        org_id: &str,
        since_rfc3339: &str,
        group: ActivityGroup,
    ) -> Result<Vec<ActivityBucket>> {
        // The grouping column is an allow-listed constant, never user input.
        let sql = format!(
            "SELECT left(ts, 10) AS day, {col} AS key, COUNT(*) AS n
             FROM event WHERE org_id = $1 AND ts >= $2
             GROUP BY day, key ORDER BY day ASC, key ASC",
            col = group.column()
        );
        let rows = self.client()?.query(&sql, &[&org_id, &since_rfc3339])?;
        Ok(rows
            .iter()
            .map(|r| ActivityBucket {
                day: r.get(0),
                key: r.get(1),
                count: r.get::<_, i64>(2) as u64,
            })
            .collect())
    }


    pub(crate) fn repo_facts(&self, org_id: &str, repo_id: &str) -> Result<RepoFacts> {
        let mut client = self.client()?;
        let event_count: i64 = client
            .query_one(
                "SELECT COUNT(*) FROM event WHERE org_id = $1 AND repo_id = $2",
                &[&org_id, &repo_id],
            )?
            .get(0);
        let last_activity: Option<String> = client
            .query_one(
                "SELECT MAX(ts) FROM event WHERE org_id = $1 AND repo_id = $2",
                &[&org_id, &repo_id],
            )?
            .get(0);
        let rows = client.query(
            "SELECT DISTINCT actor FROM event
             WHERE org_id = $1 AND repo_id = $2 ORDER BY actor",
            &[&org_id, &repo_id],
        )?;
        Ok(RepoFacts {
            event_count: event_count as u64,
            contributors: rows.iter().map(|r| r.get(0)).collect(),
            last_activity,
        })
    }


    pub(crate) fn list_sessions(
        &self,
        org_id: &str,
        repo_id: Option<&str>,
        actor: Option<&str>,
        since_rfc3339: Option<&str>,
        limit: u32,
    ) -> Result<Vec<SessionSummary>> {
        let rows = self.client()?.query(
            "SELECT session_id, COUNT(*) AS n, MIN(ts) AS f, MAX(ts) AS l,
                    string_agg(DISTINCT actor, ',') AS actors,
                    string_agg(DISTINCT repo_id, ',') AS repos
             FROM event
             WHERE org_id = $1
               AND ($2::text IS NULL OR repo_id = $2)
               AND ($3::text IS NULL OR actor = $3)
               AND ($4::text IS NULL OR ts >= $4)
             GROUP BY session_id ORDER BY l DESC LIMIT $5",
            &[&org_id, &repo_id, &actor, &since_rfc3339, &(limit as i64)],
        )?;
        Ok(rows
            .iter()
            .map(|r| SessionSummary {
                session_id: r.get(0),
                event_count: r.get::<_, i64>(1) as u64,
                first_ts: r.get(2),
                last_ts: r.get(3),
                actors: split_csv(r.get::<_, Option<String>>(4)),
                repos: split_csv(r.get::<_, Option<String>>(5)),
            })
            .collect())
    }


    pub(crate) fn session_events(
        &self,
        org_id: &str,
        session_id: &str,
        limit: u32,
    ) -> Result<Vec<StoredEvent>> {
        let rows = self.client()?.query(
            "SELECT seq, id, repo_id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = $1 AND session_id = $2 ORDER BY seq ASC LIMIT $3",
            &[&org_id, &session_id, &(limit as i64)],
        )?;
        let mut out = Vec::new();
        for r in &rows {
            let id: String = r.get(1);
            let payload_str: String = r.get(7);
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq: r.get(0),
                id,
                repo_id: r.get(2),
                session_id: r.get(3),
                timestamp: r.get(4),
                event_type: r.get(5),
                actor: r.get(6),
                payload,
            });
        }
        Ok(out)
    }


    pub(crate) fn export_events(&self, org_id: &str) -> Result<Vec<StoredEvent>> {
        let rows = self.client()?.query(
            "SELECT seq, id, repo_id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = $1 ORDER BY seq ASC",
            &[&org_id],
        )?;
        let mut out = Vec::new();
        for r in &rows {
            let id: String = r.get(1);
            let payload_str: String = r.get(7);
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq: r.get(0),
                id,
                repo_id: r.get(2),
                session_id: r.get(3),
                timestamp: r.get(4),
                event_type: r.get(5),
                actor: r.get(6),
                payload,
            });
        }
        Ok(out)
    }

}
