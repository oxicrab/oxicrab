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

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

/// Hard cap on the size of a staged skill file read during promotion.
/// Bounds memory consumption if an attacker swaps the staged file
/// for something huge between propose and promote.
const MAX_STAGED_READ_BYTES: usize = 64 * 1024;

/// Maximum size of a proposed skill file.
const MAX_PROPOSED_SKILL_BYTES: u64 = 32_768;

/// Open `path` for reading and refuse to follow symlinks at the kernel
/// level. On Unix this uses `O_NOFOLLOW`; on other platforms the
/// regular `File::open` is used (callers must rely on the prior
/// `symlink_metadata` check there).
fn open_no_follow(path: &Path) -> std::io::Result<File> {
    #[cfg(unix)]
    {
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        File::open(path)
    }
}

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

    // Open the staged file. On Unix we set `O_NOFOLLOW` so the kernel
    // refuses any symlink at open time — `symlink_metadata` above only
    // catches the link before the open syscall, and a `file.metadata()`
    // (fstat) check on the resulting handle returns the *target's*
    // attributes, so an attacker who swaps the regular file for a
    // symlink-to-regular-file in the race window between `stat` and
    // `open` would otherwise go undetected. `O_NOFOLLOW` closes that
    // residual TOCTOU window. Non-Unix platforms keep the path-based
    // open and rely on `symlink_metadata` alone.
    let mut file = open_no_follow(&staged_path)
        .with_context(|| format!("opening {}", staged_path.display()))?;
    let opened_meta = file
        .metadata()
        .with_context(|| format!("stat after open {}", staged_path.display()))?;
    if !opened_meta.is_file() {
        return Err(anyhow!(
            "staged skill '{name}' lost its regular-file type between stat and open"
        ));
    }
    // `Read::read` is permitted to return fewer bytes than the buffer
    // even for local regular files. A short read followed by `scan_skill`
    // on the truncated slice would let an attacker promote unscanned
    // tail content. `take(N).read_to_end` loops until EOF or the cap,
    // guaranteeing the scan covers everything that will be promoted.
    let mut buf: Vec<u8> = Vec::with_capacity(MAX_STAGED_READ_BYTES + 1);
    let bytes_read = file
        .by_ref()
        .take((MAX_STAGED_READ_BYTES as u64) + 1)
        .read_to_end(&mut buf)
        .with_context(|| format!("reading {}", staged_path.display()))?;
    if bytes_read > MAX_STAGED_READ_BYTES {
        return Err(anyhow!(
            "staged skill '{name}' grew past {MAX_STAGED_READ_BYTES} bytes during read"
        ));
    }
    let content = std::str::from_utf8(&buf)
        .map_err(|e| anyhow!("staged skill '{name}' is not valid UTF-8: {e}"))?
        .to_string();

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
    match std::fs::read_dir(&staged_dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                if let Some(file_name) = entry.file_name().to_str()
                    && let Some(name) = file_name.strip_suffix(".md")
                {
                    out.push(name.to_string());
                }
            }
        }
        Err(e) => {
            // Don't bubble up — listing is best-effort — but make the
            // failure visible so a permission/storage problem on the
            // staged dir doesn't masquerade as "no proposals".
            warn!(
                "skill_propose: read_dir({}) failed: {e}",
                staged_dir.display()
            );
        }
    }
    out.sort();
    out
}

/// One staged proposal with summary metadata for the operator UI.
#[derive(Debug, Clone)]
pub struct StagedSkill {
    pub name: String,
    pub bytes: u64,
    pub created_at_ms: i64,
    /// First non-blank line under `description:` in the frontmatter,
    /// or the first body line, or empty.
    pub description: String,
}

/// Return staged proposals with size + description so a tool wrapper
/// can build human-friendly listings without re-reading every file.
pub fn list_staged_with_metadata(workspace_skills: &Path) -> Vec<StagedSkill> {
    let staged_dir = workspace_skills.join("staged");
    if !staged_dir.exists() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&staged_dir) {
        Ok(e) => e,
        Err(e) => {
            warn!(
                "skill_propose: read_dir({}) failed: {e}",
                staged_dir.display()
            );
            return out;
        }
    };
    for entry in entries.flatten() {
        let Some(file_name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Some(name) = file_name.strip_suffix(".md") else {
            continue;
        };
        let path = entry.path();
        // Use symlink_metadata so a symlink in the staged dir reports
        // as `is_file() == false` and gets filtered out — consistent
        // with promote/reject which refuse symlinks for TOCTOU
        // reasons. `entry.metadata()` would follow the link.
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let created_at_ms = meta
            .created()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0_i64, |d| d.as_millis() as i64);
        let description = std::fs::read_to_string(&path)
            .ok()
            .and_then(|c| extract_description(&c))
            .unwrap_or_default();
        out.push(StagedSkill {
            name: name.to_string(),
            bytes: meta.len(),
            created_at_ms,
            description,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Discard a staged proposal without promoting. Used by the operator
/// "reject" path (e.g. a button click). Symmetric counterpart to
/// `promote_staged_skill`. Refuses symlinks for the same TOCTOU
/// reasons promote does.
pub fn reject_staged_skill(workspace_skills: &Path, name: &str) -> Result<()> {
    validate_skill_name(name)?;
    let staged_path = workspace_skills.join("staged").join(format!("{name}.md"));
    let meta = std::fs::symlink_metadata(&staged_path).with_context(|| {
        format!(
            "no staged skill named '{name}' at {}",
            staged_path.display()
        )
    })?;
    if meta.file_type().is_symlink() {
        // Don't follow a symlink target on reject either — let the
        // operator manually clean it up after investigating.
        return Err(anyhow!(
            "staged skill '{name}' is a symlink — refusing to delete (manual cleanup required)"
        ));
    }
    std::fs::remove_file(&staged_path)
        .with_context(|| format!("removing {}", staged_path.display()))?;
    info!("skill_propose: rejected staged '{}'", name);
    Ok(())
}

/// Re-extract the description from a body string. Mirrors the logic
/// in `SkillIndex` so both produce the same value for the same file.
fn extract_description(content: &str) -> Option<String> {
    if let Some(rest) = content.strip_prefix("---")
        && let Some(end_idx) = rest.find("\n---\n")
    {
        let frontmatter = &rest[..end_idx];
        for line in frontmatter.lines() {
            if let Some(value) = line.trim().strip_prefix("description:") {
                let v = value.trim().trim_matches('"').trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with("---") {
            return Some(trimmed.to_string());
        }
    }
    None
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

    #[cfg(unix)]
    #[test]
    fn open_no_follow_rejects_symlinks() {
        // Defense for the residual TOCTOU race: even if symlink_metadata
        // saw a regular file and an attacker swapped it for a symlink
        // before File::open ran, O_NOFOLLOW makes the open syscall fail
        // with ELOOP. Verify directly against a known symlink path.
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.md");
        std::fs::write(&target, "real content").unwrap();
        let link = dir.path().join("link.md");
        symlink(&target, &link).unwrap();

        let err = open_no_follow(&link).expect_err("open should fail on symlink");
        // O_NOFOLLOW returns ELOOP on both Linux and macOS. Check the
        // raw errno rather than ErrorKind::FilesystemLoop because the
        // latter is unstable on the current MSRV.
        assert_eq!(
            err.raw_os_error(),
            Some(libc::ELOOP),
            "expected ELOOP, got: {err:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn list_staged_with_metadata_skips_symlinks() {
        // Listing must hide symlinked files for the same TOCTOU reason
        // promote/reject refuse them. Using the entry's metadata()
        // would follow the link and report the target as a regular
        // file — verify symlink_metadata is in use.
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside.md");
        std::fs::write(&outside, "real content").unwrap();
        let staged_dir = dir.path().join("staged");
        std::fs::create_dir_all(&staged_dir).unwrap();
        // One real file, one symlink.
        std::fs::write(staged_dir.join("real.md"), "body").unwrap();
        symlink(&outside, staged_dir.join("link.md")).unwrap();

        let entries = list_staged_with_metadata(dir.path());
        let names: Vec<_> = entries.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"real"), "real file should be listed");
        assert!(
            !names.contains(&"link"),
            "symlink should be excluded, got: {names:?}"
        );
    }
}
