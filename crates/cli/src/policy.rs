//! Policy commands: `policy check`, `policy explain`, and `policy pull`.

use std::path::Path;

use anyhow::{Context, Result};

use tellur_core::storage::RepoStorage;

pub(crate) fn cmd_policy_check() -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }

    let policy_path = storage.policies_dir.join("default.yml");
    if !policy_path.exists() {
        println!("No policy file found.");
        return Ok(());
    }

    let engine = tellur_core::policy::PolicyEngine::load_from_file(&policy_path)?;
    let policy = engine.policy();

    println!("Policy Check");
    println!("════════════");
    println!();

    if let Some(ref paths) = policy.sensitive_paths {
        println!("Sensitive paths ({}):", paths.len());
        for sp in paths {
            println!("  {} [{}]", sp.path, sp.tags.join(", "));
        }
    }

    if let Some(ref rules) = policy.rules {
        if rules.is_empty() {
            println!("Custom rules: none");
        } else {
            println!("Custom rules ({}):", rules.len());
            for rule in rules {
                println!("  {} — {}", rule.id, rule.description);
            }
        }
    }

    Ok(())
}

pub(crate) fn cmd_policy_explain(rule_id: Option<&str>) -> Result<()> {
    let storage = RepoStorage::discover()?;
    let policy_path = storage.policies_dir.join("default.yml");
    if !policy_path.exists() {
        println!("No policy file found.");
        return Ok(());
    }

    let engine = tellur_core::policy::PolicyEngine::load_from_file(&policy_path)?;
    let policy = engine.policy();

    if let Some(id) = rule_id {
        if let Some(ref rules) = policy.rules {
            if let Some(rule) = rules.iter().find(|r| r.id == id) {
                println!("Rule: {}", rule.id);
                println!("Description: {}", rule.description);
                if let Some(ref rationale) = rule.rationale {
                    println!("Rationale: {}", rationale);
                }
                println!("Action: {:?}", rule.action);
                println!("When: {}", serde_json::to_string_pretty(&rule.when)?);
            } else {
                println!("Rule '{}' not found.", id);
            }
        }
    } else {
        println!("Available rules:");
        if let Some(ref rules) = policy.rules {
            for rule in rules {
                println!("  {} — {}", rule.id, rule.description);
            }
        }
        if policy.rules.is_none() || policy.rules.as_ref().map(|r| r.is_empty()).unwrap_or(true) {
            println!("  (no custom rules defined)");
        }
    }

    Ok(())
}

/// Pull a central policy from a Tellur team hub (Tier 0/Tier 1 distribution) and
/// write it into this repo's `.tellur/policies/`. Validates the content before
/// writing so a broken policy is never installed.
pub(crate) fn cmd_policy_pull(
    org: &str,
    name: &str,
    hub: Option<&str>,
    token: Option<&str>,
    out: Option<&Path>,
) -> Result<()> {
    let hub = hub
        .map(str::to_string)
        .or_else(|| std::env::var("TELLUR_HUB_URL").ok())
        .context("hub URL required (--hub or TELLUR_HUB_URL)")?;
    let token = token
        .map(str::to_string)
        .or_else(|| std::env::var("TELLUR_HUB_TOKEN").ok())
        .context("hub token required (--token or TELLUR_HUB_TOKEN)")?;

    let url = format!(
        "{}/v1/orgs/{}/policies/{}",
        hub.trim_end_matches('/'),
        org,
        name
    );
    let body = ureq::get(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| anyhow::anyhow!("policy pull request failed: {e}"))?
        .into_string()
        .context("failed to read hub response")?;
    let parsed: serde_json::Value =
        serde_json::from_str(&body).context("hub response was not valid JSON")?;
    let content = parsed["content"]
        .as_str()
        .context("hub response missing policy content")?;

    // Validate before writing — never install a broken policy.
    tellur_core::policy::PolicyEngine::from_yaml_str(content)
        .context("hub returned invalid policy YAML")?;

    let out_path = match out {
        Some(p) => p.to_path_buf(),
        None => {
            let storage = RepoStorage::discover()?;
            storage.policies_dir.join(format!("{name}.yml"))
        }
    };
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out_path, content)?;
    println!(
        "Pulled policy '{}' (version {}) → {}",
        name,
        parsed["version"],
        out_path.display()
    );
    Ok(())
}
