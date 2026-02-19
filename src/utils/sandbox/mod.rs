use crate::config::SandboxConfig;
use std::path::Path;

/// Resolved sandbox rules for a single shell command execution.
pub struct SandboxRules {
    /// Paths to grant read-only access (system dirs).
    pub read_only_paths: Vec<String>,
    /// Paths to grant read-write access (workspace, /tmp).
    pub read_write_paths: Vec<String>,
    /// Block all outbound network connections.
    pub block_network: bool,
}

impl SandboxRules {
    /// Build sandbox rules for a shell command execution.
    ///
    /// Default read-only: `/usr`, `/lib`, `/lib64`, `/bin`, `/sbin`, `/etc`
    /// Default read-write: workspace dir, `/tmp`, `/var/tmp`
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

        // Canonicalize workspace to resolve symlinks (e.g. /var → /private/var on macOS)
        // so sandbox rules match the real paths the kernel uses.
        let workspace_resolved = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf());
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

/// Check whether Landlock LSM is available on this kernel.
#[cfg(target_os = "linux")]
pub fn is_available() -> bool {
    use landlock::{ABI, Access, AccessFs, Ruleset, RulesetAttr, RulesetStatus};

    // Probe by creating a minimal ruleset — if the kernel supports it,
    // this will succeed without actually restricting anything.
    let abi = ABI::V5;
    match Ruleset::default().handle_access(AccessFs::from_all(abi)) {
        Ok(ruleset) => match ruleset.create() {
            Ok(created) => match created.restrict_self() {
                Ok(status) => !matches!(status.ruleset, RulesetStatus::NotEnforced),
                Err(_) => false,
            },
            Err(_) => false,
        },
        Err(_) => false,
    }
}

/// Check whether macOS Seatbelt sandbox is available.
#[cfg(target_os = "macos")]
pub fn is_available() -> bool {
    // Seatbelt has been available since macOS 10.5 (Leopard, 2007).
    // All Rust-supported macOS versions include it.
    true
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn is_available() -> bool {
    false
}

/// Apply sandbox rules to a `tokio::process::Command` via `pre_exec`.
///
/// On Linux, uses Landlock LSM for filesystem/network restrictions.
/// On macOS, uses Seatbelt (`sandbox_init`) for filesystem/network restrictions.
/// On other platforms, this is a no-op.
#[cfg(target_os = "linux")]
#[allow(clippy::unnecessary_wraps)]
pub fn apply_to_command(
    cmd: &mut tokio::process::Command,
    rules: &SandboxRules,
) -> anyhow::Result<()> {
    use landlock::{
        ABI, Access, AccessFs, AccessNet, PathBeneath, PathFd, Ruleset, RulesetAttr,
        RulesetCreatedAttr,
    };

    let abi = ABI::V5;

    // Clone data for the pre_exec closure (which is FnMut + 'static)
    let read_only = rules.read_only_paths.clone();
    let read_write = rules.read_write_paths.clone();
    let block_network = rules.block_network;

    // SAFETY: pre_exec runs between fork() and exec() in the child process.
    // We only call Landlock syscalls here — no async, no allocations that could
    // deadlock. The landlock crate's restrict_self() is safe in this context.
    unsafe {
        cmd.pre_exec(move || {
            let read_access = AccessFs::from_read(abi);
            let full_access = AccessFs::from_all(abi);

            let mut ruleset = Ruleset::default()
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

            // Grant read-only access to system directories
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

            // Grant full access to workspace and temp directories
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

            // No network port rules = all TCP connections blocked
            // (only applies when block_network is true and AccessNet is handled)

            created
                .restrict_self()
                .map_err(|e| std::io::Error::other(e.to_string()))?;

            Ok(())
        });
    }

    Ok(())
}

/// Build a macOS Seatbelt (SBPL) profile string from sandbox rules.
#[cfg(target_os = "macos")]
fn build_seatbelt_profile(rules: &SandboxRules) -> String {
    use std::fmt::Write;

    let mut p = String::with_capacity(1024);
    p.push_str("(version 1)\n(deny default)\n");

    // Process and IPC operations required for child process execution
    p.push_str("(allow process-exec)\n");
    p.push_str("(allow process-fork)\n");
    p.push_str("(allow signal)\n");
    p.push_str("(allow sysctl-read)\n");
    p.push_str("(allow mach-lookup)\n");
    p.push_str("(allow process-info* (target self))\n");

    // Device nodes needed by most processes
    p.push_str("(allow file-read* (subpath \"/dev\"))\n");
    p.push_str("(allow file-write-data (literal \"/dev/null\"))\n");
    p.push_str("(allow file-ioctl (literal \"/dev/null\"))\n");

    // Path traversal: stat() on any path for directory resolution
    // (safe — only exposes file existence/metadata, not contents)
    p.push_str("(allow file-read-metadata)\n");

    // POSIX shared memory needed by dyld for the shared cache
    p.push_str("(allow ipc-posix-shm-read-data)\n");
    p.push_str("(allow ipc-posix-shm-read-metadata)\n");

    // Read-only paths from rules (includes /usr, /lib, /bin, /sbin, /etc)
    for path in &rules.read_only_paths {
        let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
        let _ = writeln!(p, "(allow file-read* (subpath \"{escaped}\"))");
    }

    // macOS-specific read-only paths for frameworks and system libraries
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

    // Read-write paths from rules (includes workspace, /tmp, /var/tmp)
    for path in &rules.read_write_paths {
        let escaped = path.replace('\\', "\\\\").replace('"', "\\\"");
        let _ = writeln!(p, "(allow file-read* file-write* (subpath \"{escaped}\"))");
    }

    // macOS /tmp → /private/tmp, /var → /private/var (symlink targets)
    for rw_path in ["/private/tmp", "/private/var/tmp", "/private/var/folders"] {
        let _ = writeln!(p, "(allow file-read* file-write* (subpath \"{rw_path}\"))");
    }

    // Network access
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
    // No async, no allocations that could deadlock in the success path.
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
#[allow(clippy::unnecessary_wraps)]
pub fn apply_to_command(
    _cmd: &mut tokio::process::Command,
    _rules: &SandboxRules,
) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
