use anyhow::{Context, Result, bail};
use std::path::PathBuf;

const MAX_MEDIA_SIZE: usize = 20 * 1024 * 1024; // 20MB

/// Return the `~/.oxicrab/media/` directory, creating it if needed.
pub fn media_dir() -> Result<PathBuf> {
    let dir = super::get_oxicrab_home()
        .context("failed to determine oxicrab home")?
        .join("media");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create media directory: {}", dir.display()))?;
    Ok(dir)
}

/// Save binary data to a file in `~/.oxicrab/media/`.
///
/// Validates size (20MB max) and image magic bytes for image extensions.
/// Returns the absolute path to the saved file.
pub fn save_media_file(bytes: &[u8], prefix: &str, extension: &str) -> Result<String> {
    if bytes.is_empty() {
        bail!("empty media data");
    }
    if bytes.len() > MAX_MEDIA_SIZE {
        bail!(
            "media too large: {} bytes (max {})",
            bytes.len(),
            MAX_MEDIA_SIZE
        );
    }

    // Validate image magic bytes for known image extensions
    let image_exts = ["png", "jpg", "jpeg", "gif", "webp"];
    if image_exts.contains(&extension) && !is_image_magic_bytes(bytes) {
        bail!("data does not match expected image format for .{extension}");
    }

    let media_dir = media_dir()?;

    let safe_prefix = super::safe_filename(prefix);
    let safe_ext = super::safe_filename(extension);

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let random = fastrand::u32(..);
    let filename = format!("{safe_prefix}_{timestamp}_{random:08x}.{safe_ext}");
    let path = media_dir.join(&filename);

    std::fs::write(&path, bytes)
        .with_context(|| format!("failed to write media file: {}", path.display()))?;

    Ok(path.to_string_lossy().to_string())
}

/// Check if bytes start with known image magic bytes.
pub fn is_image_magic_bytes(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return true;
    }
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return true;
    }
    if data.starts_with(b"GIF8") {
        return true;
    }
    if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        return true;
    }
    false
}
