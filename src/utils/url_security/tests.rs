use super::*;

/// Helper: `validate_and_resolve` as a pass/fail check
fn validate(url: &str) -> Result<(), String> {
    validate_and_resolve(url).map(|_| ())
}

#[test]
fn allows_public_https() {
    assert!(validate("https://example.com").is_ok());
}

#[test]
fn allows_public_http() {
    assert!(validate("http://example.com/path").is_ok());
}

#[test]
fn blocks_ftp() {
    assert!(validate("ftp://example.com").is_err());
}

#[test]
fn blocks_file() {
    assert!(validate("file:///etc/passwd").is_err());
}

#[test]
fn blocks_localhost() {
    assert!(validate("http://localhost/secret").is_err());
}

#[test]
fn blocks_127_0_0_1() {
    assert!(validate("http://127.0.0.1/admin").is_err());
}

#[test]
fn blocks_loopback_range() {
    assert!(validate("http://127.0.0.2:8080").is_err());
}

#[test]
fn blocks_private_10() {
    assert!(validate("http://10.0.0.1").is_err());
}

#[test]
fn blocks_private_172() {
    assert!(validate("http://172.16.0.1").is_err());
}

#[test]
fn blocks_private_192() {
    assert!(validate("http://192.168.1.1").is_err());
}

#[test]
fn blocks_metadata_endpoint() {
    assert!(validate("http://169.254.169.254/latest/meta-data/").is_err());
}

#[test]
fn blocks_zero_address() {
    assert!(validate("http://0.0.0.0").is_err());
}

#[test]
fn blocks_ipv6_loopback() {
    assert!(validate("http://[::1]:8080").is_err());
}

#[test]
fn blocks_ipv6_unspecified() {
    assert!(validate("http://[::]:8080").is_err());
}

#[test]
fn rejects_no_scheme() {
    assert!(validate("not-a-url").is_err());
}

// --- validate_and_resolve tests ---

#[test]
fn resolve_returns_addrs_for_public_ip() {
    let resolved = validate_and_resolve("http://1.1.1.1/path").unwrap();
    assert_eq!(resolved.host, "1.1.1.1");
    assert!(!resolved.addrs.is_empty());
    assert_eq!(
        resolved.addrs[0].ip(),
        std::net::IpAddr::V4(std::net::Ipv4Addr::new(1, 1, 1, 1))
    );
}

#[test]
fn resolve_blocks_private_ip() {
    assert!(validate_and_resolve("http://192.168.1.1").is_err());
}

#[test]
fn resolve_blocks_localhost() {
    assert!(validate_and_resolve("http://127.0.0.1").is_err());
}

#[test]
fn resolve_returns_addrs_for_domain() {
    let resolved = validate_and_resolve("https://example.com");
    assert!(resolved.is_ok());
    let resolved = resolved.unwrap();
    assert_eq!(resolved.host, "example.com");
    assert!(!resolved.addrs.is_empty());
}
