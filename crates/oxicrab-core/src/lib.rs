//! Core types and traits for the oxicrab framework.
//!
//! This crate provides foundational types that are shared across
//! the oxicrab workspace: error types, time utilities, and bus events.

pub mod bus;
pub mod channels;
pub mod config;
pub mod credential_store;
pub mod cron_types;
pub mod dispatch;
pub mod errors;
pub mod providers;
pub mod safety;
pub mod time;
pub mod tools;
pub mod utils;
