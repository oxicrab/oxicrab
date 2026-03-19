//! Obsidian tool for the oxicrab framework.

pub mod obsidian;
mod utils;

pub use obsidian::{ObsidianSyncService, ObsidianTool};

use oxicrab_core::tools::base::Tool;
use oxicrab_memory::memory_db::MemoryDB;
use std::sync::Arc;

/// Create the Obsidian tool.
///
/// Returns `(tool, cache)` on success, where `cache` is needed by `ObsidianSyncService`.
pub fn create_obsidian_tool(
    api_url: &str,
    api_key: &str,
    vault_name: &str,
    timeout: u64,
    db: Option<Arc<MemoryDB>>,
) -> anyhow::Result<(Arc<dyn Tool>, Arc<obsidian::cache::ObsidianCache>)> {
    let (tool, cache) = ObsidianTool::new(api_url, api_key, vault_name, timeout, db)?;
    Ok((Arc::new(tool), cache))
}
