use super::*;
use std::path::PathBuf;

#[test]
fn test_sanitize_path_workspace_relative() {
    let home = dirs::home_dir().unwrap();
    let workspace = home.join("projects/myapp");
    let path = home.join("projects/myapp/src/main.rs");
    let result = sanitize_path(&path, Some(&workspace));
    assert_eq!(result, "~/projects/myapp/src/main.rs");
}

#[test]
fn test_sanitize_path_oxicrab_dir() {
    let home = dirs::home_dir().unwrap();
    let path = home.join(".oxicrab/config.json");
    let result = sanitize_path(&path, None);
    assert_eq!(result, "~/.oxicrab/config.json");
}

#[test]
fn test_sanitize_path_outside_workspace_under_home() {
    let home = dirs::home_dir().unwrap();
    let workspace = home.join("projects/myapp");
    let path = home.join("secrets/key.pem");
    let result = sanitize_path(&path, Some(&workspace));
    assert_eq!(result, "<redacted>/key.pem");
}

#[test]
fn test_sanitize_path_system_path_unchanged() {
    let path = PathBuf::from("/etc/passwd");
    let result = sanitize_path(&path, None);
    assert_eq!(result, "/etc/passwd");
}

#[test]
fn test_sanitize_path_system_usr_unchanged() {
    let path = PathBuf::from("/usr/lib/libfoo.so");
    let result = sanitize_path(&path, None);
    assert_eq!(result, "/usr/lib/libfoo.so");
}

#[test]
fn test_sanitize_error_message_with_embedded_paths() {
    let home = dirs::home_dir().unwrap();
    let workspace = home.join("projects/myapp");
    let home_str = home.to_string_lossy();
    let msg = format!(
        "Error: file not found: {}/secrets/key.pem and /etc/hosts",
        home_str
    );
    let result = sanitize_error_message(&msg, Some(&workspace));
    assert!(result.contains("<redacted>/key.pem"));
    assert!(result.contains("/etc/hosts"));
    assert!(!result.contains(&*home_str));
}

#[test]
fn test_sanitize_path_no_home_dir_unchanged() {
    // When path is not under home at all
    let workspace = PathBuf::from("/opt/app");
    let path = PathBuf::from("/opt/app/data/file.txt");
    let result = sanitize_path(&path, Some(&workspace));
    // Not under home → unchanged
    assert_eq!(result, "/opt/app/data/file.txt");
}

#[test]
fn test_sanitize_error_message_no_paths() {
    let result = sanitize_error_message("simple error with no paths", None);
    assert_eq!(result, "simple error with no paths");
}
