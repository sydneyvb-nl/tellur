//! Built-in adapter definitions
//!
//! Static info about the adapters that ship with Tellur.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltinAdapter {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub supports_hooks: bool,
}

pub const BUILTIN_ADAPTERS: &[BuiltinAdapter] = &[
    BuiltinAdapter {
        id: "claude-code",
        name: "Claude Code",
        description: "Anthropic's CLI coding agent",
        supports_hooks: true,
    },
    BuiltinAdapter {
        id: "aider",
        name: "Aider",
        description: "AI pair programming in your terminal",
        supports_hooks: false,
    },
    BuiltinAdapter {
        id: "cursor",
        name: "Cursor",
        description: "AI-first code editor",
        supports_hooks: false,
    },
    BuiltinAdapter {
        id: "gemini-cli",
        name: "Gemini CLI",
        description: "Google Gemini CLI hooks and JSONL import",
        supports_hooks: true,
    },
    BuiltinAdapter {
        id: "antigravity",
        name: "Google Antigravity",
        description: "Google Antigravity 2.0 hooks, MCP, and JSONL import",
        supports_hooks: true,
    },
    BuiltinAdapter {
        id: "generic",
        name: "Generic",
        description: "CLI and HTTP event ingestion for any tool",
        supports_hooks: false,
    },
    BuiltinAdapter {
        id: "codex",
        name: "Codex CLI",
        description: "OpenAI Codex CLI JSONL event stream import",
        supports_hooks: false,
    },
    BuiltinAdapter {
        id: "copilot",
        name: "GitHub Copilot",
        description: "GitHub Copilot metadata import",
        supports_hooks: false,
    },
    BuiltinAdapter {
        id: "windsurf",
        name: "Windsurf",
        description: "Windsurf / Cascade agent session JSONL import",
        supports_hooks: false,
    },
    BuiltinAdapter {
        id: "jetbrains",
        name: "JetBrains AI / Junie",
        description: "JetBrains AI Assistant and Junie action-log import",
        supports_hooks: false,
    },
    BuiltinAdapter {
        id: "devin",
        name: "Devin",
        description: "Devin cloud agent run/session export import",
        supports_hooks: false,
    },
    BuiltinAdapter {
        id: "continue",
        name: "Continue",
        description: "Continue dev_data JSONL import",
        supports_hooks: false,
    },
    BuiltinAdapter {
        id: "cline",
        name: "Cline / Roo Code",
        description: "Cline and Roo Code task-history import",
        supports_hooks: false,
    },
];
