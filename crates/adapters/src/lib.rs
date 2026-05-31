//! TraceGit Adapters
//!
//! AI tool adapter implementations for importing and capturing
//! development activity from various AI coding tools.

pub mod claude_code;
pub mod generic;

pub use claude_code::ClaudeCodeAdapter;
pub use generic::GenericAdapter;
