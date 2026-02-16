use anyhow::{bail, Context, Result};

const MAX_MEDIA_SIZE: usize = 20 * 1024 * 1024; // 20MB

/// Save binary data to a file in `~/.oxicrab/media/`.
///
/// Validates size (10MB max) and image magic bytes for image extensions.
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
        bail!(
            "data does not match expected image format for .{}",
            extension
        );
    }

    let media_dir = super::get_oxicrab_home()
        .context("failed to determine oxicrab home")?
        .join("media");
    std::fs::create_dir_all(&media_dir)
        .with_context(|| format!("failed to create media directory: {}", media_dir.display()))?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let random = fastrand::u32(..);
    let filename = format!("{prefix}_{timestamp}_{random:08x}.{extension}");
    let path = media_dir.join(&filename);

    std::fs::write(&path, bytes)
        .with_context(|| format!("failed to write media file: {}", path.display()))?;

    Ok(path.to_string_lossy().to_string())
}

/// Map a Content-Type header to a file extension.
///
/// Returns `None` for text/* and application/json (callers should fall through
/// to text handling). Returns `Some("bin")` for unknown binary types.
pub fn extension_from_content_type(ct: &str) -> Option<&'static str> {
    let ct_lower = ct.to_lowercase();

    // Text types — fall through to text handling
    if ct_lower.starts_with("text/") || ct_lower.contains("application/json") {
        return None;
    }

    // Known image types
    if ct_lower.contains("image/png") {
        return Some("png");
    }
    if ct_lower.contains("image/jpeg") {
        return Some("jpg");
    }
    if ct_lower.contains("image/gif") {
        return Some("gif");
    }
    if ct_lower.contains("image/webp") {
        return Some("webp");
    }
    if ct_lower.contains("image/svg") {
        return Some("svg");
    }

    // Known audio types
    if ct_lower.contains("audio/mpeg") {
        return Some("mp3");
    }
    if ct_lower.contains("audio/wav") {
        return Some("wav");
    }
    if ct_lower.contains("audio/ogg") {
        return Some("ogg");
    }

    // Known video types
    if ct_lower.contains("video/mp4") {
        return Some("mp4");
    }
    if ct_lower.contains("video/webm") {
        return Some("webm");
    }

    // PDF
    if ct_lower.contains("application/pdf") {
        return Some("pdf");
    }

    // application/octet-stream or other binary
    if ct_lower.contains("application/octet-stream")
        || ct_lower.starts_with("image/")
        || ct_lower.starts_with("audio/")
        || ct_lower.starts_with("video/")
    {
        return Some("bin");
    }

    // Unknown — fall through to text handling
    None
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

    // --- is_image_magic_bytes ---

    #[test]
    fn test_magic_png() {
        assert!(is_image_magic_bytes(&[0x89, 0x50, 0x4E, 0x47, 0x0D]));
    }

    #[test]
    fn test_magic_jpeg() {
        assert!(is_image_magic_bytes(&[0xFF, 0xD8, 0xFF, 0xE0]));
    }

    #[test]
    fn test_magic_gif() {
        assert!(is_image_magic_bytes(b"GIF89a"));
    }

    #[test]
    fn test_magic_webp() {
        let mut d = Vec::new();
        d.extend_from_slice(b"RIFF");
        d.extend_from_slice(&[0; 4]);
        d.extend_from_slice(b"WEBP");
        assert!(is_image_magic_bytes(&d));
    }

    #[test]
    fn test_magic_not_image() {
        assert!(!is_image_magic_bytes(b"hello world"));
        assert!(!is_image_magic_bytes(&[0x00, 0x01]));
    }

    // --- extension_from_content_type ---

    #[test]
    fn test_ext_png() {
        assert_eq!(extension_from_content_type("image/png"), Some("png"));
    }

    #[test]
    fn test_ext_jpeg() {
        assert_eq!(extension_from_content_type("image/jpeg"), Some("jpg"));
    }

    #[test]
    fn test_ext_text_html() {
        assert_eq!(
            extension_from_content_type("text/html; charset=utf-8"),
            None
        );
    }

    #[test]
    fn test_ext_json() {
        assert_eq!(extension_from_content_type("application/json"), None);
    }

    #[test]
    fn test_ext_octet_stream() {
        assert_eq!(
            extension_from_content_type("application/octet-stream"),
            Some("bin")
        );
    }

    #[test]
    fn test_ext_pdf() {
        assert_eq!(extension_from_content_type("application/pdf"), Some("pdf"));
    }

    #[test]
    fn test_ext_unknown_text() {
        assert_eq!(extension_from_content_type("text/csv"), None);
    }

    // --- save_media_file ---

    #[test]
    fn test_save_empty_data() {
        let result = save_media_file(&[], "test", "bin");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[test]
    fn test_save_too_large() {
        let big = vec![0u8; MAX_MEDIA_SIZE + 1];
        let result = save_media_file(&big, "test", "bin");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too large"));
    }

    #[test]
    fn test_save_image_magic_mismatch() {
        let result = save_media_file(b"not a png file", "test", "png");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not match"));
    }

    #[test]
    fn test_save_valid_png() {
        let png_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        let result = save_media_file(&png_bytes, "test", "png");
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.contains("test_"));
        assert!(std::path::Path::new(&path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("png")));
        // Cleanup
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_save_binary_no_magic_check() {
        let result = save_media_file(b"arbitrary binary data", "test", "bin");
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(std::path::Path::new(&path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bin")));
        let _ = std::fs::remove_file(&path);
    }
}
