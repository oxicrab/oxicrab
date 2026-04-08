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
mod tests;
