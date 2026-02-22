use tokio::process::Command;

/// Environment variables safe to pass through to child processes.
const ALLOWED_ENV_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LANG",
    "LC_ALL",
    "TZ",
    "TERM",
    "RUST_LOG",
    "TMPDIR",
    "XDG_RUNTIME_DIR",
];

/// Create a `Command` with a scrubbed environment.
///
/// Calls `env_clear()` then copies only the allowlisted environment
/// variables from the current process. This prevents accidental leakage
/// of API keys, tokens, and other secrets to child processes.
pub fn scrubbed_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.env_clear();
    for &var in ALLOWED_ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }
    cmd
}

#[cfg(test)]
mod tests;
