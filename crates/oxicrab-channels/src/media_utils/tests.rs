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
