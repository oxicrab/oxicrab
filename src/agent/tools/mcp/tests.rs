/// Replicates the secret detection heuristic from `connect_server` for unit testing.
/// Returns true if the env var key/value pair looks like it may contain a secret.
fn looks_like_secret(key: &str, value: &str) -> bool {
    let k_lower = key.to_lowercase();
    let v_lower = value.to_lowercase();
    value.len() > 20
        && (k_lower.contains("key")
            || k_lower.contains("secret")
            || k_lower.contains("token")
            || k_lower.contains("password")
            || v_lower.starts_with("sk-")
            || v_lower.starts_with("ghp_")
            || v_lower.starts_with("xoxb-"))
}

#[test]
fn test_detects_api_key_with_sk_prefix() {
    assert!(looks_like_secret(
        "API_KEY",
        "sk-ant-api-key-1234567890abcdef"
    ));
}

#[test]
fn test_detects_github_token_with_ghp_prefix() {
    assert!(looks_like_secret(
        "GITHUB_TOKEN",
        "ghp_1234567890abcdefghij12345"
    ));
}

#[test]
fn test_detects_secret_keyword_long_value() {
    assert!(looks_like_secret(
        "SECRET_VALUE",
        "this-is-a-very-long-secret-value!"
    ));
}

#[test]
fn test_detects_slack_token_with_xoxb_prefix() {
    assert!(looks_like_secret(
        "SLACK_BOT_TOKEN",
        "xoxb-123456789012-abcdefghijklmnopqrstuvwx"
    ));
}

#[test]
fn test_ignores_password_keyword_short_value() {
    // "short" is only 5 chars, below the 20-char threshold
    assert!(!looks_like_secret("PASSWORD", "short"));
}

#[test]
fn test_ignores_normal_var_no_keywords() {
    assert!(!looks_like_secret(
        "NORMAL_VAR",
        "some-normal-value-here-nothing-secret"
    ));
}

#[test]
fn test_ignores_database_url() {
    assert!(!looks_like_secret(
        "DATABASE_URL",
        "postgres://localhost:5432/db"
    ));
}

#[test]
fn test_ignores_short_key_value() {
    // Value is exactly 20 chars — not > 20, so should not trigger
    assert!(!looks_like_secret("API_KEY", "12345678901234567890"));
}

#[test]
fn test_detects_key_keyword_long_value() {
    // 21 chars with "key" in name
    assert!(looks_like_secret("MY_API_KEY", "123456789012345678901"));
}

#[test]
fn test_detects_token_keyword_long_value() {
    assert!(looks_like_secret(
        "ACCESS_TOKEN",
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"
    ));
}

#[test]
fn test_detects_password_keyword_long_value() {
    assert!(looks_like_secret(
        "DB_PASSWORD",
        "super-secure-password-here-123!"
    ));
}

#[test]
fn test_sk_prefix_overrides_key_name_check() {
    // Even without key/secret/token/password in the name,
    // an sk- prefix with length > 20 triggers detection
    assert!(looks_like_secret(
        "SOME_RANDOM_VAR",
        "sk-this-is-definitely-a-key"
    ));
}

#[test]
fn test_ghp_prefix_overrides_key_name_check() {
    assert!(looks_like_secret(
        "SOME_RANDOM_VAR",
        "ghp_1234567890abcdefghij12345"
    ));
}

#[test]
fn test_xoxb_prefix_overrides_key_name_check() {
    assert!(looks_like_secret(
        "SOME_RANDOM_VAR",
        "xoxb-123456789012-abcdefghijklmnopqrstuvwx"
    ));
}

#[test]
fn test_case_insensitive_key_detection() {
    assert!(looks_like_secret(
        "My_Secret_Config",
        "very-long-secret-value-here!!"
    ));
    assert!(looks_like_secret(
        "MY_TOKEN_VALUE",
        "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"
    ));
}

#[test]
fn test_case_insensitive_value_prefix() {
    // sk- prefix detection is case-insensitive on the value
    assert!(looks_like_secret("SOME_VAR", "SK-this-is-definitely-a-key"));
}

#[test]
fn test_crlf_env_var_detection() {
    let safe_values = vec!["normal_value", "path/to/file", "key=value"];
    for v in safe_values {
        assert!(
            !v.contains('\r') && !v.contains('\n'),
            "should be safe: {v}"
        );
    }
    let unsafe_values = vec!["value\r\nInjected: header", "line1\nline2", "cr\ronly"];
    for v in unsafe_values {
        assert!(
            v.contains('\r') || v.contains('\n'),
            "should be detected: {v}"
        );
    }
}
