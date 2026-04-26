// Re-export from oxicrab-providers so all existing `use crate::providers::*` continues to work.
pub use oxicrab_providers::*;

pub mod base {
    // Re-export from oxicrab-core so all existing `use crate::providers::base::*` continues to work.
    pub use oxicrab_core::providers::base::*;
}

pub mod fixture {
    pub use oxicrab_core::providers::fixture::*;
}

pub mod streaming {
    pub use oxicrab_core::streaming::*;
}
