use crate::config::CircuitBreakerConfig;
use crate::providers::base::{ChatRequest, LLMProvider, LLMResponse};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{info, warn};

#[derive(Debug, Clone, PartialEq)]
enum CircuitState {
    Closed,
    Open { since: Instant },
    HalfOpen { successes: u32 },
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "Closed"),
            Self::Open { .. } => write!(f, "Open"),
            Self::HalfOpen { successes } => write!(f, "HalfOpen({})", successes),
        }
    }
}

struct BreakerState {
    state: CircuitState,
    consecutive_failures: u32,
    /// Number of in-flight probe requests in `HalfOpen` state.
    /// Prevents concurrent requests from all passing through before
    /// any failure is recorded.
    active_probes: u32,
}

pub struct CircuitBreakerProvider {
    inner: Arc<dyn LLMProvider>,
    breaker: Mutex<BreakerState>,
    config: CircuitBreakerConfig,
}

impl CircuitBreakerProvider {
    pub fn wrap(
        inner: Arc<dyn LLMProvider>,
        config: &CircuitBreakerConfig,
    ) -> Arc<dyn LLMProvider> {
        Arc::new(Self {
            inner,
            breaker: Mutex::new(BreakerState {
                state: CircuitState::Closed,
                consecutive_failures: 0,
                active_probes: 0,
            }),
            config: config.clone(),
        })
    }

    fn is_transient(error: &str) -> bool {
        // First, try typed error downcasting for precise classification
        // (This method receives a string, so we use pattern matching as fallback)
        let lower = error.to_lowercase();
        let non_transient_patterns = [
            "authentication",
            "unauthorized",
            "context length",
            "invalid api key",
            "invalid_api_key",
            "permission",
            "forbidden",
        ];

        // Non-transient errors take priority
        if non_transient_patterns.iter().any(|p| lower.contains(p)) {
            return false;
        }

        // Use word-boundary-aware matching for HTTP status codes to avoid
        // false positives (e.g., "50000" matching "500")
        let transient_words = [
            "rate limit",
            "rate_limit",
            "overloaded",
            "timeout",
            "connection refused",
            "connection reset",
            "broken pipe",
        ];
        if transient_words.iter().any(|p| lower.contains(p)) {
            return true;
        }

        // Match HTTP status codes with word boundaries (preceded/followed by
        // non-digit or string boundary) to avoid false positives
        for code in ["429", "500", "502", "503", "504"] {
            if let Some(pos) = lower.find(code) {
                let before_ok = pos == 0 || !lower.as_bytes()[pos - 1].is_ascii_digit();
                let after_pos = pos + code.len();
                let after_ok =
                    after_pos >= lower.len() || !lower.as_bytes()[after_pos].is_ascii_digit();
                if before_ok && after_ok {
                    return true;
                }
            }
        }

        false
    }

    async fn should_allow(&self) -> Result<(), anyhow::Error> {
        let mut breaker = self.breaker.lock().await;
        match &breaker.state {
            CircuitState::Closed => Ok(()),
            CircuitState::HalfOpen { successes } => {
                // Limit concurrent probes: only allow half_open_probes in-flight at once
                if breaker.active_probes + successes >= self.config.half_open_probes {
                    Err(anyhow::anyhow!(
                        "Circuit breaker is half-open with {} active probe(s). Waiting for results.",
                        breaker.active_probes
                    ))
                } else {
                    breaker.active_probes += 1;
                    Ok(())
                }
            }
            CircuitState::Open { since } => {
                let elapsed = since.elapsed();
                if elapsed.as_secs() >= self.config.recovery_timeout_secs {
                    info!(
                        "circuit breaker transitioning Open -> HalfOpen after {}s",
                        elapsed.as_secs()
                    );
                    breaker.state = CircuitState::HalfOpen { successes: 0 };
                    breaker.active_probes = 1; // This request is the first probe
                    Ok(())
                } else {
                    Err(anyhow::anyhow!(
                        "Circuit breaker is open ({}s remaining). Provider appears to be down.",
                        self.config.recovery_timeout_secs - elapsed.as_secs()
                    ))
                }
            }
        }
    }

    async fn record_success(&self) {
        let mut breaker = self.breaker.lock().await;
        breaker.consecutive_failures = 0;
        if let CircuitState::HalfOpen { successes } = breaker.state {
            breaker.active_probes = breaker.active_probes.saturating_sub(1);
            let new_successes = successes + 1;
            if new_successes >= self.config.half_open_probes {
                info!(
                    "circuit breaker transitioning HalfOpen -> Closed after {} successful probes",
                    new_successes
                );
                breaker.state = CircuitState::Closed;
                breaker.active_probes = 0;
            } else {
                breaker.state = CircuitState::HalfOpen {
                    successes: new_successes,
                };
            }
        }
    }

    async fn record_failure(&self, is_transient: bool) {
        if !is_transient {
            return;
        }
        let mut breaker = self.breaker.lock().await;
        breaker.consecutive_failures += 1;
        let failures = breaker.consecutive_failures;

        match &breaker.state {
            CircuitState::Closed => {
                if failures >= self.config.failure_threshold {
                    warn!(
                        "circuit breaker tripped after {} consecutive failures: Closed -> Open",
                        failures
                    );
                    breaker.state = CircuitState::Open {
                        since: Instant::now(),
                    };
                }
            }
            CircuitState::HalfOpen { .. } => {
                breaker.active_probes = breaker.active_probes.saturating_sub(1);
                warn!("circuit breaker probe failed: HalfOpen -> Open");
                breaker.state = CircuitState::Open {
                    since: Instant::now(),
                };
                breaker.active_probes = 0;
            }
            CircuitState::Open { .. } => {}
        }
    }
}

#[async_trait]
impl LLMProvider for CircuitBreakerProvider {
    async fn chat(&self, req: ChatRequest<'_>) -> anyhow::Result<LLMResponse> {
        self.should_allow().await?;

        match self.inner.chat(req).await {
            Ok(response) => {
                self.record_success().await;
                Ok(response)
            }
            Err(e) => {
                // Use typed error downcasting when available for precise classification,
                // falling back to string matching for untyped errors.
                let transient = e.downcast_ref::<crate::errors::OxicrabError>().map_or_else(
                    || Self::is_transient(&e.to_string()),
                    crate::errors::OxicrabError::is_retryable,
                );
                self.record_failure(transient).await;
                Err(e)
            }
        }
    }

    fn default_model(&self) -> &str {
        self.inner.default_model()
    }

    fn metrics(&self) -> crate::providers::base::ProviderMetrics {
        self.inner.metrics()
    }

    async fn warmup(&self) -> anyhow::Result<()> {
        self.inner.warmup().await
    }
}

#[cfg(test)]
mod tests;
