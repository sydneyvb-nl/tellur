//! Tellur Adapters — AI tool integration layer
//!
//! Each adapter knows how to:
//! - Detect its AI tool (is it installed? active?)
//! - Install hooks if supported
//! - Parse tool-specific formats into Tellur events
//! - Export Tellur events back to the tool's format

pub mod aider;
pub mod antigravity;
pub mod claude_code;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod gemini;
pub mod generic;
mod sanitize;

pub use aider::AiderAdapter;
pub use antigravity::AntigravityAdapter;
pub use claude_code::ClaudeCodeAdapter;
pub use codex::CodexAdapter;
pub use copilot::CopilotAdapter;
pub use cursor::CursorAdapter;
pub use gemini::GeminiAdapter;
pub use generic::GenericAdapter;
