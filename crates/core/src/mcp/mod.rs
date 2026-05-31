//! MCP (Model Context Protocol) server
//!
//! Exposes TraceGit data as MCP tools so AI agents can query
//! attribution, provenance, and policy status.

use serde::{Deserialize, Serialize};

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

/// Handle a tool call
pub fn handle_tool_call(tool_name: &str, args: &serde_json::Value) -> McpToolResponse {
    match tool_name {
        "tracegit_explain" => {
            let file = args.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            let line = args.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
            McpToolResponse {
                content: vec![McpContent {
                    content_type: "text".to_string(),
                    text: format!("Querying attribution for {}:{}...", file, line),
                }],
            }
        }
        "tracegit_blame" => {
            let file = args.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            McpToolResponse {
                content: vec![McpContent {
                    content_type: "text".to_string(),
                    text: format!("Querying attribution for {}...", file),
                }],
            }
        }
        "tracegit_sessions" => McpToolResponse {
            content: vec![McpContent {
                content_type: "text".to_string(),
                text: "Listing sessions...".to_string(),
            }],
        },
        "tracegit_policy_check" => McpToolResponse {
            content: vec![McpContent {
                content_type: "text".to_string(),
                text: "Checking policies...".to_string(),
            }],
        },
        "tracegit_pr_report" => McpToolResponse {
            content: vec![McpContent {
                content_type: "text".to_string(),
                text: "Generating PR report...".to_string(),
            }],
        },
        "tracegit_verify" => McpToolResponse {
            content: vec![McpContent {
                content_type: "text".to_string(),
                text: "Verifying event log integrity...".to_string(),
            }],
        },
        _ => McpToolResponse {
            content: vec![McpContent {
                content_type: "text".to_string(),
                text: format!("Unknown tool: {}", tool_name),
            }],
        },
    }
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
    fn test_handle_explain() {
        let resp = handle_tool_call("tracegit_explain", &serde_json::json!({
            "file_path": "src/main.rs",
            "line": 42
        }));
        assert!(resp.content[0].text.contains("src/main.rs"));
    }

    #[test]
    fn test_handle_unknown() {
        let resp = handle_tool_call("unknown_tool", &serde_json::json!({}));
        assert!(resp.content[0].text.contains("Unknown"));
    }
}
