use super::*;
use oxicrab_core::config::schema::TranscriptionConfig;

#[cfg(feature = "local-whisper")]
#[test]
fn test_expand_tilde_with_home_prefix() {
    let result = expand_tilde("~/models/whisper.bin");
    // Should not start with ~ — it should be expanded to home dir
    assert!(!result.to_string_lossy().starts_with('~'));
    assert!(result.to_string_lossy().ends_with("models/whisper.bin"));
}

#[cfg(feature = "local-whisper")]
#[test]
fn test_expand_tilde_absolute_path_unchanged() {
    let result = expand_tilde("/usr/local/models/whisper.bin");
    assert_eq!(result, PathBuf::from("/usr/local/models/whisper.bin"));
}

#[cfg(feature = "local-whisper")]
#[test]
fn test_expand_tilde_relative_path_unchanged() {
    let result = expand_tilde("models/whisper.bin");
    assert_eq!(result, PathBuf::from("models/whisper.bin"));
}

#[cfg(feature = "local-whisper")]
#[test]
fn test_expand_tilde_just_tilde_slash() {
    let result = expand_tilde("~/");
    // Should be the home directory
    if let Some(home) = dirs::home_dir() {
        assert_eq!(result, home.join(""));
    }
}

#[cfg(feature = "local-whisper")]
#[test]
fn test_expand_tilde_bare_tilde_not_expanded() {
    // Bare "~" without trailing slash is NOT expanded (strip_prefix("~/") doesn't match)
    let result = expand_tilde("~");
    assert_eq!(result, PathBuf::from("~"));
}

#[test]
fn test_new_disabled_returns_none() {
    let config = TranscriptionConfig {
        enabled: false,
        ..Default::default()
    };
    assert!(TranscriptionService::new(&config).is_none());
}

#[test]
fn test_new_no_backends_returns_none() {
    let config = TranscriptionConfig {
        enabled: true,
        api_key: String::new(),
        local_model_path: String::new(),
        ..Default::default()
    };
    assert!(TranscriptionService::new(&config).is_none());
}

#[test]
fn test_new_cloud_only() {
    let config = TranscriptionConfig {
        enabled: true,
        api_key: "test-key".to_string(),
        local_model_path: String::new(),
        ..Default::default()
    };
    assert!(TranscriptionService::new(&config).is_some());
}
