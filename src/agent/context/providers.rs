use crate::config::ContextProviderConfig;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

struct CachedOutput {
    content: String,
    fetched_at: Instant,
}

pub struct ContextProviderRunner {
    providers: Vec<ContextProviderConfig>,
    cache: Mutex<HashMap<String, CachedOutput>>,
}

impl ContextProviderRunner {
    pub fn new(providers: Vec<ContextProviderConfig>) -> Self {
        Self {
            providers,
            cache: Mutex::new(HashMap::new()),
        }
    }

    pub async fn get_all_context(&self) -> String {
        let mut sections = Vec::new();

        for provider in &self.providers {
            if !provider.enabled {
                continue;
            }

            // Check required binaries
            if !provider.requires_bins.is_empty()
                && !Self::check_bins_available(&provider.requires_bins)
            {
                debug!(
                    "context provider '{}' skipped: missing required binaries",
                    provider.name
                );
                continue;
            }

            // Check required env vars
            if !provider.requires_env.is_empty()
                && !Self::check_env_available(&provider.requires_env)
            {
                debug!(
                    "context provider '{}' skipped: missing required env vars",
                    provider.name
                );
                continue;
            }

            match self.get_provider_output(provider).await {
                Some(output) if !output.trim().is_empty() => {
                    sections.push(format!("### {}\n{}", provider.name, output));
                }
                _ => {}
            }
        }

        if sections.is_empty() {
            return String::new();
        }

        format!("# Dynamic Context\n\n{}", sections.join("\n\n"))
    }

    async fn get_provider_output(&self, provider: &ContextProviderConfig) -> Option<String> {
        // Check cache
        {
            let cache = self.cache.lock().ok()?;
            if let Some(cached) = cache.get(&provider.name)
                && cached.fetched_at.elapsed() < Duration::from_secs(provider.ttl)
            {
                return Some(cached.content.clone());
            }
        }

        // Execute command
        let output = match tokio::time::timeout(
            Duration::from_secs(provider.timeout),
            tokio::process::Command::new(&provider.command)
                .args(&provider.args)
                .output(),
        )
        .await
        {
            Ok(Ok(output)) if output.status.success() => {
                let mut result = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.trim().is_empty() {
                    result.push_str("\n[stderr] ");
                    result.push_str(stderr.trim());
                }
                result
            }
            Ok(Ok(output)) => {
                warn!(
                    "context provider '{}' exited with status {}",
                    provider.name, output.status
                );
                return None;
            }
            Ok(Err(e)) => {
                warn!(
                    "context provider '{}' failed to execute: {}",
                    provider.name, e
                );
                return None;
            }
            Err(_) => {
                warn!(
                    "context provider '{}' timed out after {}s",
                    provider.name, provider.timeout
                );
                return None;
            }
        };

        // Update cache
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(
                provider.name.clone(),
                CachedOutput {
                    content: output.clone(),
                    fetched_at: Instant::now(),
                },
            );
        }

        Some(output)
    }

    fn check_bins_available(bins: &[String]) -> bool {
        bins.iter().all(|bin| which::which(bin).is_ok())
    }

    fn check_env_available(vars: &[String]) -> bool {
        vars.iter().all(|var| std::env::var(var).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_providers() {
        let runner = ContextProviderRunner::new(vec![]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let output = rt.block_on(runner.get_all_context());
        assert!(output.is_empty());
    }

    #[test]
    fn test_disabled_provider_skipped() {
        let runner = ContextProviderRunner::new(vec![ContextProviderConfig {
            name: "test".to_string(),
            command: "echo".to_string(),
            args: vec!["hello".to_string()],
            enabled: false,
            timeout: 5,
            ttl: 300,
            requires_bins: vec![],
            requires_env: vec![],
        }]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let output = rt.block_on(runner.get_all_context());
        assert!(output.is_empty());
    }

    #[test]
    fn test_echo_provider_returns_output() {
        let runner = ContextProviderRunner::new(vec![ContextProviderConfig {
            name: "test".to_string(),
            command: "echo".to_string(),
            args: vec!["hello world".to_string()],
            enabled: true,
            timeout: 5,
            ttl: 300,
            requires_bins: vec![],
            requires_env: vec![],
        }]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let output = rt.block_on(runner.get_all_context());
        assert!(output.contains("hello world"));
        assert!(output.contains("### test"));
        assert!(output.contains("# Dynamic Context"));
    }

    #[test]
    fn test_missing_binary_skipped() {
        let runner = ContextProviderRunner::new(vec![ContextProviderConfig {
            name: "test".to_string(),
            command: "echo".to_string(),
            args: vec![],
            enabled: true,
            timeout: 5,
            ttl: 300,
            requires_bins: vec!["nonexistent_binary_xyz_123".to_string()],
            requires_env: vec![],
        }]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let output = rt.block_on(runner.get_all_context());
        assert!(output.is_empty());
    }

    #[test]
    fn test_ttl_cache() {
        let runner = ContextProviderRunner::new(vec![ContextProviderConfig {
            name: "test".to_string(),
            command: "echo".to_string(),
            args: vec!["cached".to_string()],
            enabled: true,
            timeout: 5,
            ttl: 300,
            requires_bins: vec![],
            requires_env: vec![],
        }]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let output1 = rt.block_on(runner.get_all_context());
        let output2 = rt.block_on(runner.get_all_context());
        assert_eq!(output1, output2);
        // Cache should have an entry
        let cache = runner.cache.lock().unwrap();
        assert!(cache.contains_key("test"));
    }

    #[test]
    fn test_command_timeout() {
        let runner = ContextProviderRunner::new(vec![ContextProviderConfig {
            name: "slow".to_string(),
            command: "sleep".to_string(),
            args: vec!["10".to_string()],
            enabled: true,
            timeout: 1, // 1 second timeout
            ttl: 300,
            requires_bins: vec![],
            requires_env: vec![],
        }]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let output = rt.block_on(runner.get_all_context());
        assert!(
            output.is_empty(),
            "timed-out provider should produce no output"
        );
    }

    #[test]
    fn test_missing_env_var_skipped() {
        let runner = ContextProviderRunner::new(vec![ContextProviderConfig {
            name: "needs-env".to_string(),
            command: "echo".to_string(),
            args: vec!["hi".to_string()],
            enabled: true,
            timeout: 5,
            ttl: 300,
            requires_bins: vec![],
            requires_env: vec!["OXICRAB_NONEXISTENT_TEST_VAR_12345".to_string()],
        }]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let output = rt.block_on(runner.get_all_context());
        assert!(output.is_empty());
    }

    #[test]
    fn test_stderr_included_in_output() {
        // bash -c writes to stderr then stdout
        let runner = ContextProviderRunner::new(vec![ContextProviderConfig {
            name: "stderr-test".to_string(),
            command: "bash".to_string(),
            args: vec![
                "-c".to_string(),
                "echo 'stdout line'; echo 'warning' >&2".to_string(),
            ],
            enabled: true,
            timeout: 5,
            ttl: 300,
            requires_bins: vec![],
            requires_env: vec![],
        }]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let output = rt.block_on(runner.get_all_context());
        assert!(output.contains("stdout line"));
        assert!(output.contains("[stderr] warning"));
    }

    #[test]
    fn test_nonzero_exit_code_skipped() {
        let runner = ContextProviderRunner::new(vec![ContextProviderConfig {
            name: "failing".to_string(),
            command: "bash".to_string(),
            args: vec!["-c".to_string(), "exit 1".to_string()],
            enabled: true,
            timeout: 5,
            ttl: 300,
            requires_bins: vec![],
            requires_env: vec![],
        }]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let output = rt.block_on(runner.get_all_context());
        assert!(output.is_empty());
    }

    #[test]
    fn test_cache_expiration() {
        // TTL of 0 means cache always expires
        let runner = ContextProviderRunner::new(vec![ContextProviderConfig {
            name: "volatile".to_string(),
            command: "bash".to_string(),
            args: vec!["-c".to_string(), "echo $RANDOM".to_string()],
            enabled: true,
            timeout: 5,
            ttl: 0,
            requires_bins: vec![],
            requires_env: vec![],
        }]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        // First call populates cache
        let _ = rt.block_on(runner.get_all_context());
        // With TTL=0, cache entry is always expired, so provider re-executes
        // The cache entry should exist but be stale
        let cache = runner.cache.lock().unwrap();
        assert!(cache.contains_key("volatile"));
    }

    #[test]
    fn test_multiple_providers_combined_output() {
        let runner = ContextProviderRunner::new(vec![
            ContextProviderConfig {
                name: "alpha".to_string(),
                command: "echo".to_string(),
                args: vec!["first".to_string()],
                enabled: true,
                timeout: 5,
                ttl: 300,
                requires_bins: vec![],
                requires_env: vec![],
            },
            ContextProviderConfig {
                name: "beta".to_string(),
                command: "echo".to_string(),
                args: vec!["second".to_string()],
                enabled: true,
                timeout: 5,
                ttl: 300,
                requires_bins: vec![],
                requires_env: vec![],
            },
        ]);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let output = rt.block_on(runner.get_all_context());
        assert!(output.contains("# Dynamic Context"));
        assert!(output.contains("### alpha"));
        assert!(output.contains("first"));
        assert!(output.contains("### beta"));
        assert!(output.contains("second"));
    }
}
