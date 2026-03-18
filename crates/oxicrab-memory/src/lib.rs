//! Memory subsystem for the oxicrab framework.
//!
//! This crate provides the memory database, memory store, embedding
//! utilities, quality gates, remember fast-path, and hygiene routines.

pub mod embeddings;
pub mod hygiene;
pub mod memory_db;
pub mod memory_store;
pub mod quality;
pub mod remember;
pub mod session;

pub use memory_db::MemoryDB;
pub use memory_store::MemoryStore;
