use super::*;

use std::ffi::OsStr;

#[test]
fn test_scrubbed_command_clears_env() {
    // Set a dangerous env var
    unsafe { std::env::set_var("SUPER_SECRET_KEY", "should-not-leak") };
    let cmd = scrubbed_command("echo");
    let envs: Vec<_> = cmd.as_std().get_envs().collect();
    // Should not contain our secret
    assert!(
        !envs
            .iter()
            .any(|(k, _)| *k == OsStr::new("SUPER_SECRET_KEY")),
        "secret env var should not be passed through"
    );
}

#[test]
fn test_scrubbed_command_passes_path() {
    if std::env::var("PATH").is_ok() {
        let cmd = scrubbed_command("echo");
        let envs: Vec<_> = cmd.as_std().get_envs().collect();
        assert!(
            envs.iter()
                .any(|(k, v)| *k == OsStr::new("PATH") && v.is_some()),
            "PATH should be passed through"
        );
    }
}

#[test]
fn test_scrubbed_command_passes_home() {
    if std::env::var("HOME").is_ok() {
        let cmd = scrubbed_command("echo");
        let envs: Vec<_> = cmd.as_std().get_envs().collect();
        assert!(
            envs.iter()
                .any(|(k, v)| *k == OsStr::new("HOME") && v.is_some()),
            "HOME should be passed through"
        );
    }
}
