use super::*;

#[test]
fn allows_public_https() {
    assert!(validate_url("https://example.com").is_ok());
}

#[test]
fn allows_public_http() {
    assert!(validate_url("http://example.com/path").is_ok());
}

#[test]
fn blocks_ftp() {
    assert!(validate_url("ftp://example.com").is_err());
}

#[test]
fn blocks_file() {
    assert!(validate_url("file:///etc/passwd").is_err());
}

#[test]
fn blocks_localhost() {
    assert!(validate_url("http://localhost/secret").is_err());
}

#[test]
fn blocks_127_0_0_1() {
    assert!(validate_url("http://127.0.0.1/admin").is_err());
}

#[test]
fn blocks_loopback_range() {
    assert!(validate_url("http://127.0.0.2:8080").is_err());
}

#[test]
fn blocks_private_10() {
    assert!(validate_url("http://10.0.0.1").is_err());
}

#[test]
fn blocks_private_172() {
    assert!(validate_url("http://172.16.0.1").is_err());
}

#[test]
fn blocks_private_192() {
    assert!(validate_url("http://192.168.1.1").is_err());
}

#[test]
fn blocks_metadata_endpoint() {
    assert!(validate_url("http://169.254.169.254/latest/meta-data/").is_err());
}

#[test]
fn blocks_zero_address() {
    assert!(validate_url("http://0.0.0.0").is_err());
}

#[test]
fn blocks_ipv6_loopback() {
    assert!(validate_url("http://[::1]:8080").is_err());
}

#[test]
fn blocks_ipv6_unspecified() {
    assert!(validate_url("http://[::]:8080").is_err());
}

#[test]
fn rejects_no_scheme() {
    assert!(validate_url("not-a-url").is_err());
}
