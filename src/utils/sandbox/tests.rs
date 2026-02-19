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
    let _ = is_available();
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
