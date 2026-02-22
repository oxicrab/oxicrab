use super::*;
use crate::errors::OxicrabError;

#[test]
fn test_parse_api_error_with_json_body() {
    let error_json = r#"{"error": {"type": "invalid_request", "message": "bad request"}}"#;
    let result = ProviderErrorHandler::parse_api_error(400, error_json);
    let err = result.unwrap_err();
    match err {
        OxicrabError::Provider { message, retryable } => {
            assert!(message.contains("invalid_request"));
            assert!(message.contains("bad request"));
            assert!(!retryable);
        }
        _ => panic!("expected Provider error, got {:?}", err),
    }
}

#[test]
fn test_parse_api_error_retryable_500() {
    let error_json = r#"{"error": {"type": "server_error", "message": "internal"}}"#;
    let result = ProviderErrorHandler::parse_api_error(500, error_json);
    let err = result.unwrap_err();
    match err {
        OxicrabError::Provider { retryable, .. } => assert!(retryable),
        _ => panic!("expected Provider error"),
    }
}

#[test]
fn test_parse_api_error_retryable_502() {
    let error_json = r#"{"error": {"type": "overloaded", "message": "busy"}}"#;
    let result = ProviderErrorHandler::parse_api_error(502, error_json);
    let err = result.unwrap_err();
    match err {
        OxicrabError::Provider { retryable, .. } => assert!(retryable),
        _ => panic!("expected Provider error"),
    }
}

#[test]
fn test_parse_api_error_retryable_503() {
    let error_json = r#"{"error": {"type": "overloaded", "message": "busy"}}"#;
    let result = ProviderErrorHandler::parse_api_error(503, error_json);
    let err = result.unwrap_err();
    match err {
        OxicrabError::Provider { retryable, .. } => assert!(retryable),
        _ => panic!("expected Provider error"),
    }
}

#[test]
fn test_parse_api_error_not_retryable_400() {
    let error_json = r#"{"error": {"type": "bad_request", "message": "invalid"}}"#;
    let result = ProviderErrorHandler::parse_api_error(400, error_json);
    let err = result.unwrap_err();
    match err {
        OxicrabError::Provider { retryable, .. } => assert!(!retryable),
        _ => panic!("expected Provider error"),
    }
}

#[test]
fn test_parse_api_error_non_json_body() {
    let result = ProviderErrorHandler::parse_api_error(500, "plain text error");
    let err = result.unwrap_err();
    match err {
        OxicrabError::Provider { message, retryable } => {
            assert!(message.contains("500"));
            assert!(message.contains("plain text error"));
            assert!(retryable);
        }
        _ => panic!("expected Provider error"),
    }
}

#[test]
fn test_parse_api_error_model_not_found() {
    let error_json = r#"{"error": {"type": "not_found_error", "message": "model: claude-old"}}"#;
    let result = ProviderErrorHandler::parse_api_error(404, error_json);
    let err = result.unwrap_err();
    match err {
        OxicrabError::Provider { message, retryable } => {
            assert!(message.contains("not found"));
            assert!(message.contains("claude-sonnet-4-6"));
            assert!(!retryable);
        }
        _ => panic!("expected Provider error"),
    }
}

#[test]
fn test_handle_rate_limit_with_retry_after() {
    let result = ProviderErrorHandler::handle_rate_limit(429, Some(30));
    let err = result.unwrap_err();
    match err {
        OxicrabError::RateLimit { retry_after } => {
            assert_eq!(retry_after, Some(30));
        }
        _ => panic!("expected RateLimit error"),
    }
}

#[test]
fn test_handle_rate_limit_without_retry_after() {
    let result = ProviderErrorHandler::handle_rate_limit(429, None);
    let err = result.unwrap_err();
    match err {
        OxicrabError::RateLimit { retry_after } => {
            assert_eq!(retry_after, None);
        }
        _ => panic!("expected RateLimit error"),
    }
}

#[test]
fn test_handle_auth_error() {
    let result = ProviderErrorHandler::handle_auth_error(401, "invalid token");
    let err = result.unwrap_err();
    match err {
        OxicrabError::Auth(msg) => {
            assert!(msg.contains("invalid token"));
            assert!(msg.contains("Authentication failed"));
        }
        _ => panic!("expected Auth error"),
    }
}
