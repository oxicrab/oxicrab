use super::*;
use proptest::prelude::*;

proptest! {
    #[test]
    fn safe_filename_never_panics(s in "\\PC*") {
        let _ = safe_filename(&s);
    }

    #[test]
    fn safe_filename_no_path_separators(s in "\\PC{0,200}") {
        let result = safe_filename(&s);
        assert!(!result.contains('/'), "output should not contain /");
        assert!(!result.contains('\\'), "output should not contain \\");
    }

    #[test]
    fn safe_filename_no_nul_bytes(s in "\\PC{0,200}") {
        let result = safe_filename(&s);
        assert!(!result.contains('\0'), "output should not contain NUL");
    }
}

#[test]
fn safe_filename_replaces_dangerous_chars() {
    assert_eq!(safe_filename("a/b\\c:d*e"), "a_b_c_d_e");
    assert_eq!(safe_filename("file<>|name"), "file___name");
}

#[test]
fn workspace_path_tilde_slash() {
    let result = get_workspace_path("~/foo/bar");
    let home = dirs::home_dir().unwrap();
    assert_eq!(result, home.join("foo/bar"));
}

#[test]
fn workspace_path_tilde_only() {
    let result = get_workspace_path("~");
    let home = dirs::home_dir().unwrap();
    assert_eq!(result, home);
}

#[test]
fn workspace_path_relative() {
    let result = get_workspace_path("relative/path");
    assert_eq!(result, PathBuf::from("relative/path"));
}

#[test]
fn ensure_dir_creates_and_returns() {
    let tmp = tempfile::tempdir().unwrap();
    let new_dir = tmp.path().join("subdir");
    let result = ensure_dir(&new_dir).unwrap();
    assert_eq!(result, new_dir);
    assert!(new_dir.exists());
}

#[test]
fn atomic_write_creates_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test.txt");
    atomic_write(&path, "hello").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
}

#[test]
fn atomic_write_overwrites() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test.txt");
    atomic_write(&path, "first").unwrap();
    atomic_write(&path, "second").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
}
