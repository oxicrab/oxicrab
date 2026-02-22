use thiserror::Error;

/// Typed error hierarchy for oxicrab.
///
/// Use at module boundaries (provider calls, tool execution, config validation, sessions).
/// Internal/leaf functions can continue using `anyhow::Result` — the `Internal` variant
/// allows seamless conversion via the `?` operator.
#[derive(Debug, Error)]
pub enum OxicrabError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Provider error: {message}")]
    Provider { message: String, retryable: bool },

    #[error("Rate limit exceeded")]
    RateLimit { retry_after: Option<u64> },

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl OxicrabError {
    /// Whether this error is transient and the operation should be retried.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Provider { retryable, .. } => *retryable,
            Self::RateLimit { .. } | Self::Internal(_) => true,
            Self::Auth(_) | Self::Config(_) => false,
        }
    }
}

#[cfg(test)]
mod tests;
