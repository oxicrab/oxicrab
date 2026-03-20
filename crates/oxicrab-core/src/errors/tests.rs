use super::*;

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
