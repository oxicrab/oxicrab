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
    assert!(
        std::path::Path::new(&path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
    );
    // Cleanup
    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_save_binary_no_magic_check() {
    let result = save_media_file(b"arbitrary binary data", "test", "bin");
    assert!(result.is_ok());
    let path = result.unwrap();
    assert!(
        std::path::Path::new(&path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("bin"))
    );
    let _ = std::fs::remove_file(&path);
}
