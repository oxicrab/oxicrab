/// Memory hygiene: purge old search logs and clean up workspace files.
use crate::agent::memory::MemoryDB;
use anyhow::Result;
use tracing::{info, warn};

/// Clean up workspace files that have exceeded their category TTL.
pub fn cleanup_workspace_files<S: ::std::hash::BuildHasher>(
    db: &MemoryDB,
    workspace_root: &std::path::Path,
    ttl_map: &std::collections::HashMap<String, Option<u64>, S>,
) -> Result<u32> {
    let mut total_removed = 0;
    for (category, ttl) in ttl_map {
        let Some(days) = ttl else { continue };
        let expired = db.list_expired_workspace_files(category, *days as u32)?;
        for entry in expired {
            let abs_path = workspace_root.join(&entry.path);
            if abs_path.exists()
                && let Err(e) = std::fs::remove_file(&abs_path)
            {
                warn!(
                    "failed to remove expired workspace file {}: {e}",
                    abs_path.display()
                );
                continue;
            }
            db.unregister_workspace_file(&entry.path)?;
            total_removed += 1;
        }
    }
    if total_removed > 0 {
        info!("cleaned up {} expired workspace files", total_removed);
    }
    Ok(total_removed)
}

/// Run all hygiene tasks (purge old search logs, intent metrics,
/// complexity routing logs, and cost logs).
pub fn run_hygiene(db: &MemoryDB, purge_log_days: u32) {
    match db.purge_old_search_logs(purge_log_days) {
        Ok(n) if n > 0 => info!("purged {} old search log entries", n),
        Err(e) => warn!("search log purge failed: {}", e),
        _ => {}
    }
    match db.purge_old_intent_metrics(purge_log_days) {
        Ok(n) if n > 0 => info!("purged {} old intent metric entries", n),
        Err(e) => warn!("intent metrics purge failed: {}", e),
        _ => {}
    }
    match db.purge_old_complexity_logs(purge_log_days) {
        Ok(n) if n > 0 => info!("purged {} old complexity routing log entries", n),
        Err(e) => warn!("complexity log purge failed: {}", e),
        _ => {}
    }
    match db.purge_old_cost_logs(purge_log_days) {
        Ok(n) if n > 0 => info!("purged {} old cost log entries", n),
        Err(e) => warn!("cost log purge failed: {}", e),
        _ => {}
    }
    // Purge old memory entries (keep knowledge: prefixed sources).
    // Use a longer retention period than log tables since memory entries
    // are more valuable — 180 days by default.
    match db.purge_old_memory_entries(180) {
        Ok(n) if n > 0 => info!("purged {} old memory entries", n),
        Err(e) => warn!("memory entry purge failed: {}", e),
        _ => {}
    }
}

#[cfg(test)]
mod tests;
