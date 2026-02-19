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

        let mut read_write = vec![
            workspace.to_string_lossy().to_string(),
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

#[cfg(not(target_os = "linux"))]
pub fn is_available() -> bool {
    false
}

/// Apply Landlock sandbox rules to a `tokio::process::Command` via `pre_exec`.
///
/// On Linux, sets up filesystem access restrictions and (on ABI v4+) network
/// restrictions. The sandbox is applied with `BestEffort` so partial support
/// degrades gracefully. On non-Linux platforms, this is a no-op.
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

#[cfg(not(target_os = "linux"))]
#[allow(clippy::unnecessary_wraps)]
pub fn apply_to_command(
    _cmd: &mut tokio::process::Command,
    _rules: &SandboxRules,
) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
