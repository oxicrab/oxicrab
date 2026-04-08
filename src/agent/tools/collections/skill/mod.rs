use crate::agent::memory::memory_db::{FieldDef, FieldType};
use std::fmt::Write;
use tracing::warn;

use super::format_field_type;

#[cfg(test)]
mod tests;

/// Generate skill markdown content for a collection.
pub fn generate_collection_skill(name: &str, description: &str, fields: &[FieldDef]) -> String {
    let mut s = String::with_capacity(512);

    let _ = writeln!(s, "# {name} Collection\n");
    let _ = writeln!(s, "{description}\n");
    let _ = writeln!(s, "## Fields");
    for f in fields {
        let mut line = format!("- **{}** ({}", f.name, format_field_type(&f.field_type),);
        if f.required {
            line.push_str(", required");
        }
        if !f.values.is_empty() && matches!(f.field_type, FieldType::Enum) {
            let _ = write!(line, ", values: {:?}", f.values);
        }
        line.push(')');
        let _ = writeln!(s, "{line}");
    }

    let _ = writeln!(s, "\n## Usage");
    let _ = writeln!(
        s,
        "- To add an item: use the `{name}` tool with action \"add\""
    );
    let _ = writeln!(
        s,
        "- To search: use the `{name}` tool with action \"query\" and filters"
    );
    let _ = writeln!(
        s,
        "- To update: use the `{name}` tool with action \"update\" with the record id"
    );
    let _ = writeln!(
        s,
        "- To remove: use the `{name}` tool with action \"delete\" with the record id"
    );
    let _ = writeln!(
        s,
        "- To aggregate: use the `{name}` tool with action \"aggregate\""
    );

    s
}

/// Write a collection skill file to the skills directory.
pub fn write_collection_skill(name: &str, description: &str, fields: &[FieldDef]) {
    let Some(base) = skills_dir() else {
        return;
    };

    let dir = base.join(format!("collection_{name}"));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        warn!("failed to create skill dir for collection '{name}': {e}");
        return;
    }

    let content = generate_collection_skill(name, description, fields);
    let file = dir.join(format!("collection_{name}.md"));
    if let Err(e) = std::fs::write(&file, content) {
        warn!("failed to write skill file for collection '{name}': {e}");
    }
}

/// Remove a collection skill file.
pub fn remove_collection_skill(name: &str) {
    let Some(base) = skills_dir() else {
        return;
    };

    let dir = base.join(format!("collection_{name}"));
    if dir.exists()
        && let Err(e) = std::fs::remove_dir_all(&dir)
    {
        warn!("failed to remove skill dir for collection '{name}': {e}");
    }
}

fn skills_dir() -> Option<std::path::PathBuf> {
    crate::utils::get_oxicrab_home()
        .ok()
        .map(|h| h.join("skills"))
}
