//! Tellur Adapters — AI tool integration layer
//!
//! Each adapter knows how to:
//! - Detect its AI tool (is it installed? active?)
//! - Install hooks if supported
//! - Parse tool-specific formats into Tellur events
//! - Export Tellur events back to the tool's format
//!
//! Import adapters share the tolerant JSONL/array/envelope parsing loop in
//! [`import`]; each adapter only owns its tool-specific event-type mapping.

pub mod aider;
pub mod antigravity;
pub mod claude_code;
pub mod cline;
pub mod codex;
pub mod continue_dev;
pub mod copilot;
pub mod cursor;
pub mod devin;
pub mod gemini;
pub mod generic;
pub mod import;
pub mod jetbrains;
mod sanitize;
pub mod windsurf;

pub use aider::AiderAdapter;
pub use antigravity::AntigravityAdapter;
pub use claude_code::ClaudeCodeAdapter;
pub use cline::ClineAdapter;
pub use codex::CodexAdapter;
pub use continue_dev::ContinueAdapter;
pub use copilot::CopilotAdapter;
pub use cursor::CursorAdapter;
pub use devin::DevinAdapter;
pub use gemini::GeminiAdapter;
pub use generic::GenericAdapter;
pub use jetbrains::JetBrainsAdapter;
pub use windsurf::WindsurfAdapter;
