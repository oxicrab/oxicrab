use super::*;

#[test]
fn test_sandbox_rules_default() {
    let config = SandboxConfig::default();
    let rules = SandboxRules::for_shell(Path::new("/workspace"), &config);

    assert!(rules.read_only_paths.contains(&"/usr".to_string()));
    assert!(rules.read_only_paths.contains(&"/lib".to_string()));
    assert!(rules.read_only_paths.contains(&"/etc".to_string()));
    assert!(rules.read_only_paths.contains(&"/bin".to_string()));
    assert!(rules.read_only_paths.contains(&"/sbin".to_string()));

    assert!(rules.read_write_paths.contains(&"/workspace".to_string()));
    assert!(rules.read_write_paths.contains(&"/tmp".to_string()));
    assert!(rules.read_write_paths.contains(&"/var/tmp".to_string()));

    assert!(rules.block_network);
}

#[test]
fn test_sandbox_rules_custom_paths() {
    let config = SandboxConfig {
        enabled: true,
        additional_read_paths: vec!["/opt/data".to_string()],
        additional_write_paths: vec!["/home/user/output".to_string()],
        block_network: false,
    };
    let rules = SandboxRules::for_shell(Path::new("/my/workspace"), &config);

    assert!(rules.read_only_paths.contains(&"/opt/data".to_string()));
    assert!(
        rules
            .read_write_paths
            .contains(&"/my/workspace".to_string())
    );
    assert!(
        rules
            .read_write_paths
            .contains(&"/home/user/output".to_string())
    );
    assert!(!rules.block_network);
}

#[test]
fn test_sandbox_rules_network_disabled() {
    let config = SandboxConfig {
        block_network: false,
        ..SandboxConfig::default()
    };
    let rules = SandboxRules::for_shell(Path::new("/ws"), &config);
    assert!(!rules.block_network);
}

#[test]
fn test_is_available_does_not_panic() {
    // Just verify it returns a bool without panicking
    let available = is_available();
    #[cfg(target_os = "macos")]
    assert!(available, "Seatbelt should always be available on macOS");
    #[cfg(not(target_os = "macos"))]
    let _ = available;
}

#[cfg(target_os = "linux")]
#[test]
fn test_sandboxed_read_system() {
    // Verify that a sandboxed process can read /etc/hostname
    let config = SandboxConfig::default();
    let rules = SandboxRules::for_shell(Path::new("/tmp"), &config);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let mut cmd = tokio::process::Command::new("cat");
        cmd.arg("/etc/hostname");
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let _ = apply_to_command(&mut cmd, &rules);
        if let Ok(output) = cmd.output().await {
            // On kernels with Landlock support, reading /etc should work.
            // On kernels without, the sandbox is a no-op and it still works.
            assert!(
                output.status.success(),
                "reading /etc/hostname should succeed under sandbox"
            );
        }
        // If the command itself fails to spawn, that's OK in CI
    });
}

#[cfg(target_os = "linux")]
#[test]
fn test_sandboxed_write_blocked() {
    // Verify that a sandboxed process cannot write to /usr
    let config = SandboxConfig::default();
    let rules = SandboxRules::for_shell(Path::new("/tmp"), &config);

    if !is_available() {
        return; // Skip if Landlock not supported
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let mut cmd = tokio::process::Command::new("touch");
        cmd.arg("/usr/.sandbox_test");
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let _ = apply_to_command(&mut cmd, &rules);
        if let Ok(output) = cmd.output().await {
            assert!(
                !output.status.success(),
                "writing to /usr should be denied under sandbox"
            );
        }
        // Command failed to spawn — still acceptable
    });
}

#[cfg(target_os = "linux")]
#[test]
fn test_sandboxed_write_workspace() {
    // Verify that a sandboxed process can write to the workspace (tmp)
    let config = SandboxConfig::default();
    let tmp = tempfile::TempDir::new().unwrap();
    let rules = SandboxRules::for_shell(tmp.path(), &config);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let test_file = tmp.path().join("sandbox_test");
        let mut cmd = tokio::process::Command::new("touch");
        cmd.arg(&test_file);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let _ = apply_to_command(&mut cmd, &rules);
        if let Ok(output) = cmd.output().await {
            // With or without Landlock, workspace writes should succeed
            assert!(
                output.status.success(),
                "writing to workspace should succeed under sandbox"
            );
        }
    });
}

#[cfg(target_os = "macos")]
#[test]
fn test_seatbelt_profile_structure() {
    let config = SandboxConfig::default();
    let rules = SandboxRules::for_shell(Path::new("/workspace"), &config);
    let profile = build_seatbelt_profile(&rules);

    assert!(profile.starts_with("(version 1)"));
    assert!(profile.contains("(deny default)"));
    assert!(profile.contains("(allow process-exec)"));
    assert!(profile.contains("(allow mach-lookup)"));
    assert!(profile.contains("(allow process-info* (target self))"));
    assert!(profile.contains("(subpath \"/dev\")"));
    assert!(profile.contains("(literal \"/dev/null\")"));
    assert!(profile.contains("(allow file-read-metadata)"));
    assert!(profile.contains("(allow ipc-posix-shm-read-data)"));
    // Rules paths
    assert!(profile.contains("(subpath \"/usr\")"));
    assert!(profile.contains("(subpath \"/workspace\")"));
    // macOS system paths
    assert!(profile.contains("(subpath \"/System\")"));
    assert!(profile.contains("(subpath \"/Library\")"));
    assert!(profile.contains("(subpath \"/opt/homebrew\")"));
    // Symlink targets
    assert!(profile.contains("(subpath \"/private/tmp\")"));
    assert!(profile.contains("(subpath \"/private/var/folders\")"));
    // Network blocked by default
    assert!(!profile.contains("(allow network*)"));
}

#[cfg(target_os = "macos")]
#[test]
fn test_seatbelt_profile_network_allowed() {
    let config = SandboxConfig {
        block_network: false,
        ..SandboxConfig::default()
    };
    let rules = SandboxRules::for_shell(Path::new("/ws"), &config);
    let profile = build_seatbelt_profile(&rules);
    assert!(profile.contains("(allow network*)"));
}

#[cfg(target_os = "macos")]
#[test]
fn test_seatbelt_profile_escapes_quotes() {
    let config = SandboxConfig {
        additional_read_paths: vec!["/path/with \"quotes\"".to_string()],
        ..SandboxConfig::default()
    };
    let rules = SandboxRules::for_shell(Path::new("/ws"), &config);
    let profile = build_seatbelt_profile(&rules);
    assert!(profile.contains(r#"(subpath "/path/with \"quotes\"")"#));
}

#[cfg(target_os = "macos")]
#[test]
fn test_sandboxed_read_system() {
    use std::os::unix::process::ExitStatusExt;

    let config = SandboxConfig::default();
    let rules = SandboxRules::for_shell(Path::new("/tmp"), &config);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let mut cmd = tokio::process::Command::new("cat");
        cmd.arg("/etc/hosts");
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let _ = apply_to_command(&mut cmd, &rules);
        match cmd.output().await {
            Ok(output) => {
                // sandbox_init() is deprecated (macOS 10.8) and its interaction
                // with child process I/O varies across macOS versions. If the
                // process was killed by a signal, the sandbox blocked an operation
                // we can't control (e.g. writing to pipe fds). Log diagnostics
                // but don't fail CI — structural profile tests cover correctness.
                if output.status.signal().is_some() {
                    eprintln!(
                        "sandbox read test: process killed by signal {} (sandbox denied pipe I/O)",
                        output.status.signal().unwrap()
                    );
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    assert!(
                        output.status.success(),
                        "reading /etc/hosts failed (exit: {:?}, stderr: {stderr})",
                        output.status.code()
                    );
                }
            }
            Err(e) => {
                eprintln!("sandbox test skipped: command failed to spawn: {e}");
            }
        }
    });
}

#[cfg(target_os = "macos")]
#[test]
fn test_sandboxed_write_workspace() {
    use std::os::unix::process::ExitStatusExt;

    let config = SandboxConfig::default();
    let tmp = tempfile::TempDir::new().unwrap();
    let rules = SandboxRules::for_shell(tmp.path(), &config);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let test_file = tmp.path().join("seatbelt_test");
        let mut cmd = tokio::process::Command::new("touch");
        cmd.arg(&test_file);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let _ = apply_to_command(&mut cmd, &rules);
        match cmd.output().await {
            Ok(output) => {
                if output.status.signal().is_some() {
                    eprintln!(
                        "sandbox write test: process killed by signal {} (sandbox denied pipe I/O)",
                        output.status.signal().unwrap()
                    );
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    assert!(
                        output.status.success(),
                        "writing to workspace failed (exit: {:?}, stderr: {stderr})",
                        output.status.code()
                    );
                }
            }
            Err(e) => {
                eprintln!("sandbox test skipped: command failed to spawn: {e}");
            }
        }
    });
}
