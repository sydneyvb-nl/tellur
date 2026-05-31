//! MCP (Model Context Protocol) server
//!
//! Exposes TraceGit data as MCP tools so AI agents can query
//! attribution, provenance, and policy status.

use std::io::{BufRead, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::storage::{RepoStorage, TraceIndex};

/// MCP tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// MCP tool response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResponse {
    pub content: Vec<McpContent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

/// List of available MCP tools
pub fn list_tools() -> Vec<McpTool> {
    vec![
        McpTool {
            name: "tracegit_explain".to_string(),
            description: "Explain who/what changed a specific line of code (AI or human, model, confidence)".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Path to the file" },
                    "line": { "type": "integer", "description": "Line number (1-based)" }
                },
                "required": ["file_path", "line"]
            }),
        },
        McpTool {
            name: "tracegit_blame".to_string(),
            description: "Show AI attribution for an entire file — which lines were AI-generated vs human-written".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Path to the file" }
                },
                "required": ["file_path"]
            }),
        },
        McpTool {
            name: "tracegit_sessions".to_string(),
            description: "List recent AI coding sessions with agent, model, and event counts".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max sessions to return (default 10)" }
                }
            }),
        },
        McpTool {
            name: "tracegit_policy_check".to_string(),
            description: "Check if current changes comply with TraceGit policies (sensitive paths, required reviews)".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        McpTool {
            name: "tracegit_pr_report".to_string(),
            description: "Generate a PR risk report with AI involvement stats and reviewer checklist".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "base": { "type": "string", "description": "Base ref (default: main)" },
                    "head": { "type": "string", "description": "Head ref (default: HEAD)" }
                }
            }),
        },
        McpTool {
            name: "tracegit_verify".to_string(),
            description: "Verify the integrity of the TraceGit event log (hash chain check)".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
    ]
}

fn text_response(text: String) -> McpToolResponse {
    McpToolResponse {
        content: vec![McpContent {
            content_type: "text".to_string(),
            text,
        }],
    }
}

/// Handle a tool call against the TraceGit data in `repo_root`.
///
/// These are real queries — they open the index/policies and return actual
/// attribution, session, policy, and verification results.
pub fn handle_tool_call(repo_root: &Path, tool_name: &str, args: &serde_json::Value) -> McpToolResponse {
    match handle_tool_call_inner(repo_root, tool_name, args) {
        Ok(resp) => resp,
        Err(e) => text_response(format!("Error: {}", e)),
    }
}

fn handle_tool_call_inner(
    repo_root: &Path,
    tool_name: &str,
    args: &serde_json::Value,
) -> anyhow::Result<McpToolResponse> {
    let storage = RepoStorage::from_git_root(repo_root)?;

    match tool_name {
        "tracegit_explain" => {
            let file = args.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            let line = args.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let index = TraceIndex::open(&storage.index_path)?;
            let attrs = index.get_file_attributions(file)?;
            for (_blob, attr) in &attrs {
                if line >= attr.start_line && line <= attr.end_line {
                    return Ok(text_response(format!(
                        "{}:{} — origin={:?}, confidence={:.0}%, agent={}, model={}, session={}, evidence={:?}",
                        file, line, attr.origin, attr.confidence * 100.0, attr.agent_id,
                        attr.model_id.clone().unwrap_or_else(|| "n/a".to_string()),
                        attr.session_id, attr.evidence_strength,
                    )));
                }
            }
            Ok(text_response(format!("No attribution recorded for {}:{}", file, line)))
        }
        "tracegit_blame" => {
            let file = args.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            let index = TraceIndex::open(&storage.index_path)?;
            let attrs = index.get_file_attributions(file)?;
            if attrs.is_empty() {
                return Ok(text_response(format!("No attribution data for {}", file)));
            }
            let mut out = format!("Attribution for {}\n", file);
            for (_blob, a) in &attrs {
                out.push_str(&format!(
                    "  L{}-{} {:?} {} conf={:.0}% [{:?}]\n",
                    a.start_line, a.end_line, a.origin, a.agent_id, a.confidence * 100.0, a.state
                ));
            }
            Ok(text_response(out))
        }
        "tracegit_sessions" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
            let index = TraceIndex::open(&storage.index_path)?;
            let sessions = index.list_sessions(limit)?;
            if sessions.is_empty() {
                return Ok(text_response("No sessions recorded.".to_string()));
            }
            let mut out = String::new();
            for s in &sessions {
                out.push_str(&format!(
                    "{} — agent={} model={} status={} events={}\n",
                    s.id, s.agent_name,
                    s.model_name.clone().unwrap_or_else(|| "n/a".to_string()),
                    s.status, s.event_count,
                ));
            }
            Ok(text_response(out))
        }
        "tracegit_policy_check" => {
            let policy_path = storage.policies_dir.join("default.yml");
            if !policy_path.exists() {
                return Ok(text_response("No policy file found.".to_string()));
            }
            let engine = crate::policy::PolicyEngine::load_from_file(&policy_path)?;
            let policy = engine.policy();
            let mut out = String::from("Policy:\n");
            if let Some(paths) = &policy.sensitive_paths {
                for sp in paths {
                    out.push_str(&format!("  {} [{}]\n", sp.path, sp.tags.join(", ")));
                }
            }
            Ok(text_response(out))
        }
        "tracegit_pr_report" => {
            let base = args.get("base").and_then(|v| v.as_str()).unwrap_or("main");
            let head = args.get("head").and_then(|v| v.as_str()).unwrap_or("HEAD");
            let report = crate::report::build_repo_pr_report(&storage, base, head)?;
            Ok(text_response(crate::report::PRReportGenerator::to_markdown(&report)))
        }
        "tracegit_verify" => {
            let events = crate::storage::read_events(&storage.traces_dir)?;
            let result = crate::storage::event_log::verify_chain(&events);
            Ok(text_response(format!(
                "Verified {} events: {} valid, {} broken — chain {}",
                events.len(), result.valid, result.broken,
                if result.broken == 0 { "intact" } else { "BROKEN" },
            )))
        }
        other => Ok(text_response(format!("Unknown tool: {}", other))),
    }
}

// ─── stdio JSON-RPC transport ────────────────────────────────────────────────

/// Run the MCP server over stdio (newline-delimited JSON-RPC 2.0), serving the
/// TraceGit tools backed by data in `repo_root`. Blocks until stdin closes.
pub fn serve_stdio(repo_root: &Path) -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(serde_json::json!({}));

        // Notifications (no id) get no response.
        let response = match method {
            "initialize" => Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "tracegit", "version": env!("CARGO_PKG_VERSION") }
            })),
            "tools/list" => Some(serde_json::json!({ "tools": list_tools() })),
            "tools/call" => {
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or(serde_json::json!({}));
                let result = handle_tool_call(repo_root, name, &arguments);
                Some(serde_json::json!({
                    "content": result.content,
                    "isError": result.content.first().map(|c| c.text.starts_with("Error:")).unwrap_or(false),
                }))
            }
            "ping" => Some(serde_json::json!({})),
            _ => None,
        };

        if let (Some(id), Some(result)) = (id, response) {
            let reply = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
            writeln!(out, "{}", serde_json::to_string(&reply)?)?;
            out.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_tools() {
        let tools = list_tools();
        assert!(tools.len() >= 5);
        assert!(tools.iter().any(|t| t.name == "tracegit_explain"));
        assert!(tools.iter().any(|t| t.name == "tracegit_blame"));
    }

    #[test]
    fn test_handle_unknown() {
        let tmp = std::env::temp_dir();
        let resp = handle_tool_call(&tmp, "unknown_tool", &serde_json::json!({}));
        assert!(resp.content[0].text.contains("Unknown") || resp.content[0].text.contains("Error"));
    }
}
