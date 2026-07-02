use super::cli_types::MemoryCommands;
use anyhow::Result;

pub(super) fn memory_command(cmd: &MemoryCommands) -> Result<()> {
    let db_path = crate::utils::get_memory_db_path()?;

    if !db_path.exists() {
        anyhow::bail!(
            "memory database not found at {}. Run the agent first to initialize it.",
            db_path.display()
        );
    }

    let db = crate::agent::memory::MemoryDB::new(&db_path)?;

    match cmd {
        MemoryCommands::Snapshot { label } => {
            let id = db.snapshot_memory(label)?;
            println!("Captured memory snapshot #{id} (label: {label}).");
        }
        MemoryCommands::Snapshots { limit } => {
            let snapshots = db.list_snapshots(*limit)?;
            if snapshots.is_empty() {
                println!("No memory snapshots recorded.");
                return Ok(());
            }
            println!(
                "{:<5} {:<20} {:>8} {:<12} Label",
                "ID", "Created", "Entries", "Hash"
            );
            println!("{}", "\u{2500}".repeat(72));
            for s in &snapshots {
                let created = chrono::DateTime::from_timestamp_millis(s.created_at_ms).map_or_else(
                    || s.created_at_ms.to_string(),
                    |dt| dt.format("%Y-%m-%d %H:%M:%S").to_string(),
                );
                let short_hash = s.content_sha256.chars().take(10).collect::<String>();
                println!(
                    "{:<5} {:<20} {:>8} {:<12} {}",
                    s.id, created, s.entry_count, short_hash, s.label
                );
            }
        }
        MemoryCommands::Restore { id } => {
            let outcome = db.restore_snapshot(*id)?;
            println!(
                "Restored memory to snapshot #{id} ({} entries). \
                 A pre-restore snapshot was saved as #{} — run \
                 `oxicrab memory restore {}` to undo.",
                outcome.restored_entries,
                outcome.pre_restore_snapshot_id,
                outcome.pre_restore_snapshot_id,
            );
        }
        MemoryCommands::DeleteSnapshot { id } => {
            if db.delete_snapshot(*id)? {
                println!("Deleted memory snapshot #{id}.");
            } else {
                anyhow::bail!("snapshot #{id} not found");
            }
        }
    }

    Ok(())
}
