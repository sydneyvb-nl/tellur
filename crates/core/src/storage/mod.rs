//! Storage module — event log, SQLite index, repo management

pub mod event_log;
pub mod export;
pub mod file_watcher;
pub mod index;
pub mod repo;

pub use event_log::{EventWriter, read_events};
pub use export::{ExportProfile, export_provenance_bundle};
pub use file_watcher::{FileChange, FileChangeType, capture_git_diff};
pub use index::{SessionSummary, TraceIndex};
pub use repo::RepoStorage;
