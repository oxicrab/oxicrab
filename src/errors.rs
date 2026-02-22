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

    #[test]
    fn is_retryable_provider() {
        let retryable = OxicrabError::Provider {
            message: "timeout".into(),
            retryable: true,
        };
        assert!(retryable.is_retryable());

        let not_retryable = OxicrabError::Provider {
            message: "bad request".into(),
            retryable: false,
        };
        assert!(!not_retryable.is_retryable());
    }

    #[test]
    fn is_retryable_auth_config() {
        assert!(!OxicrabError::Auth("bad key".into()).is_retryable());
        assert!(!OxicrabError::Config("missing field".into()).is_retryable());
    }

    #[test]
    fn is_retryable_rate_limit() {
        assert!(
            OxicrabError::RateLimit {
                retry_after: Some(30)
            }
            .is_retryable()
        );
    }
}
