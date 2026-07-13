//! Hub sync commands: `login`, `logout`, and `push` (incremental, idempotent
//! delivery of events and line-level attribution to a team hub).

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use tellur_core::schema::types::{EventActor, FileAttribution};
use tellur_core::storage::{RepoStorage, TraceIndex};

use crate::hub;

/// Resolve the hub base URL from an explicit flag, the `TELLUR_HUB_URL` env, the
/// machine-wide default, or — when exactly one hub is saved — stored credentials.
/// Errors otherwise so a typo never silently targets the wrong hub.
pub(crate) fn resolve_hub(explicit: Option<&str>, creds: &hub::Credentials) -> Result<String> {
    if let Some(h) = explicit {
        return Ok(hub::normalize_host(h));
    }
    if let Ok(h) = std::env::var("TELLUR_HUB_URL") {
        return Ok(hub::normalize_host(&h));
    }
    if let Some(default) = creds.default_host.as_deref() {
        let normalized = hub::normalize_host(default);
        if creds.hosts.contains_key(&normalized) {
            return Ok(normalized);
        }
    }
    // Resolve the single saved host without indexing-then-unwrapping, so a future
    // refactor of the match condition can't turn this into a panic.
    let mut hosts = creds.hosts.keys();
    match (hosts.next(), hosts.next()) {
        (Some(only), None) => Ok(only.clone()),
        (None, _) => {
            bail!("no hub configured — pass --hub or set TELLUR_HUB_URL (or run `tellur login`)")
        }
        _ => bail!("multiple hubs are saved — pass --hub to choose one"),
    }
}

/// Best-effort open of a URL in the user's default browser. A failure is not
/// fatal: the URL is always printed so the user can open it manually.
fn open_browser(url: &str) -> bool {
    let (program, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(target_os = "windows") {
        ("cmd", vec!["/C", "start", "", url])
    } else {
        ("xdg-open", vec![url])
    };
    std::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// `tellur login` — device-authorization flow. Opens the hub's approval page in
/// a browser, then polls until a signed-in member approves, and stores the
/// minted token under the per-user config dir.
pub(crate) fn cmd_login(hub_arg: Option<&str>, no_browser: bool) -> Result<()> {
    let mut creds = hub::Credentials::load()?;
    // For login the hub must be explicit (flag or env); we are not yet logged in.
    let hub_url = hub_arg
        .map(hub::normalize_host)
        .or_else(|| {
            std::env::var("TELLUR_HUB_URL")
                .ok()
                .map(|h| hub::normalize_host(&h))
        })
        .context("hub URL required for login (--hub or TELLUR_HUB_URL)")?;

    let auth = hub::device_authorize(&hub_url)
        .context("could not start login (is the hub reachable and SSO enabled?)")?;
    let verify_url = format!(
        "{}/auth/device?user_code={}",
        hub_url,
        auth.user_code.replace('-', "%2D")
    );

    println!("\nTo sign in, open this URL in your browser:\n");
    println!("    {verify_url}\n");
    println!("and confirm this code:\n");
    println!("    {}\n", auth.user_code);

    if !no_browser && open_browser(&verify_url) {
        println!("(Opened your browser automatically.)\n");
    }

    let mut interval = auth.interval.max(1);
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(auth.expires_in.max(60));
    print!("Waiting for approval");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    loop {
        if std::time::Instant::now() >= deadline {
            println!();
            bail!("login timed out before approval — run `tellur login` again");
        }
        std::thread::sleep(std::time::Duration::from_secs(interval));
        match hub::device_poll(&hub_url, &auth.device_code)? {
            hub::DevicePoll::Approved(host_creds) => {
                let role = host_creds.role.clone();
                let org = host_creds.org_id.clone();
                let normalized = hub::normalize_host(&hub_url);
                creds.hosts.insert(normalized.clone(), host_creds);
                creds.default_host = Some(normalized);
                creds.save()?;
                println!("\n\n✓ Signed in to {hub_url}");
                println!("  org {org} · role {role}");
                println!("  Token stored in {}", hub::Credentials::path()?.display());
                println!("\nNext: run `tellur push` from a repo to send activity to the hub.");
                return Ok(());
            }
            hub::DevicePoll::Pending => {
                print!(".");
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
            hub::DevicePoll::SlowDown => {
                interval += 5;
            }
            hub::DevicePoll::Denied => {
                println!();
                bail!("login was denied in the browser");
            }
            hub::DevicePoll::Expired => {
                println!();
                bail!("the login request expired — run `tellur login` again");
            }
        }
    }
}

/// `tellur logout` — forget stored credentials for a hub.
pub(crate) fn cmd_logout(hub_arg: Option<&str>) -> Result<()> {
    let mut creds = hub::Credentials::load()?;
    let hub_url = resolve_hub(hub_arg, &creds)?;
    if creds.hosts.remove(&hub_url).is_some() {
        if creds.default_host.as_deref() == Some(hub_url.as_str()) {
            creds.default_host = if creds.hosts.len() == 1 {
                creds.hosts.keys().next().cloned()
            } else {
                None
            };
        }
        creds.save()?;
        println!("Removed stored credentials for {hub_url}");
    } else {
        println!("No stored credentials for {hub_url}");
    }
    Ok(())
}

/// Per-target push high-water mark, persisted in `.tellur/push_state.json`.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct PushState {
    #[serde(default)]
    targets: std::collections::BTreeMap<String, PushTarget>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct PushTarget {
    /// Id of the last event already delivered to this target. `None` until the
    /// first event push (e.g. when only attribution has been pushed so far).
    #[serde(default)]
    last_pushed_id: Option<String>,
    /// How many events have been delivered (for display).
    #[serde(default)]
    count: u64,
    /// File paths whose attribution we last pushed — used to send delete
    /// tombstones for files that have since been removed from the repo.
    #[serde(default)]
    attr_paths: Vec<String>,
}

fn push_state_path(storage: &RepoStorage) -> PathBuf {
    storage.tellur_dir.join("push_state.json")
}

fn load_push_state(storage: &RepoStorage) -> Result<PushState> {
    let path = push_state_path(storage);
    if !path.exists() {
        return Ok(PushState::default());
    }
    let body = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&body).unwrap_or_default())
}

fn save_push_state(storage: &RepoStorage, state: &PushState) -> Result<()> {
    let path = push_state_path(storage);
    // Write to a temp file then rename, so a crash mid-write can't leave a
    // truncated push_state.json (which would silently reset the high-water mark).
    // The temp name is per-process (pid + nanos) so two concurrent `tellur push`
    // runs — e.g. the `connect --background` timer overlapping a pre-push — don't
    // share one temp and make each other's `rename` fail. `rename` is atomic;
    // concurrent renames are last-writer-wins on the final file, which is safe
    // because hub ingest is idempotent.
    let tmp = storage.tellur_dir.join(format!(
        "push_state.{}.{}.tmp",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::write(&tmp, serde_json::to_string_pretty(state)?)?;
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// The hub's ingest wire string for an event actor.
fn actor_wire(actor: &EventActor) -> &'static str {
    match actor {
        EventActor::Human => "human",
        EventActor::Agent => "agent",
        EventActor::System => "system",
        EventActor::Unknown => "unknown",
    }
}

/// Index of the first event to push, given the ordered local event ids and the
/// saved high-water mark. `reset` (or no mark yet) pushes everything; otherwise
/// resume strictly after the last delivered id. A missing mark means the local
/// log was rotated/pruned out from under us — error rather than risk silently
/// re-sending (the hub would store duplicates).
fn push_start_index(ids: &[&str], last_pushed: Option<&str>, reset: bool) -> Result<usize> {
    if reset {
        return Ok(0);
    }
    match last_pushed {
        None => Ok(0),
        Some(id) => match ids.iter().rposition(|x| *x == id) {
            Some(pos) => Ok(pos + 1),
            None => bail!(
                "the last pushed event ({id}) is no longer in the local log — it may have been \
                 rotated or pruned. Re-run with --reset to push all events again."
            ),
        },
    }
}

/// `tellur push` — forward locally-captured events to a team hub, incrementally.
pub(crate) fn cmd_push(
    hub_arg: Option<&str>,
    org_arg: Option<&str>,
    repo_arg: Option<&str>,
    token_arg: Option<&str>,
    dry_run: bool,
    reset: bool,
) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        bail!("Tellur is not initialized here — run `tellur init` first");
    }
    let creds = hub::Credentials::load()?;
    let hub_url = resolve_hub(hub_arg, &creds)?;
    let saved = creds.get(&hub_url);

    // Token: flag › env › stored credentials.
    let token = token_arg
        .map(str::to_string)
        .or_else(|| std::env::var("TELLUR_HUB_TOKEN").ok())
        .or_else(|| saved.map(|s| s.token.clone()))
        .context("no token — run `tellur login`, pass --token, or set TELLUR_HUB_TOKEN")?;

    // Org: flag › env › stored credentials.
    let org = org_arg
        .map(str::to_string)
        .or_else(|| std::env::var("TELLUR_HUB_ORG").ok())
        .or_else(|| saved.map(|s| s.org_id.clone()))
        .context("no org — pass --org, set TELLUR_HUB_ORG, or run `tellur login`")?;

    // Repo: flag › env › this repo's directory name.
    let repo = repo_arg
        .map(str::to_string)
        .or_else(|| std::env::var("TELLUR_HUB_REPO").ok())
        .or_else(|| {
            storage
                .root
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
        })
        .context("could not determine a repo name — pass --repo")?;

    let events = tellur_core::storage::read_events(&storage.traces_dir)?;
    let target_key = format!("{hub_url}#{org}#{repo}");
    let mut state = load_push_state(&storage)?;

    // Determine the slice of new events using the saved high-water mark. Skip the
    // high-water-mark check entirely when there are no local events, so an
    // attribution-only push still works.
    let (start, to_send): (usize, &[tellur_core::schema::types::TraceEvent]) = if events.is_empty()
    {
        (0, &[])
    } else {
        let last_pushed = state
            .targets
            .get(&target_key)
            .and_then(|t| t.last_pushed_id.as_deref());
        let ids: Vec<&str> = events.iter().map(|e| e.id.as_str()).collect();
        let s = push_start_index(&ids, last_pushed, reset)?;
        (s, &events[s..])
    };

    // Line-level attribution is a current-state projection (latest ranges per
    // file), so push the full local snapshot every run — the hub upserts per
    // file, so it's idempotent. This is what drives the AI-share / AI-lines
    // metrics; without it the dashboard shows 0 AI even though events arrived.
    let mut attr_payload = read_local_attributions(&storage)?;
    let current_paths: std::collections::BTreeSet<String> = attr_payload
        .iter()
        .filter_map(|v| v["file_path"].as_str().map(String::from))
        .collect();

    // Tombstones: files we previously pushed attribution for that are now gone
    // **from disk** (deleted from the repo). Gating on disk-absence — not just
    // absence from the index — avoids wiping the hub's attribution when the local
    // index is merely reset while the files still exist. An empty-ranges entry
    // tells the hub to delete that file's record so it stops counting.
    let prev_paths = state
        .targets
        .get(&target_key)
        .map(|t| t.attr_paths.clone())
        .unwrap_or_default();
    let mut tombstones = 0usize;
    for p in &prev_paths {
        if !current_paths.contains(p) && !storage.root.join(p).exists() {
            attr_payload.push(serde_json::json!({
                "schema": "tellur.attribution.v1",
                "file_path": p,
                "git_blob_sha": "",
                "ranges": [],
                "updated_at": chrono::Utc::now().to_rfc3339(),
            }));
            tombstones += 1;
        }
    }

    if dry_run {
        println!(
            "Would push {} new event(s) and {} attributed file(s){} to {hub_url}\n  org {org} · repo {repo}",
            to_send.len(),
            current_paths.len(),
            if tombstones > 0 {
                format!(" (+{tombstones} removed)")
            } else {
                String::new()
            },
        );
        return Ok(());
    }

    if to_send.is_empty() && attr_payload.is_empty() {
        println!("Already up to date — nothing to push.");
        return Ok(());
    }

    // Chunk under the server's per-request cap and update the high-water mark
    // after each accepted batch, so an interruption resumes cleanly.
    const CHUNK: usize = 500;
    let mut pushed = 0usize;
    for chunk in to_send.chunks(CHUNK) {
        let wire: Vec<serde_json::Value> = chunk
            .iter()
            .map(|e| {
                serde_json::json!({
                    "session_id": e.session_id,
                    "type": e.event_type.as_wire(),
                    "timestamp": e.timestamp,
                    "actor": actor_wire(&e.actor),
                    "payload": e.payload,
                })
            })
            .collect();
        let accepted = hub::ingest_events(&hub_url, &token, &org, &repo, &wire)
            .with_context(|| format!("failed pushing a batch of {} events", wire.len()))?;
        pushed += accepted;
        let last = chunk.last().unwrap();
        let entry = state.targets.entry(target_key.clone()).or_default();
        entry.last_pushed_id = Some(last.id.clone());
        entry.count = (start + pushed) as u64;
        save_push_state(&storage, &state)?;
    }

    // Push the attribution snapshot + any tombstones (idempotent per file).
    for chunk in attr_payload.chunks(CHUNK) {
        hub::ingest_attributions(&hub_url, &token, &org, &repo, chunk)
            .with_context(|| format!("failed pushing {} attribution record(s)", chunk.len()))?;
    }
    // Remember the file set we just pushed, so a future deletion can be tombstoned.
    let entry = state.targets.entry(target_key.clone()).or_default();
    entry.attr_paths = current_paths.iter().cloned().collect();
    save_push_state(&storage, &state)?;

    let removed_note = if tombstones > 0 {
        format!(" ({tombstones} removed)")
    } else {
        String::new()
    };
    println!(
        "✓ Pushed {pushed} event(s) and {} attributed file(s){removed_note} to {hub_url}\n  org {org} · repo {repo}",
        current_paths.len()
    );
    if current_paths.is_empty() && tombstones == 0 {
        println!(
            "  note: no line-level attribution found locally — AI-share metrics need \
             attribution, which `tellur watch`/agent hooks produce. Check `tellur blame <file>`."
        );
    }
    Ok(())
}

/// Read the local attribution index and group it into the hub's wire shape
/// (`FileAttribution` per file). Empty when the index does not exist yet.
fn read_local_attributions(storage: &RepoStorage) -> Result<Vec<serde_json::Value>> {
    if !storage.index_path.exists() {
        return Ok(Vec::new());
    }
    let index = TraceIndex::open(&storage.index_path)?;
    let rows = index.list_attributions()?;
    // Group ranges by file, keeping the latest blob sha seen for each.
    let mut by_file: std::collections::BTreeMap<
        String,
        (String, Vec<tellur_core::schema::types::AttributionRange>),
    > = std::collections::BTreeMap::new();
    for ia in rows {
        let entry = by_file
            .entry(ia.file_path)
            .or_insert_with(|| (ia.git_blob_sha.clone(), Vec::new()));
        entry.0 = ia.git_blob_sha;
        entry.1.push(ia.range);
    }
    let now = chrono::Utc::now().to_rfc3339();
    let files = by_file
        .into_iter()
        .map(|(file_path, (git_blob_sha, ranges))| {
            serde_json::to_value(FileAttribution {
                schema: "tellur.attribution.v1".to_string(),
                file_path,
                git_blob_sha,
                ranges,
                updated_at: now.clone(),
            })
            .expect("FileAttribution serializes")
        })
        .collect();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_credentials(hosts: &[&str], default_host: Option<&str>) -> hub::Credentials {
        hub::Credentials {
            default_host: default_host.map(str::to_owned),
            hosts: hosts
                .iter()
                .map(|host| {
                    (
                        (*host).to_owned(),
                        hub::HostCredentials {
                            token: "token".into(),
                            org_id: "org".into(),
                            member_id: "member".into(),
                            role: "contributor".into(),
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn resolve_hub_prefers_saved_default_with_multiple_hosts() {
        let creds = test_credentials(
            &["https://one.test", "https://two.test"],
            Some("https://two.test"),
        );
        assert_eq!(resolve_hub(None, &creds).unwrap(), "https://two.test");
    }

    #[test]
    fn resolve_hub_ignores_stale_default_and_keeps_ambiguity_safe() {
        let creds = test_credentials(
            &["https://one.test", "https://two.test"],
            Some("https://gone.test"),
        );
        assert!(resolve_hub(None, &creds).is_err());
    }

    #[test]
    fn push_start_index_pushes_all_without_a_mark() {
        let ids = ["a", "b", "c"];
        assert_eq!(push_start_index(&ids, None, false).unwrap(), 0);
    }

    #[test]
    fn push_start_index_resumes_after_last_mark() {
        let ids = ["a", "b", "c", "d"];
        assert_eq!(push_start_index(&ids, Some("b"), false).unwrap(), 2);
        // Up to date: mark is the final event → nothing new.
        assert_eq!(push_start_index(&ids, Some("d"), false).unwrap(), 4);
    }

    #[test]
    fn push_start_index_reset_ignores_the_mark() {
        let ids = ["a", "b", "c"];
        assert_eq!(push_start_index(&ids, Some("b"), true).unwrap(), 0);
    }

    #[test]
    fn push_start_index_errors_when_mark_is_gone() {
        let ids = ["c", "d", "e"]; // "b" was pruned out
        assert!(push_start_index(&ids, Some("b"), false).is_err());
    }

    #[test]
    fn actor_wire_maps_every_variant() {
        assert_eq!(actor_wire(&EventActor::Human), "human");
        assert_eq!(actor_wire(&EventActor::Agent), "agent");
        assert_eq!(actor_wire(&EventActor::System), "system");
        assert_eq!(actor_wire(&EventActor::Unknown), "unknown");
    }

    #[test]
    fn normalize_host_strips_trailing_slash() {
        assert_eq!(hub::normalize_host("https://h.test/"), "https://h.test");
        assert_eq!(hub::normalize_host("https://h.test"), "https://h.test");
    }

    #[test]
    fn read_local_attributions_groups_ranges_and_preserves_ai_origin() {
        use tellur_core::schema::types::{
            AttributionRange, AttributionState, EvidenceStrength, Origin,
        };
        let tmp = std::env::temp_dir().join(format!(
            "tellur-attr-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        let storage = RepoStorage::from_git_root(&tmp).unwrap();
        storage.init().unwrap();

        // No index yet → empty (a brand-new repo must not error).
        std::fs::remove_file(&storage.index_path).ok();
        assert!(read_local_attributions(&storage).unwrap().is_empty());

        let range = AttributionRange {
            range_id: "r1".into(),
            start_line: 1,
            end_line: 10,
            origin: Origin::Ai,
            evidence_strength: EvidenceStrength::Recorded,
            confidence: 0.9,
            state: AttributionState::Exact,
            session_id: "s1".into(),
            event_ids: vec![],
            agent_id: "claude".into(),
            model_id: None,
            prompt_hash: None,
            context_set_id: None,
            policy_tags: vec![],
            risk_tags: vec![],
            risk_level: None,
            tests_run: vec![],
            tests_passed: false,
            reviewer: None,
            reviewed_at: None,
        };
        {
            let index = TraceIndex::open(&storage.index_path).unwrap();
            index
                .index_attribution(&range, "src/a.rs", "blob123", "2026-06-12T00:00:00Z")
                .unwrap();
        }

        let files = read_local_attributions(&storage).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0]["file_path"], "src/a.rs");
        assert_eq!(files[0]["git_blob_sha"], "blob123");
        assert_eq!(files[0]["ranges"][0]["origin"], "ai");
        assert_eq!(files[0]["ranges"][0]["start_line"], 1);

        std::fs::remove_dir_all(&tmp).ok();
    }
}
