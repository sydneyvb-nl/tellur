//! Storage module — event log, SQLite index, repo management

pub mod event_log;
pub mod export;
pub mod file_watcher;
pub mod index;
pub mod repo;

pub use event_log::{EventWriter, read_events};
pub use export::{export_provenance_bundle, ExportProfile};
pub use file_watcher::{capture_git_diff, FileChange, FileChangeType};
pub use index::TraceIndex;
pub use repo::RepoStorage;
