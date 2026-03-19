/// Validate a user-supplied value that will be interpolated into a URL path segment.
/// Rejects values containing path traversal characters, control characters, or empty values.
/// This is a blacklist approach suitable for API identifiers (IDs, names, slugs).
pub fn validate_url_segment(value: &str, param_name: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("'{param_name}' must not be empty"));
    }
    if value.len() > 500 {
        return Err(format!(
            "'{param_name}' too long ({} chars, max 500)",
            value.len()
        ));
    }
    if value.contains('/') || value.contains("..") {
        return Err(format!("'{param_name}' must not contain '/' or '..'"));
    }
    if value.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Err(format!(
            "'{param_name}' must not contain control characters"
        ));
    }
    Ok(())
}

/// Validate a GitHub-style name (owner, repo, username).
/// Whitelist approach: only allows `[a-zA-Z0-9_.-]`.
pub fn validate_identifier(name: &str, param_name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 100 {
        return Err(format!("'{param_name}' must be 1-100 characters"));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_')
    {
        return Err(format!("'{param_name}' contains invalid characters"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_url_segment_valid() {
        assert!(validate_url_segment("abc123", "id").is_ok());
        assert!(validate_url_segment("some-event-id", "event_id").is_ok());
        assert!(validate_url_segment("ci.yml", "workflow_id").is_ok());
    }

    #[test]
    fn test_validate_url_segment_rejects_traversal() {
        assert!(validate_url_segment("../etc/passwd", "id").is_err());
        assert!(validate_url_segment("foo/bar", "id").is_err());
        assert!(validate_url_segment("..", "id").is_err());
    }

    #[test]
    fn test_validate_url_segment_rejects_control_chars() {
        assert!(validate_url_segment("foo\0bar", "id").is_err());
        assert!(validate_url_segment("foo\nbar", "id").is_err());
        assert!(validate_url_segment("foo\rbar", "id").is_err());
    }

    #[test]
    fn test_validate_url_segment_rejects_empty() {
        assert!(validate_url_segment("", "id").is_err());
    }

    #[test]
    fn test_validate_url_segment_rejects_too_long() {
        let long = "a".repeat(501);
        assert!(validate_url_segment(&long, "id").is_err());
        let ok = "a".repeat(500);
        assert!(validate_url_segment(&ok, "id").is_ok());
    }

    #[test]
    fn test_validate_identifier_valid() {
        assert!(validate_identifier("octocat", "owner").is_ok());
        assert!(validate_identifier("my-repo.v2", "repo").is_ok());
        assert!(validate_identifier("user_name", "name").is_ok());
    }

    #[test]
    fn test_validate_identifier_rejects_special() {
        assert!(validate_identifier("foo/bar", "owner").is_err());
        assert!(validate_identifier("foo bar", "owner").is_err());
        assert!(validate_identifier("", "owner").is_err());
    }

    #[test]
    fn test_validate_identifier_rejects_too_long() {
        let long = "a".repeat(101);
        assert!(validate_identifier(&long, "owner").is_err());
        let ok = "a".repeat(100);
        assert!(validate_identifier(&ok, "owner").is_ok());
    }
}
