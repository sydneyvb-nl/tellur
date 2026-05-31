//! TraceGit Core — AI Code Provenance Engine
//!
//! This crate provides the foundational types, schemas, storage, attribution engine,
//! redaction, and policy evaluation for TraceGit.

pub mod adapter;
pub mod attribution;
pub mod policy;
pub mod redaction;
pub mod report;
pub mod schema;
pub mod storage;

pub use schema::types::*;
