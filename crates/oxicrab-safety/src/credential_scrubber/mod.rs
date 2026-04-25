//! Credential scrubbing for tool args and recorded HTTP requests.
//!
//! Adopted from IronClaw [PR #2529](https://github.com/nearai/ironclaw/pull/2529):
//! `LeakDetector` only catches *known secret shapes* (sk-ant-api…, ghp_…,
//! AKIA…). Tool calls routinely carry credentials via header values
//! and URL query params that don't match any known shape — an
//! arbitrary `Authorization: Bearer xxx` or `?api_key=USER_VAR`. Those
//! end up in `cron_execution_traces`, `tool_reflections`, and the DLQ
//! payload field unless we strip them at the recording boundary.
//!
//! The scrubber is **structure-aware**, not pattern-only:
//! - JSON objects with `authorization` / `api_key` / `token` / `secret`
//!   / `password` / `access_token` keys (case-insensitive) get the
//!   value replaced with `"[REDACTED]"`.
//! - URL strings with `?api_key=…&token=…` get the value of each
//!   sensitive query parameter replaced with `[REDACTED]`.
//! - Bare strings that look like a `<sensitive-key>: <value>` line
//!   (e.g. cURL recordings) get the value replaced.
//!
//! False positives on this side cost nothing — a redacted trace is
//! still useful for debugging tool arguments. False negatives leak
//! credentials into the DB.

use serde_json::Value;

/// Header / param keys whose values must be redacted. Match is
/// case-insensitive against the trimmed key.
const SENSITIVE_KEYS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "api_key",
    "apikey",
    "api-key",
    "x-api-key",
    "access_token",
    "accesstoken",
    "access-token",
    "refresh_token",
    "refresh-token",
    "client_secret",
    "clientsecret",
    "client-secret",
    "secret",
    "secret_key",
    "secret-key",
    "token",
    "auth_token",
    "auth-token",
    "password",
    "passwd",
    "pwd",
    "x-auth-token",
    "x-amz-security-token",
    "session_token",
    "session-token",
    "bearer",
];

const REDACTION: &str = "[REDACTED]";

/// Recursively walk a `serde_json::Value` and replace any value whose
/// key matches `SENSITIVE_KEYS` with `[REDACTED]`. Strings whose value
/// itself is a URL get their query params scrubbed too. Arrays and
/// nested objects are walked.
#[must_use]
pub fn scrub_credentials_in_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (key, child) in map {
                if is_sensitive_key(key) {
                    out.insert(key.clone(), Value::String(REDACTION.to_string()));
                } else {
                    out.insert(key.clone(), scrub_credentials_in_json(child));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(scrub_credentials_in_json).collect()),
        Value::String(s) => Value::String(scrub_credentials_in_text(s)),
        other => other.clone(),
    }
}

/// Best-effort URL-aware redaction of a free-form string. Replaces
/// the value of every sensitive query param and any
/// `Header: value` line whose header is sensitive. Non-URL strings
/// are returned unchanged unless they contain a header line.
#[must_use]
pub fn scrub_credentials_in_text(input: &str) -> String {
    let url_redacted = scrub_query_string(input);
    scrub_header_lines(&url_redacted)
}

/// Replace the value of any sensitive query parameter in a URL-shaped
/// string. Operates on substrings between `?` / `&` and the next `&`
/// or end-of-string.
fn scrub_query_string(input: &str) -> String {
    let Some(query_start) = input.find('?') else {
        return input.to_string();
    };
    let (prefix, query_with_marker) = input.split_at(query_start);
    let query = &query_with_marker[1..]; // skip the '?'
    // Allow query strings to also embed a fragment.
    let (query, fragment) = match query.find('#') {
        Some(i) => (&query[..i], &query[i..]),
        None => (query, ""),
    };
    let mut redacted = String::with_capacity(query.len());
    for (idx, part) in query.split('&').enumerate() {
        if idx > 0 {
            redacted.push('&');
        }
        if let Some(eq) = part.find('=') {
            let key = &part[..eq];
            if is_sensitive_key(key) {
                redacted.push_str(key);
                redacted.push('=');
                redacted.push_str(REDACTION);
                continue;
            }
        }
        redacted.push_str(part);
    }
    format!("{prefix}?{redacted}{fragment}")
}

/// Walk a multi-line string and replace the value half of any
/// `Header-Name: value` line whose header is in `SENSITIVE_KEYS`.
fn scrub_header_lines(input: &str) -> String {
    if !input.contains(':') || !input.contains('\n') && !looks_like_single_header(input) {
        // Cheap exit for the common "plain prose" case.
        if !looks_like_single_header(input) {
            return input.to_string();
        }
    }
    let mut out = String::with_capacity(input.len());
    let mut first = true;
    for line in input.split('\n') {
        if !first {
            out.push('\n');
        }
        first = false;
        if let Some(redacted) = redact_header_line(line) {
            out.push_str(&redacted);
        } else {
            out.push_str(line);
        }
    }
    out
}

fn looks_like_single_header(s: &str) -> bool {
    let s = s.trim();
    let Some((key, _)) = s.split_once(':') else {
        return false;
    };
    is_sensitive_key(key.trim())
}

fn redact_header_line(line: &str) -> Option<String> {
    // Match shapes like `Authorization: Bearer …`, `X-Api-Key: …`, `api_key=…`.
    if let Some((key, _value)) = line.split_once(':') {
        let trimmed = key.trim();
        if is_sensitive_key(trimmed) {
            return Some(format!("{key}: {REDACTION}"));
        }
    }
    None
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.trim().to_ascii_lowercase();
    SENSITIVE_KEYS.iter().any(|s| *s == lower)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_sensitive_object_keys() {
        let v = json!({"name": "alice", "api_key": "sk-12345", "nested": {"token": "abc"}});
        let out = scrub_credentials_in_json(&v);
        assert_eq!(out["name"], "alice");
        assert_eq!(out["api_key"], "[REDACTED]");
        assert_eq!(out["nested"]["token"], "[REDACTED]");
    }

    #[test]
    fn case_insensitive_match() {
        let v = json!({"Authorization": "Bearer xxx", "API-KEY": "k"});
        let out = scrub_credentials_in_json(&v);
        assert_eq!(out["Authorization"], "[REDACTED]");
        assert_eq!(out["API-KEY"], "[REDACTED]");
    }

    #[test]
    fn walks_arrays() {
        let v = json!([{"token": "x"}, {"name": "ok"}]);
        let out = scrub_credentials_in_json(&v);
        assert_eq!(out[0]["token"], "[REDACTED]");
        assert_eq!(out[1]["name"], "ok");
    }

    #[test]
    fn redacts_url_query_params() {
        let raw = "https://api.example.com/v1/things?user=alice&api_key=secret123&format=json";
        let out = scrub_credentials_in_text(raw);
        assert!(out.contains("api_key=[REDACTED]"));
        assert!(out.contains("user=alice"));
        assert!(out.contains("format=json"));
    }

    #[test]
    fn redacts_url_query_params_with_fragment() {
        let raw = "https://x.io/p?token=abc&x=1#section";
        let out = scrub_credentials_in_text(raw);
        assert!(out.contains("token=[REDACTED]"));
        assert!(out.contains("x=1"));
        assert!(out.ends_with("#section"));
    }

    #[test]
    fn redacts_authorization_header_line() {
        let raw = "GET /v1/things\nAuthorization: Bearer secret-xyz\nUser-Agent: oxicrab/1.0";
        let out = scrub_credentials_in_text(raw);
        assert!(out.contains("Authorization: [REDACTED]"));
        assert!(out.contains("User-Agent: oxicrab/1.0"));
    }

    #[test]
    fn passes_through_non_sensitive_strings() {
        let raw = "this is a perfectly normal string with a colon: value";
        assert_eq!(scrub_credentials_in_text(raw), raw);
    }

    #[test]
    fn passes_through_url_without_sensitive_params() {
        let raw = "https://example.com/search?q=rust&limit=10";
        assert_eq!(scrub_credentials_in_text(raw), raw);
    }

    #[test]
    fn redacts_value_inside_url_string_field() {
        let v = json!({"url": "https://x.io?api_key=hunter2"});
        let out = scrub_credentials_in_json(&v);
        let url = out["url"].as_str().unwrap();
        assert!(url.contains("api_key=[REDACTED]"));
    }

    #[test]
    fn handles_deeply_nested() {
        let v = json!({"a": {"b": {"c": {"password": "x"}}}});
        let out = scrub_credentials_in_json(&v);
        assert_eq!(out["a"]["b"]["c"]["password"], "[REDACTED]");
    }
}
