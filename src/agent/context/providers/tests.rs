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
