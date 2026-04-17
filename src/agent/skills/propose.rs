//! Track 2b: `skill_propose` tool.
//!
//! Lets the agent stage a candidate skill file under
//! `~/.oxicrab/skills/staged/` for operator approval. Promoting a
//! staged skill to active runs `scan_skill` again before moving the
//! file into the live directory.

use crate::agent::skills::scanner;
use anyhow::{Context, Result, anyhow};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Hard cap on the size of a staged skill file read during promotion.
/// Bounds memory consumption if an attacker swaps the staged file
/// for something huge between propose and promote.
const MAX_STAGED_READ_BYTES: usize = 64 * 1024;

/// Maximum size of a proposed skill file.
const MAX_PROPOSED_SKILL_BYTES: u64 = 32_768;

/// Validate that a skill name is safe to use as a filesystem path
/// component.
fn validate_skill_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(anyhow!("skill name must not be empty"));
    }
    if name.len() > 64 {
        return Err(anyhow!("skill name must be ≤64 chars"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(anyhow!(
            "skill name must contain only alphanumeric, '_', and '-'"
        ));
    }
    if name.starts_with('-') || name.starts_with('_') {
        return Err(anyhow!("skill name must not start with '-' or '_'"));
    }
    Ok(())
}

/// Propose a new skill by writing the body to the staged directory.
/// The file lives at `<staged_dir>/<name>.md` and is **not** loaded
/// into agent context until promoted.
pub fn propose_skill(workspace_skills: &Path, name: &str, body: &str) -> Result<PathBuf> {
    validate_skill_name(name)?;

    if (body.len() as u64) > MAX_PROPOSED_SKILL_BYTES {
        return Err(anyhow!(
            "proposed skill body too large ({} > {} bytes)",
            body.len(),
            MAX_PROPOSED_SKILL_BYTES
        ));
    }

    // Defence in depth: scan the proposed body before writing. Blocked
    // patterns refuse the proposal entirely.
    let scan = scanner::scan_skill(body);
    if !scan.is_clean() {
        for finding in &scan.blocked {
            warn!(
                "skill_propose: blocked '{}' ({}:{}): {} at line {}",
                name,
                finding.category,
                finding.pattern_name,
                finding.matched_text,
                finding.line_number
            );
        }
        if !scan.blocked.is_empty() {
            return Err(anyhow!(
                "proposed skill rejected: matches {} blocked safety pattern(s)",
                scan.blocked.len()
            ));
        }
    }

    let staged_dir = workspace_skills.join("staged");
    std::fs::create_dir_all(&staged_dir)
        .with_context(|| format!("creating staged dir at {}", staged_dir.display()))?;
    let path = staged_dir.join(format!("{name}.md"));
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
    info!("skill_propose: staged '{}' at {}", name, path.display());
    Ok(path)
}

/// Promote a staged skill to active by moving it into a per-skill
/// directory.
///
/// Hardened against TOCTOU: opens the staged file with
/// `std::fs::symlink_metadata` to verify it is a regular file (not a
/// symlink), reads the bounded content, scans, and then moves. The
/// symlink check is critical — without it, an attacker who can write
/// to the staged dir could swap the file for a symlink to an
/// arbitrary path between propose and promote.
pub fn promote_staged_skill(workspace_skills: &Path, name: &str) -> Result<PathBuf> {
    validate_skill_name(name)?;
    let staged_path = workspace_skills.join("staged").join(format!("{name}.md"));

    // symlink_metadata does NOT follow symlinks (unlike metadata).
    let meta = std::fs::symlink_metadata(&staged_path).with_context(|| {
        format!(
            "no staged skill named '{name}' at {}",
            staged_path.display()
        )
    })?;
    if meta.file_type().is_symlink() {
        return Err(anyhow!(
            "staged skill '{name}' is a symlink — refusing to promote (potential TOCTOU)"
        ));
    }
    if !meta.is_file() {
        return Err(anyhow!(
            "staged skill '{name}' is not a regular file (mode {:?})",
            meta.file_type()
        ));
    }
    if meta.len() as usize > MAX_STAGED_READ_BYTES {
        return Err(anyhow!(
            "staged skill '{name}' too large to promote ({} > {} bytes)",
            meta.len(),
            MAX_STAGED_READ_BYTES
        ));
    }

    // Open and read with a bounded length. Open(read) on the path can
    // still race with a symlink swap between symlink_metadata and
    // open — read the bytes immediately and re-validate file type
    // from the opened handle.
    let mut file =
        File::open(&staged_path).with_context(|| format!("opening {}", staged_path.display()))?;
    let opened_meta = file
        .metadata()
        .with_context(|| format!("stat after open {}", staged_path.display()))?;
    if !opened_meta.is_file() {
        return Err(anyhow!(
            "staged skill '{name}' lost its regular-file type between stat and open"
        ));
    }
    let mut content = String::new();
    let mut buf = vec![0u8; MAX_STAGED_READ_BYTES + 1];
    let n = file
        .read(&mut buf)
        .with_context(|| format!("reading {}", staged_path.display()))?;
    if n > MAX_STAGED_READ_BYTES {
        return Err(anyhow!(
            "staged skill '{name}' grew past {MAX_STAGED_READ_BYTES} bytes during read"
        ));
    }
    content.push_str(
        std::str::from_utf8(&buf[..n])
            .map_err(|e| anyhow!("staged skill '{name}' is not valid UTF-8: {e}"))?,
    );

    let scan = scanner::scan_skill(&content);
    if !scan.blocked.is_empty() {
        for finding in &scan.blocked {
            warn!(
                "skill_propose: blocked on promotion '{}' ({}:{}): {} at line {}",
                name,
                finding.category,
                finding.pattern_name,
                finding.matched_text,
                finding.line_number
            );
        }
        return Err(anyhow!(
            "staged skill rejected on promotion: {} blocked pattern(s)",
            scan.blocked.len()
        ));
    }

    let target_dir = workspace_skills.join(name);
    std::fs::create_dir_all(&target_dir)
        .with_context(|| format!("creating skill dir {}", target_dir.display()))?;
    let target = target_dir.join(format!("{name}.md"));
    std::fs::rename(&staged_path, &target)
        .with_context(|| format!("moving {} -> {}", staged_path.display(), target.display()))?;
    info!("skill_propose: promoted '{}' to {}", name, target.display());
    Ok(target)
}

/// List names of staged proposals.
pub fn list_staged(workspace_skills: &Path) -> Vec<String> {
    let staged_dir = workspace_skills.join("staged");
    if !staged_dir.exists() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&staged_dir) {
        for entry in entries.flatten() {
            if let Some(file_name) = entry.file_name().to_str()
                && let Some(name) = file_name.strip_suffix(".md")
            {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_skill_name() {
        assert!(validate_skill_name("foo").is_ok());
        assert!(validate_skill_name("foo_bar-baz").is_ok());
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name("_under").is_err());
        assert!(validate_skill_name("-dash").is_err());
        assert!(validate_skill_name("with space").is_err());
        assert!(validate_skill_name("path/traversal").is_err());
        assert!(validate_skill_name("../escape").is_err());
        assert!(validate_skill_name(&"a".repeat(65)).is_err());
    }

    #[test]
    fn propose_then_promote_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let body = "---\nname: test_skill\ndescription: A safe skill\n---\n\nUseful instructions.";
        let staged = propose_skill(dir.path(), "test_skill", body).unwrap();
        assert!(staged.exists());
        assert_eq!(list_staged(dir.path()), vec!["test_skill"]);

        let promoted = promote_staged_skill(dir.path(), "test_skill").unwrap();
        assert!(promoted.exists());
        assert!(!staged.exists(), "staged file should be moved on promote");
        assert!(list_staged(dir.path()).is_empty());
    }

    #[test]
    fn propose_rejects_oversized_body() {
        let dir = tempfile::tempdir().unwrap();
        let body = "x".repeat(MAX_PROPOSED_SKILL_BYTES as usize + 1);
        assert!(propose_skill(dir.path(), "big", &body).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn promote_rejects_symlinked_staged_file() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        // Create a target file outside the skills tree.
        let outside = dir.path().join("outside.md");
        std::fs::write(&outside, "totally fine content").unwrap();

        // Stage a symlink pointing at the outside file.
        let staged_dir = dir.path().join("staged");
        std::fs::create_dir_all(&staged_dir).unwrap();
        let link = staged_dir.join("evil.md");
        symlink(&outside, &link).unwrap();

        // Promotion must refuse — even though the linked-to content is
        // benign, accepting symlinks defeats the propose+promote
        // approval boundary.
        let err = promote_staged_skill(dir.path(), "evil").unwrap_err();
        assert!(
            err.to_string().contains("symlink"),
            "expected symlink rejection, got: {err}"
        );
        // Symlink should still be present (we did not move it).
        assert!(link.exists());
    }

    #[test]
    fn promote_rejects_oversized_staged_file() {
        let dir = tempfile::tempdir().unwrap();
        let staged_dir = dir.path().join("staged");
        std::fs::create_dir_all(&staged_dir).unwrap();
        let big = staged_dir.join("big.md");
        std::fs::write(&big, vec![b'x'; MAX_STAGED_READ_BYTES + 1]).unwrap();
        let err = promote_staged_skill(dir.path(), "big").unwrap_err();
        assert!(
            err.to_string().contains("too large"),
            "expected size rejection, got: {err}"
        );
    }
}
