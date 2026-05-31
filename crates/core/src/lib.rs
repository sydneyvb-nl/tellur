//! Tellur Core — AI Code Provenance Engine
//!
//! This crate provides the foundational types, schemas, storage, attribution engine,
//! redaction, and policy evaluation for Tellur.

pub mod adapter;
pub mod attribution;
pub mod capture;
pub mod daemon;
pub mod export;
pub mod glob;
pub mod mcp;
pub mod notes;
pub mod policy;
pub mod redaction;
pub mod remap;
pub mod report;
pub mod schema;
pub mod storage;

pub use schema::types::*;
