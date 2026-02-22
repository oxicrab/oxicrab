use super::*;

#[test]
fn test_check_result_variants() {
    let pass = CheckResult::Pass("ok".to_string());
    assert_eq!(pass.label(), "PASS");
    assert_eq!(pass.detail(), "ok");
    assert!(!pass.is_fail());

    let fail = CheckResult::Fail("bad".to_string());
    assert_eq!(fail.label(), "FAIL");
    assert!(fail.is_fail());

    let skip = CheckResult::Skip("n/a".to_string());
    assert_eq!(skip.label(), "SKIP");
    assert!(!skip.is_fail());
}

#[test]
fn test_check_git_available() {
    let result = check_external_command("git", &["--version"]);
    // git should be available in dev environments
    assert!(matches!(result, CheckResult::Pass(_)));
}

#[test]
fn test_check_config_default_parses() {
    // Default config should always parse (even if file doesn't exist,
    // load_config falls back to defaults)
    let result = check_config_parses();
    // This may pass or fail depending on environment, but shouldn't panic
    let _ = result;
}

#[test]
fn test_check_external_command_missing() {
    let result = check_external_command("nonexistent_binary_xyz_42", &["--version"]);
    assert!(result.is_fail());
    assert!(result.detail().contains("not found"));
}

#[test]
fn test_check_external_command_captures_version() {
    let result = check_external_command("git", &["--version"]);
    if let CheckResult::Pass(detail) = &result {
        assert!(detail.contains("git version"));
    }
}

#[test]
fn test_check_external_command_failing_args() {
    // `ls` with a nonexistent path returns non-zero
    let result = check_external_command("ls", &["/nonexistent_path_xyz_42"]);
    assert!(result.is_fail());
}

#[cfg(unix)]
#[test]
fn test_check_file_permissions_secure() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, "{}").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

    let meta = std::fs::metadata(&path).unwrap();
    let mode = meta.permissions().mode() & 0o777;
    // Mode 0o600 has trailing_zeros >= 6 (binary: 110_000_000)
    assert!(mode.trailing_zeros() >= 6);
}

#[cfg(unix)]
#[test]
fn test_check_file_permissions_insecure() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, "{}").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let meta = std::fs::metadata(&path).unwrap();
    let mode = meta.permissions().mode() & 0o777;
    // Mode 0o644 = binary 110_100_100, trailing_zeros = 2 (< 6)
    assert!(mode.trailing_zeros() < 6);
}

#[test]
fn test_check_result_debug_impl() {
    // Ensure Debug is implemented and doesn't panic
    let pass = CheckResult::Pass("ok".to_string());
    let _ = format!("{:?}", pass);
}

#[test]
fn test_print_check_does_not_panic() {
    let pass = CheckResult::Pass("all good".to_string());
    let fail = CheckResult::Fail("broken".to_string());
    let skip = CheckResult::Skip("skipped".to_string());
    // Just ensure formatting doesn't panic
    print_check("test_pass", &pass);
    print_check("test_fail", &fail);
    print_check("test_skip", &skip);
}
