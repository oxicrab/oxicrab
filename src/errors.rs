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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_is_provider_error() {
        let err = OxicrabError::RateLimit {
            retry_after: Some(30),
        };
        assert!(matches!(err, OxicrabError::RateLimit { .. }));
    }

    #[test]
    fn auth_error_variant() {
        let err = OxicrabError::Auth("invalid key".into());
        assert!(matches!(err, OxicrabError::Auth(..)));
    }
}
