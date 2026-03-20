//! Sandbox utilities for shell command execution.

use oxicrab_core::config::schema::SandboxConfig;
use std::path::Path;
use tracing::warn;

/// Resolved sandbox rules for a single shell command execution.
pub struct SandboxRules {
    pub read_only_paths: Vec<String>,
    pub read_write_paths: Vec<String>,
    pub block_network: bool,
}

impl SandboxRules {
    pub fn for_shell(workspace: &Path, config: &SandboxConfig) -> Self {
        let mut read_only = vec![
            "/usr".to_string(),
            "/lib".to_string(),
            "/lib64".to_string(),
            "/bin".to_string(),
            "/sbin".to_string(),
            "/etc".to_string(),
        ];
        read_only.extend(config.additional_read_paths.clone());

        let workspace_resolved = workspace.canonicalize().unwrap_or_else(|e| {
            warn!(
                "failed to canonicalize workspace path {:?}: {} — using original path",
                workspace, e
            );
            workspace.to_path_buf()
        });
        let mut read_write = vec![
            workspace_resolved.to_string_lossy().to_string(),
            "/tmp".to_string(),
            "/var/tmp".to_string(),
        ];
        read_write.extend(config.additional_write_paths.clone());

        Self {
            read_only_paths: read_only,
            read_write_paths: read_write,
            block_network: config.block_network,
        }
    }
}

#[cfg(target_os = "linux")]
#[allow(clippy::unnecessary_wraps)]
pub fn apply_to_command(
    cmd: &mut tokio::process::Command,
    rules: &SandboxRules,
) -> anyhow::Result<()> {
    use landlock::{
        ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, PathBeneath, PathFd, Ruleset,
        RulesetAttr, RulesetCreatedAttr,
    };

    let abi = ABI::V5;
    let read_only = rules.read_only_paths.clone();
    let read_write = rules.read_write_paths.clone();
    let block_network = rules.block_network;

    // SAFETY: pre_exec runs between fork() and exec() in the child process.
    // We only call Landlock syscalls here.
    unsafe {
        cmd.pre_exec(move || {
            let read_access = AccessFs::from_read(abi);
            let full_access = AccessFs::from_all(abi);

            let mut ruleset = Ruleset::default()
                .set_compatibility(CompatLevel::BestEffort)
                .handle_access(full_access)
                .map_err(|e| std::io::Error::other(e.to_string()))?;

            if block_network {
                ruleset = ruleset
                    .handle_access(AccessNet::from_all(abi))
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
            }

            let mut created = ruleset
                .create()
                .map_err(|e| std::io::Error::other(e.to_string()))?;

            for path_str in &read_only {
                let path = std::path::Path::new(path_str);
                if path.exists()
                    && let Ok(fd) = PathFd::new(path)
                {
                    let rule = PathBeneath::new(fd, read_access);
                    created = created
                        .add_rule(rule)
                        .map_err(|e| std::io::Error::other(e.to_string()))?;
                }
            }

            for path_str in &read_write {
                let path = std::path::Path::new(path_str);
                if path.exists()
                    && let Ok(fd) = PathFd::new(path)
                {
                    let rule = PathBeneath::new(fd, full_access);
                    created = created
                        .add_rule(rule)
                        .map_err(|e| std::io::Error::other(e.to_string()))?;
                }
            }

            created
                .restrict_self()
                .map_err(|e| std::io::Error::other(e.to_string()))?;

            Ok(())
        });
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn escape_sbpl_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '(' | ')' | ';' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn build_seatbelt_profile(rules: &SandboxRules) -> String {
    use std::fmt::Write;

    let mut p = String::with_capacity(1024);
    p.push_str("(version 1)\n(deny default)\n");
    p.push_str("(allow process-exec)\n");
    p.push_str("(allow process-fork)\n");
    p.push_str("(allow signal)\n");
    p.push_str("(allow sysctl-read)\n");
    p.push_str("(allow mach-lookup)\n");
    p.push_str("(allow process-info* (target self))\n");
    p.push_str("(allow file-read* (subpath \"/dev\"))\n");
    p.push_str("(allow file-write-data (literal \"/dev/null\"))\n");
    p.push_str("(allow file-ioctl (literal \"/dev/null\"))\n");
    p.push_str("(allow file-read-metadata)\n");
    p.push_str("(allow ipc-posix-shm-read-data)\n");
    p.push_str("(allow ipc-posix-shm-read-metadata)\n");

    for path in &rules.read_only_paths {
        let escaped = escape_sbpl_string(path);
        let _ = writeln!(p, "(allow file-read* (subpath \"{escaped}\"))");
    }

    for sys_path in [
        "/System",
        "/Library",
        "/private/etc",
        "/private/var/db",
        "/opt/homebrew",
        "/usr/local",
    ] {
        let _ = writeln!(p, "(allow file-read* (subpath \"{sys_path}\"))");
    }

    for path in &rules.read_write_paths {
        let escaped = escape_sbpl_string(path);
        let _ = writeln!(p, "(allow file-read* file-write* (subpath \"{escaped}\"))");
    }

    for rw_path in ["/private/tmp", "/private/var/tmp", "/private/var/folders"] {
        let _ = writeln!(p, "(allow file-read* file-write* (subpath \"{rw_path}\"))");
    }

    if !rules.block_network {
        p.push_str("(allow network*)\n");
    }

    p
}

#[cfg(target_os = "macos")]
#[allow(clippy::unnecessary_wraps)]
pub fn apply_to_command(
    cmd: &mut tokio::process::Command,
    rules: &SandboxRules,
) -> anyhow::Result<()> {
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int};

    unsafe extern "C" {
        fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;
        fn sandbox_free_error(errorbuf: *mut c_char);
    }

    let profile = build_seatbelt_profile(rules);
    let profile_cstr =
        CString::new(profile).map_err(|e| anyhow::anyhow!("invalid seatbelt profile: {e}"))?;

    // SAFETY: pre_exec runs between fork() and exec() in the child process.
    // sandbox_init() applies Seatbelt restrictions to the calling (child) process.
    unsafe {
        cmd.pre_exec(move || {
            let mut err: *mut c_char = std::ptr::null_mut();
            let result = sandbox_init(profile_cstr.as_ptr(), 0, &raw mut err);
            if result != 0 {
                let msg = if err.is_null() {
                    "unknown error".to_string()
                } else {
                    let s = CStr::from_ptr(err).to_string_lossy().into_owned();
                    sandbox_free_error(err);
                    s
                };
                return Err(std::io::Error::other(format!("sandbox_init failed: {msg}")));
            }
            Ok(())
        });
    }

    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn apply_to_command(
    _cmd: &mut tokio::process::Command,
    _rules: &SandboxRules,
) -> anyhow::Result<()> {
    anyhow::bail!("sandbox not available on this platform (only Linux and macOS are supported)")
}
