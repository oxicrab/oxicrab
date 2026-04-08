use anyhow::{Context, Result};
use std::path::PathBuf;

/// Return the `~/.oxicrab/media/` directory, creating it if needed.
pub fn media_dir() -> Result<PathBuf> {
    let dir = get_oxicrab_home()
        .context("failed to determine oxicrab home")?
        .join("media");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create media directory: {}", dir.display()))?;
    Ok(dir)
}

pub fn get_oxicrab_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("OXICRAB_HOME") {
        return Ok(PathBuf::from(home));
    }
    Ok(dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".oxicrab"))
}

/// Sanitize a string for use as a filename.
pub fn safe_filename(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '\0')
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

/// Map a MIME type to a file extension.
///
/// Covers common image, audio, video, and document types used across channels.
pub fn mime_to_extension(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "audio/ogg" | "audio/ogg; codecs=opus" => "ogg",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/mp4" | "audio/m4a" => "m4a",
        "audio/webm" | "video/webm" => "webm",
        "audio/flac" => "flac",
        "video/mp4" => "mp4",
        "application/pdf" => "pdf",
        _ => "bin",
    }
}

/// Check if bytes start with known image magic bytes.
pub fn is_image_magic_bytes(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    // PNG: 89 50 4E 47
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return true;
    }
    // JPEG: FF D8 FF
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return true;
    }
    // GIF: GIF87a or GIF89a
    if data.starts_with(b"GIF8") {
        return true;
    }
    // WebP: RIFF....WEBP
    if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- mime_to_extension tests ---

    #[test]
    fn test_mime_to_extension_jpeg() {
        assert_eq!(mime_to_extension("image/jpeg"), "jpg");
        assert_eq!(mime_to_extension("image/jpg"), "jpg");
    }

    #[test]
    fn test_mime_to_extension_png() {
        assert_eq!(mime_to_extension("image/png"), "png");
    }

    #[test]
    fn test_mime_to_extension_ogg_with_codecs() {
        // The semicolon variant is important — Telegram sends this
        assert_eq!(mime_to_extension("audio/ogg; codecs=opus"), "ogg");
        assert_eq!(mime_to_extension("audio/ogg"), "ogg");
    }

    #[test]
    fn test_mime_to_extension_video_mp4() {
        assert_eq!(mime_to_extension("video/mp4"), "mp4");
    }

    #[test]
    fn test_mime_to_extension_pdf() {
        assert_eq!(mime_to_extension("application/pdf"), "pdf");
    }

    #[test]
    fn test_mime_to_extension_unknown_returns_bin() {
        assert_eq!(mime_to_extension("application/octet-stream"), "bin");
        assert_eq!(mime_to_extension("text/html"), "bin");
        assert_eq!(mime_to_extension(""), "bin");
    }

    #[test]
    fn test_mime_to_extension_audio_formats() {
        assert_eq!(mime_to_extension("audio/mpeg"), "mp3");
        assert_eq!(mime_to_extension("audio/mp3"), "mp3");
        assert_eq!(mime_to_extension("audio/wav"), "wav");
        assert_eq!(mime_to_extension("audio/x-wav"), "wav");
        assert_eq!(mime_to_extension("audio/mp4"), "m4a");
        assert_eq!(mime_to_extension("audio/m4a"), "m4a");
        assert_eq!(mime_to_extension("audio/flac"), "flac");
    }

    // --- safe_filename tests ---

    #[test]
    fn test_safe_filename_sanitizes_path_separators() {
        assert_eq!(safe_filename("path/to/file"), "path_to_file");
        assert_eq!(safe_filename("path\\to\\file"), "path_to_file");
    }

    #[test]
    fn test_safe_filename_removes_null_bytes() {
        assert_eq!(safe_filename("file\0name"), "filename");
    }

    #[test]
    fn test_safe_filename_sanitizes_special_chars() {
        assert_eq!(safe_filename("file:name"), "file_name");
        assert_eq!(safe_filename("file*name"), "file_name");
        assert_eq!(safe_filename("file?name"), "file_name");
        assert_eq!(safe_filename("file\"name"), "file_name");
        assert_eq!(safe_filename("file<name>"), "file_name_");
        assert_eq!(safe_filename("file|name"), "file_name");
    }

    #[test]
    fn test_safe_filename_preserves_normal_chars() {
        assert_eq!(safe_filename("normal-file.txt"), "normal-file.txt");
        assert_eq!(safe_filename("photo_2026.jpg"), "photo_2026.jpg");
    }

    #[test]
    fn test_safe_filename_combined_attack() {
        // Path traversal + null byte + special chars
        assert_eq!(safe_filename("../\0etc/passwd"), ".._etc_passwd");
    }
}
