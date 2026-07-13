//! Editor/agent hook integration: installing Claude Code hooks and the generic
//! stdin hook-payload ingestion entrypoint shared by all supported agents.

use anyhow::{Context, Result};

use tellur_core::capture::{CaptureContext, capture_working_changes_for_paths};
use tellur_core::schema::types::{AgentInfo, ModelInfo, Session};
use tellur_core::storage::{EventWriter, RepoStorage, TraceIndex};

use crate::util::{
    current_actor, load_policy, prompt_excerpt, prompt_redaction_engine, sanitize_id,
};

pub(crate) fn cmd_hooks_install(tool: &str) -> Result<()> {
    let storage = RepoStorage::discover()?;
    if !storage.is_initialized() {
        println!("Tellur not initialized. Run `tellur init` first.");
        return Ok(());
    }
    match tool {
        "claude-code" | "claude" => {
            tellur_adapters::ClaudeCodeAdapter::install_hooks(&storage.root)?;
            println!(
                "✓ Installed Claude Code hooks into {}/.claude/settings.json",
                storage.root.display()
            );
            println!(
                "  PostToolUse (Write|Edit|MultiEdit) and SessionStart now record provenance."
            );
        }
        other => {
            println!("Unknown tool: {}. Supported: claude-code", other);
        }
    }
    Ok(())
}

/// Handle a Claude Code hook payload delivered on stdin: capture the current
/// working-tree changes and attribute them to the AI session.
pub(crate) fn cmd_hooks_claude() -> Result<()> {
    use std::io::Read;
    let storage = match RepoStorage::discover() {
        Ok(s) if s.is_initialized() => s,
        // Never fail a hook — just no-op if Tellur isn't set up here.
        _ => return Ok(()),
    };

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload = tellur_adapters::claude_code::HookPayload::parse(&input)?;
    let session_id = payload
        .session_id
        .clone()
        .unwrap_or_else(tellur_core::schema::ids::generate_session_id);

    let index = TraceIndex::open(&storage.index_path)?;

    // Ensure the session is recorded with the Claude Code agent.
    let repo_id = tellur_core::schema::ids::hash_content(&storage.root.to_string_lossy());
    let mut session = Session::new(
        repo_id,
        current_actor(),
        AgentInfo {
            id: "claude-code".to_string(),
            name: "Claude Code".to_string(),
            version: None,
        },
    );
    session.id = session_id.clone();
    index.index_session(&session)?;

    // SessionStart just records the session; tool events trigger capture.
    let mut writer = EventWriter::new(&storage.traces_dir);
    writer.open()?;

    if payload.hook_event_name.as_deref() == Some("SessionStart") {
        let event = writer.write_event(
            &session_id,
            "session.start",
            "agent",
            serde_json::json!({"tool": "claude-code"}),
            None,
        )?;
        index.index_event(&event)?;
        writer.close();
        return Ok(());
    }

    let policy = load_policy(&storage);
    let ctx = CaptureContext::recorded_ai(&session_id, "claude-code");
    if let Some(file_path) = payload.file_path() {
        let _ = capture_working_changes_for_paths(
            &storage,
            &mut writer,
            &index,
            policy.as_ref(),
            &ctx,
            &[file_path],
        )?;
    }
    writer.close();
    Ok(())
}

#[derive(Debug, Default)]
struct AgentHookPayload {
    session_id: Option<String>,
    hook_event_name: Option<String>,
    tool_name: Option<String>,
    tool_input: Option<serde_json::Value>,
    cwd: Option<String>,
    model: Option<String>,
    prompt: Option<String>,
    message: Option<String>,
    raw: serde_json::Value,
}

impl AgentHookPayload {
    fn parse(input: &str) -> Result<Self> {
        let raw = serde_json::from_str::<serde_json::Value>(input).context("invalid hook JSON")?;
        let tool_input = first_object_value(
            &raw,
            &[
                &["tool_input"],
                &["toolInput"],
                &["input"],
                &["tool", "input"],
                &["tool_use", "input"],
                &["toolUse", "input"],
            ],
        )
        .cloned();
        Ok(Self {
            session_id: first_string(
                &raw,
                &[
                    &["session_id"],
                    &["sessionId"],
                    &["session", "id"],
                    &["conversation_id"],
                    &["conversationId"],
                ],
            )
            .map(ToString::to_string),
            hook_event_name: first_string(
                &raw,
                &[
                    &["hook_event_name"],
                    &["hookEventName"],
                    &["event_name"],
                    &["eventName"],
                    &["event"],
                    &["type"],
                ],
            )
            .map(ToString::to_string),
            tool_name: first_string(
                &raw,
                &[
                    &["tool_name"],
                    &["toolName"],
                    &["tool", "name"],
                    &["tool"],
                    &["name"],
                ],
            )
            .map(ToString::to_string),
            tool_input,
            cwd: first_string(&raw, &[&["cwd"], &["working_dir"], &["workingDir"]])
                .map(ToString::to_string),
            model: first_string(&raw, &[&["model"], &["model_id"], &["modelId"]])
                .map(ToString::to_string),
            prompt: first_string(
                &raw,
                &[
                    &["prompt"],
                    &["user_prompt"],
                    &["userPrompt"],
                    &["input", "prompt"],
                    &["message", "content"],
                ],
            )
            .map(ToString::to_string),
            message: first_string(&raw, &[&["message"]]).map(ToString::to_string),
            raw,
        })
    }

    fn event_name(&self) -> Option<String> {
        self.hook_event_name.clone()
    }

    fn file_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        if let Some(input) = self.tool_input.as_ref() {
            collect_file_paths(input, 5, &mut paths);
            collect_patch_paths(input, &mut paths);
        }
        if let Some(path) = first_string(
            &self.raw,
            &[
                &["file_path"],
                &["filePath"],
                &["tool", "file_path"],
                &["tool", "filePath"],
                &["tool_use", "file_path"],
                &["toolUse", "filePath"],
            ],
        ) {
            paths.push(path.to_string());
        }
        paths.sort();
        paths.dedup();
        paths
    }

    fn command(&self) -> Option<String> {
        self.tool_input
            .as_ref()
            .and_then(|v| find_first_string_key(v, &["command", "cmd"], 3))
            .or_else(|| first_string(&self.raw, &[&["command"], &["cmd"]]))
            .map(ToString::to_string)
    }

    fn prompt_text(&self) -> Option<&str> {
        self.prompt.as_deref().or(self.message.as_deref())
    }
}

fn collect_file_paths(value: &serde_json::Value, max_depth: usize, out: &mut Vec<String>) {
    if max_depth == 0 {
        return;
    }
    match value {
        serde_json::Value::Object(map) => {
            for key in ["file_path", "filePath", "path"] {
                if let Some(path) = map.get(key).and_then(serde_json::Value::as_str) {
                    out.push(path.to_string());
                }
            }
            for key in ["files", "file_paths", "filePaths", "paths"] {
                if let Some(items) = map.get(key).and_then(serde_json::Value::as_array) {
                    out.extend(
                        items
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(ToString::to_string),
                    );
                }
            }
            for child in map.values() {
                collect_file_paths(child, max_depth - 1, out);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_file_paths(child, max_depth - 1, out);
            }
        }
        _ => {}
    }
}

fn collect_patch_paths(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if matches!(key.as_str(), "patch" | "diff" | "input")
                    && let Some(patch) = value.as_str()
                {
                    out.extend(parse_patch_paths(patch));
                }
                collect_patch_paths(value, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_patch_paths(item, out);
            }
        }
        _ => {}
    }
}

fn parse_patch_paths(patch: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in patch.lines() {
        for marker in ["*** Add File: ", "*** Update File: ", "*** Delete File: "] {
            if let Some(path) = line.strip_prefix(marker) {
                paths.push(path.trim().to_string());
            }
        }
        if let Some(rest) = line.strip_prefix("diff --git a/")
            && let Some((path, _)) = rest.split_once(" b/")
        {
            paths.push(path.to_string());
        }
    }
    paths
}

fn first_object_value<'a>(
    value: &'a serde_json::Value,
    paths: &[&[&str]],
) -> Option<&'a serde_json::Value> {
    paths
        .iter()
        .filter_map(|path| json_path(value, path))
        .find(|value| value.is_object())
}

fn first_string<'a>(value: &'a serde_json::Value, paths: &[&[&str]]) -> Option<&'a str> {
    paths
        .iter()
        .filter_map(|path| json_path(value, path))
        .find_map(|value| value.as_str())
}

fn json_path<'a>(mut value: &'a serde_json::Value, path: &[&str]) -> Option<&'a serde_json::Value> {
    for key in path {
        value = value.get(*key)?;
    }
    Some(value)
}

fn find_first_string_key<'a>(
    value: &'a serde_json::Value,
    keys: &[&str],
    max_depth: usize,
) -> Option<&'a str> {
    if max_depth == 0 {
        return None;
    }
    match value {
        serde_json::Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(|value| value.as_str()) {
                    return Some(found);
                }
            }
            map.values()
                .find_map(|value| find_first_string_key(value, keys, max_depth - 1))
        }
        serde_json::Value::Array(items) => items
            .iter()
            .find_map(|value| find_first_string_key(value, keys, max_depth - 1)),
        _ => None,
    }
}

/// Generic hook ingestion entrypoint used by user-level Codex and Claude Code
/// hooks. It is deliberately no-op friendly so global hooks can be installed
/// once and safely run in unrelated directories.
pub(crate) fn cmd_hooks_ingest(source: &str, auto_init: bool, json_response: bool) -> Result<()> {
    use std::io::Read;

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let payload = match AgentHookPayload::parse(&input) {
        Ok(payload) => payload,
        Err(err) => {
            eprintln!("tellur hook ingest ignored invalid payload: {err:#}");
            if json_response {
                println!("{{}}");
            }
            return Ok(());
        }
    };

    if let Some(cwd) = payload.cwd.as_deref() {
        let _ = std::env::set_current_dir(cwd);
    }

    let storage = match RepoStorage::discover() {
        Ok(storage) => storage,
        Err(_) => {
            if json_response {
                println!("{{}}");
            }
            return Ok(());
        }
    };
    if storage.tellur_dir.join("disable").exists() {
        if json_response {
            println!("{{}}");
        }
        return Ok(());
    }
    if !storage.is_initialized() {
        if auto_init {
            storage.init()?;
        } else {
            if json_response {
                println!("{{}}");
            }
            return Ok(());
        }
    }

    let session_id = payload
        .session_id
        .clone()
        .unwrap_or_else(tellur_core::schema::ids::generate_session_id);
    let source = normalize_hook_source(source);
    let agent_name = match source {
        "codex" => "Codex",
        "claude-code" => "Claude Code",
        "windsurf" => "Windsurf / Cascade",
        "jetbrains" => "JetBrains AI / Junie",
        "devin" => "Devin",
        "continue" => "Continue",
        "cline" => "Cline / Roo Code",
        other => other,
    };

    let index = TraceIndex::open(&storage.index_path)?;
    let repo_id = tellur_core::schema::ids::hash_content(&storage.root.to_string_lossy());
    let mut session = Session::new(
        repo_id,
        current_actor(),
        AgentInfo {
            id: source.to_string(),
            name: agent_name.to_string(),
            version: None,
        },
    );
    session.id = session_id.clone();
    if let Some(model) = payload.model.as_deref() {
        session.model = Some(ModelInfo {
            provider: source.to_string(),
            name: model.to_string(),
            version: None,
        });
    }
    index.index_session(&session)?;

    let mut writer = EventWriter::new(&storage.traces_dir);
    writer.open()?;
    let hook_event_owned = payload
        .event_name()
        .unwrap_or_else(|| "unknown".to_string());
    let hook_event_owned = normalize_hook_event_name(&hook_event_owned).to_string();
    let hook_event = hook_event_owned.as_str();
    match hook_event {
        "SessionStart" => {
            let event = writer.write_event(
                &session_id,
                "session.start",
                "agent",
                serde_json::json!({
                    "tool": source,
                    "hook_event_name": hook_event,
                    "model": payload.model,
                }),
                None,
            )?;
            index.index_event(&event)?;
        }
        "UserPromptSubmit" => {
            let mut event_payload = serde_json::json!({
                "tool": source,
                "hook_event_name": hook_event,
                "model": payload.model,
            });
            if let Some(prompt) = payload.prompt_text() {
                event_payload["prompt_hash"] =
                    serde_json::Value::String(tellur_core::schema::ids::hash_content(prompt));
                // Opt-in (`redaction.store_prompt_excerpt`): keep a redacted,
                // length-bounded preview so the timeline can show what was asked.
                // Redaction uses the repo's own rules (+ defaults).
                if let Some(engine) = prompt_redaction_engine(&storage) {
                    event_payload["prompt_excerpt"] =
                        serde_json::Value::String(prompt_excerpt(&engine, prompt));
                }
            }
            let event =
                writer.write_event(&session_id, "user.prompt", "agent", event_payload, None)?;
            index.index_event(&event)?;
        }
        "PreToolUse" => {
            let event = writer.write_event(
                &session_id,
                "tool.pre_call",
                "agent",
                hook_tool_payload(source, hook_event, &payload),
                None,
            )?;
            index.index_event(&event)?;
        }
        "PostToolUse" => {
            let event = writer.write_event(
                &session_id,
                "tool.post_call",
                "agent",
                hook_tool_payload(source, hook_event, &payload),
                None,
            )?;
            index.index_event(&event)?;

            let policy = load_policy(&storage);
            let mut ctx = CaptureContext::recorded_ai(&session_id, source);
            ctx.model_id = payload.model.clone();
            let file_paths = payload.file_paths();
            if !file_paths.is_empty() {
                let _ = capture_working_changes_for_paths(
                    &storage,
                    &mut writer,
                    &index,
                    policy.as_ref(),
                    &ctx,
                    &file_paths,
                )?;
            }
        }
        "Stop" | "SessionEnd" => {
            let event = writer.write_event(
                &session_id,
                "session.end",
                "agent",
                serde_json::json!({
                    "tool": source,
                    "hook_event_name": hook_event,
                }),
                None,
            )?;
            index.index_event(&event)?;
        }
        _ => {
            let event = writer.write_event(
                &session_id,
                &format!("{}.hook.{}", source, sanitize_id(hook_event)),
                "agent",
                hook_tool_payload(source, hook_event, &payload),
                None,
            )?;
            index.index_event(&event)?;
        }
    }
    writer.close();
    if json_response {
        println!("{{}}");
    }
    Ok(())
}

fn normalize_hook_source(source: &str) -> &str {
    match source {
        "claude" | "claude-code" => "claude-code",
        "codex" | "codex-cli" => "codex",
        "gemini" | "gemini-cli" => "gemini-cli",
        "antigravity" | "google-antigravity" => "antigravity",
        "windsurf" | "cascade" => "windsurf",
        "jetbrains" | "junie" | "jetbrains-ai" => "jetbrains",
        "devin" => "devin",
        "continue" | "continue-dev" => "continue",
        "cline" | "roo" | "roo-code" => "cline",
        other => other,
    }
}

fn normalize_hook_event_name(event: &str) -> &str {
    match event {
        "BeforeTool" => "PreToolUse",
        "AfterTool" => "PostToolUse",
        "BeforeAgent" | "BeforeModel" => "UserPromptSubmit",
        "AfterAgent" => "SessionEnd",
        other => other,
    }
}

fn hook_tool_payload(
    source: &str,
    hook_event: &str,
    payload: &AgentHookPayload,
) -> serde_json::Value {
    let mut out = serde_json::json!({
        "tool": source,
        "hook_event_name": hook_event,
        "tool_name": payload.tool_name,
        "model": payload.model,
    });
    let file_paths = payload.file_paths();
    if let Some(file_path) = file_paths.first() {
        out["file_path"] = serde_json::Value::String(file_path.clone());
    }
    if file_paths.len() > 1 {
        out["file_paths"] = serde_json::json!(file_paths);
    }
    if let Some(command) = payload.command() {
        out["command"] = serde_json::Value::String(redact_hook_string(&command));
    }
    out
}

fn redact_hook_string(value: &str) -> String {
    tellur_core::redaction::RedactionEngine::default_engine()
        .scan_and_redact(value)
        .redacted_content
        .unwrap_or_else(|| "[REDACTED]".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_apply_patch_payload_exposes_all_changed_paths() {
        let payload = AgentHookPayload::parse(
            &serde_json::json!({
                "hook_event_name": "PostToolUse",
                "tool_name": "apply_patch",
                "tool_input": {
                    "patch": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n*** Add File: docs/new.md\n@@\n*** End Patch"
                }
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(payload.file_paths(), vec!["docs/new.md", "src/lib.rs"]);
    }

    #[test]
    fn editor_file_arrays_are_collected_without_duplicates() {
        let payload = AgentHookPayload::parse(
            &serde_json::json!({
                "tool_input": {
                    "files": ["src/a.ts", "src/b.ts"],
                    "result": {"file_path": "src/a.ts"}
                }
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(payload.file_paths(), vec!["src/a.ts", "src/b.ts"]);
    }
}
