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
];
