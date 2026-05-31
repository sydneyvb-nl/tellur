//! TraceGit Adapters — AI tool integration layer
//!
//! Each adapter knows how to:
//! - Detect its AI tool (is it installed? active?)
//! - Install hooks if supported
//! - Parse tool-specific formats into TraceGit events
//! - Export TraceGit events back to the tool's format

pub mod claude_code;
pub mod aider;
pub mod cursor;
pub mod generic;

pub use claude_code::ClaudeCodeAdapter;
pub use aider::AiderAdapter;
pub use cursor::CursorAdapter;
pub use generic::GenericAdapter;
