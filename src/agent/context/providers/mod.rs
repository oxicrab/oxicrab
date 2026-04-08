use crate::config::ContextProviderConfig;
use regex::Regex;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Patterns that indicate prompt injection in context provider output.
/// These are stripped before the output enters the system prompt.
static INJECTION_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // Role override markers
        r"(?im)^\s*(?:system|assistant|user)\s*:",
        // Common injection phrases
        r"(?i)\b(?:ignore|disregard|forget)\b.{0,30}\b(?:previous|prior|above|all)\b.{0,20}\b(?:instructions?|prompts?|rules?)\b",
        // New identity injection
        r"(?i)\byou\s+are\s+now\b.{0,50}\b(?:new|different|another)\b",
        // System prompt override
        r"(?i)\b(?:new|override|replace)\b.{0,20}\b(?:system\s+prompt|instructions)\b",
    ]
    .iter()
    .filter_map(|p| Regex::new(p).ok())
    .collect()
});

/// Strip prompt injection markers from context provider output.
fn sanitize_provider_output(output: &str) -> String {
    let mut sanitized = output.to_string();
    for pattern in INJECTION_PATTERNS.iter() {
        if pattern.is_match(&sanitized) {
            warn!(
                "security: prompt injection pattern detected in context provider output — stripping"
            );
            sanitized = pattern.replace_all(&sanitized, "[redacted]").to_string();
        }
    }
    sanitized
}

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
            let cache = self.cache.lock().unwrap_or_else(|poison| {
                warn!("context provider cache mutex was poisoned, recovering");
                poison.into_inner()
            });
            if let Some(cached) = cache.get(&provider.name)
                && cached.fetched_at.elapsed() < Duration::from_secs(provider.ttl)
            {
                return Some(cached.content.clone());
            }
        }

        // Execute command with scrubbed environment
        let output = match tokio::time::timeout(
            Duration::from_secs(provider.timeout),
            crate::utils::subprocess::scrubbed_command(&provider.command)
                .args(&provider.args)
                .output(),
        )
        .await
        {
            Ok(Ok(output)) if output.status.success() => {
                const MAX_PROVIDER_OUTPUT: usize = 100_000; // 100KB
                let stdout = &output.stdout[..output.stdout.len().min(MAX_PROVIDER_OUTPUT)];
                let mut result = String::from_utf8_lossy(stdout).to_string();
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

        debug!(
            "context provider '{}' executed: {} bytes",
            provider.name,
            output.len()
        );

        // Sanitize output for prompt injection before caching
        let output = sanitize_provider_output(&output);

        // Update cache
        {
            let mut cache = self.cache.lock().unwrap_or_else(|poison| {
                warn!("context provider cache mutex was poisoned, recovering");
                poison.into_inner()
            });
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
mod tests;
