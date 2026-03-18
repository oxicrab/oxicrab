// Re-export from oxicrab-core so all existing `use crate::errors::OxicrabError` continues to work.
pub use oxicrab_core::errors::OxicrabError;

#[cfg(test)]
mod tests;
