//! Events, attributions, reports, sessions, and activity.

use super::*;

impl SqliteStore {
    pub(crate) fn append_events(
        &self,
        org_id: &str,
        repo_id: &str,
        events: &[IngestEvent],
    ) -> Result<Vec<String>> {
        let mut guard = self.conn()?;
        let tx = guard
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to begin ingest transaction")?;

        // Tenant scoping: the repo must belong to the caller's org.
        let belongs = tx
            .query_row(
                "SELECT 1 FROM repo WHERE id = ?1 AND org_id = ?2",
                params![repo_id, org_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !belongs {
            bail!("repo {repo_id} not found in org {org_id}");
        }

        // The head checkpoint is the authoritative chain tip + length.
        let head = chain::HeadRef {
            table: "event_head",
            key_col: "repo_id",
            key: &repo_id,
        };
        let (mut prev, mut count) = chain::read_head(&tx, &head)?;

        let mut new_ids = Vec::with_capacity(events.len());
        for ev in events {
            let id = ids::generate_event_id();
            let prev_opt = if prev.is_empty() {
                None
            } else {
                Some(prev.as_str())
            };
            // Server recomputes the chain hash — client hashes are never trusted.
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
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    org_id,
                    repo_id,
                    ev.session_id,
                    ev.timestamp,
                    ev.event_type,
                    ev.actor,
                    payload_str,
                    prev,
                    entry_hash
                ],
            )
            .context("failed to insert event")?;
            prev = entry_hash;
            count += 1;
            new_ids.push(id);
        }
        chain::write_head(&tx, &head, &prev, count)?;
        tx.commit().context("failed to commit events")?;
        Ok(new_ids)
    }

    pub(crate) fn event_count(&self, org_id: &str, repo_id: &str) -> Result<u64> {
        let conn = self.conn()?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM event WHERE org_id = ?1 AND repo_id = ?2",
            params![org_id, repo_id],
            |r| r.get(0),
        )?;
        Ok(n as u64)
    }

    pub(crate) fn verify_event_chain(&self, org_id: &str, repo_id: &str) -> Result<bool> {
        let conn = self.conn()?;
        let head = chain::HeadRef {
            table: "event_head",
            key_col: "repo_id",
            key: &repo_id,
        };
        chain::verify(
            &conn,
            "SELECT id, session_id, ts, event_type, actor, payload, prev_hash, entry_hash
             FROM event WHERE org_id = ?1 AND repo_id = ?2 ORDER BY seq ASC",
            params![org_id, repo_id],
            &head,
            ("", 0),
            |r| {
                let id: String = r.get(0)?;
                let session_id: String = r.get(1)?;
                let ts: String = r.get(2)?;
                let event_type: String = r.get(3)?;
                let actor: String = r.get(4)?;
                let payload: String = r.get(5)?;
                let prev_hash: String = r.get(6)?;
                let entry_hash: String = r.get(7)?;
                let payload_value: serde_json::Value = serde_json::from_str(&payload)
                    .with_context(|| format!("corrupt event payload for event {id}"))?;
                let prev_opt = (!prev_hash.is_empty()).then_some(prev_hash.as_str());
                let recomputed = ids::hash_event(
                    &id,
                    &session_id,
                    &ts,
                    &event_type,
                    &actor,
                    &payload_value,
                    prev_opt,
                );
                Ok((prev_hash, entry_hash, recomputed))
            },
        )
    }

    pub(crate) fn put_attributions(
        &self,
        org_id: &str,
        repo_id: &str,
        files: &[FileAttribution],
    ) -> Result<usize> {
        let mut guard = self.conn()?;
        let tx = guard
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .context("failed to begin attribution transaction")?;
        // Tenant scoping: the repo must belong to the caller's org.
        let belongs = tx
            .query_row(
                "SELECT 1 FROM repo WHERE id = ?1 AND org_id = ?2",
                params![repo_id, org_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !belongs {
            bail!("repo {repo_id} not found in org {org_id}");
        }
        let now = chrono::Utc::now().to_rfc3339();
        for file in files {
            // An empty-ranges file is a tombstone: the file no longer has
            // attribution (e.g. it was deleted from the repo), so drop the row
            // rather than leaving stale ranges that keep counting toward metrics.
            if file.ranges.is_empty() {
                tx.execute(
                    "DELETE FROM attribution
                     WHERE org_id = ?1 AND repo_id = ?2 AND file_path = ?3",
                    params![org_id, repo_id, file.file_path],
                )
                .context("failed to delete attribution")?;
                continue;
            }
            let ranges_json = serde_json::to_string(&file.ranges)?;
            tx.execute(
                "INSERT INTO attribution
                     (org_id, repo_id, file_path, git_blob_sha, ranges_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(org_id, repo_id, file_path) DO UPDATE SET
                     git_blob_sha = excluded.git_blob_sha,
                     ranges_json  = excluded.ranges_json,
                     updated_at   = excluded.updated_at",
                params![
                    org_id,
                    repo_id,
                    file.file_path,
                    file.git_blob_sha,
                    ranges_json,
                    now
                ],
            )
            .context("failed to upsert attribution")?;
        }
        tx.commit().context("failed to commit attributions")?;
        Ok(files.len())
    }

    pub(crate) fn list_attributions(
        &self,
        org_id: &str,
        repo_id: &str,
    ) -> Result<Vec<FileAttribution>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT file_path, git_blob_sha, ranges_json, updated_at
             FROM attribution WHERE org_id = ?1 AND repo_id = ?2 ORDER BY file_path",
        )?;
        let rows = stmt.query_map(params![org_id, repo_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (file_path, git_blob_sha, ranges_json, updated_at) = row?;
            let ranges = serde_json::from_str(&ranges_json)
                .with_context(|| format!("corrupt attribution ranges for {file_path}"))?;
            out.push(FileAttribution {
                schema: "tellur.attribution.v1".to_string(),
                file_path,
                git_blob_sha,
                ranges,
                updated_at,
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
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT seq, id, session_id, ts, event_type, actor, payload
             FROM event
             WHERE org_id = ?1 AND repo_id = ?2 AND seq < ?3
             ORDER BY seq DESC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![org_id, repo_id, cursor, limit], |r| {
            let payload_str: String = r.get(6)?;
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                payload_str,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, id, session_id, timestamp, event_type, actor, payload_str) = row?;
            // Surface integrity problems instead of masking them as `null`.
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq,
                id,
                repo_id: repo_id.to_string(),
                session_id,
                timestamp,
                event_type,
                actor,
                payload,
            });
        }
        Ok(out)
    }

    pub(crate) fn org_report(&self, org_id: &str) -> Result<OrgReport> {
        let conn = self.conn()?;
        let total_events: i64 = conn.query_row(
            "SELECT COUNT(*) FROM event WHERE org_id = ?1",
            [org_id],
            |r| r.get(0),
        )?;
        let distinct_sessions: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT session_id) FROM event WHERE org_id = ?1",
            [org_id],
            |r| r.get(0),
        )?;

        let by_type = group_counts(&conn, "event_type", org_id)?;
        let by_actor = group_counts(&conn, "actor", org_id)?;
        drop(conn);

        Ok(OrgReport {
            org_id: org_id.to_string(),
            total_events: total_events as u64,
            distinct_sessions: distinct_sessions as u64,
            by_type,
            by_actor,
            repos: self.list_repos(org_id)?,
        })
    }

    pub(crate) fn recent_org_events(&self, org_id: &str, limit: u32) -> Result<Vec<StoredEvent>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT seq, id, repo_id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = ?1 ORDER BY seq DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![org_id, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, id, repo_id, session_id, timestamp, event_type, actor, payload_str) = row?;
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq,
                id,
                repo_id,
                session_id,
                timestamp,
                event_type,
                actor,
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
        let conn = self.conn()?;
        // The grouping column is an allow-listed constant, never user input.
        let sql = format!(
            "SELECT substr(ts, 1, 10) AS day, {col} AS key, COUNT(*) AS n
             FROM event WHERE org_id = ?1 AND ts >= ?2
             GROUP BY day, key ORDER BY day ASC, key ASC",
            col = group.column()
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![org_id, since_rfc3339], |r| {
            Ok(ActivityBucket {
                day: r.get(0)?,
                key: r.get(1)?,
                count: r.get::<_, i64>(2)? as u64,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub(crate) fn repo_facts(&self, org_id: &str, repo_id: &str) -> Result<RepoFacts> {
        let conn = self.conn()?;
        let event_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM event WHERE org_id = ?1 AND repo_id = ?2",
            params![org_id, repo_id],
            |r| r.get(0),
        )?;
        let last_activity: Option<String> = conn.query_row(
            "SELECT MAX(ts) FROM event WHERE org_id = ?1 AND repo_id = ?2",
            params![org_id, repo_id],
            |r| r.get(0),
        )?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT actor FROM event
             WHERE org_id = ?1 AND repo_id = ?2 ORDER BY actor",
        )?;
        let contributors = stmt
            .query_map(params![org_id, repo_id], |r| r.get::<_, String>(0))?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(RepoFacts {
            event_count: event_count as u64,
            contributors,
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
        let conn = self.conn()?;
        // NOTE: SQLite's `group_concat(DISTINCT x)` only supports the default
        // comma separator, so an actor/repo id literally containing a comma
        // would split wrongly in `split_csv`. Repo ids are generated (no commas)
        // and actors are agent ids; revisit if free-form actor strings arrive.
        let mut stmt = conn.prepare(
            "SELECT session_id, COUNT(*) AS n, MIN(ts) AS f, MAX(ts) AS l,
                    group_concat(DISTINCT actor) AS actors,
                    group_concat(DISTINCT repo_id) AS repos
             FROM event
             WHERE org_id = ?1
               AND (?2 IS NULL OR repo_id = ?2)
               AND (?3 IS NULL OR actor = ?3)
               AND (?4 IS NULL OR ts >= ?4)
             GROUP BY session_id ORDER BY l DESC LIMIT ?5",
        )?;
        let rows = stmt.query_map(params![org_id, repo_id, actor, since_rfc3339, limit], |r| {
            Ok(SessionSummary {
                session_id: r.get(0)?,
                event_count: r.get::<_, i64>(1)? as u64,
                first_ts: r.get(2)?,
                last_ts: r.get(3)?,
                actors: split_csv(r.get::<_, Option<String>>(4)?),
                repos: split_csv(r.get::<_, Option<String>>(5)?),
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub(crate) fn session_events(
        &self,
        org_id: &str,
        session_id: &str,
        limit: u32,
    ) -> Result<Vec<StoredEvent>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT seq, id, repo_id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = ?1 AND session_id = ?2 ORDER BY seq ASC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![org_id, session_id, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, id, repo_id, session_id, timestamp, event_type, actor, payload_str) = row?;
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq,
                id,
                repo_id,
                session_id,
                timestamp,
                event_type,
                actor,
                payload,
            });
        }
        Ok(out)
    }

    pub(crate) fn export_events(&self, org_id: &str) -> Result<Vec<StoredEvent>> {
        let conn = self.conn()?;
        // Include repo_id: an org-level export spans multiple repos, so each
        // event must carry which repo it belongs to.
        let mut stmt = conn.prepare(
            "SELECT seq, id, repo_id, session_id, ts, event_type, actor, payload
             FROM event WHERE org_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map([org_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, id, repo_id, session_id, timestamp, event_type, actor, payload_str) = row?;
            let payload = serde_json::from_str(&payload_str)
                .with_context(|| format!("corrupt event payload for event {id}"))?;
            out.push(StoredEvent {
                seq,
                id,
                repo_id,
                session_id,
                timestamp,
                event_type,
                actor,
                payload,
            });
        }
        Ok(out)
    }
}
