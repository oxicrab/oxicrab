// Re-export from oxicrab-core so all existing `use crate::errors::OxicrabError` continues to work.
// The re-export is gated on cfg(test) since non-test code now accesses OxicrabError through
// oxicrab_core directly or via the oxicrab-providers crate.
#[cfg(test)]
pub(crate) use oxicrab_core::errors::OxicrabError;

#[cfg(test)]
mod tests;
