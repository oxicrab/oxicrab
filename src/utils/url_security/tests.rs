use super::*;

/// Helper: `validate_and_resolve` as a pass/fail check
async fn validate(url: &str) -> Result<(), String> {
    validate_and_resolve(url).await.map(|_| ())
}

#[tokio::test]
async fn allows_public_https() {
    assert!(validate("https://example.com").await.is_ok());
}

#[tokio::test]
async fn allows_public_http() {
    assert!(validate("http://example.com/path").await.is_ok());
}

#[tokio::test]
async fn blocks_ftp() {
    assert!(validate("ftp://example.com").await.is_err());
}

#[tokio::test]
async fn blocks_file() {
    assert!(validate("file:///etc/passwd").await.is_err());
}

#[tokio::test]
async fn blocks_localhost() {
    assert!(validate("http://localhost/secret").await.is_err());
}

#[tokio::test]
async fn blocks_127_0_0_1() {
    assert!(validate("http://127.0.0.1/admin").await.is_err());
}

#[tokio::test]
async fn blocks_loopback_range() {
    assert!(validate("http://127.0.0.2:8080").await.is_err());
}

#[tokio::test]
async fn blocks_private_10() {
    assert!(validate("http://10.0.0.1").await.is_err());
}

#[tokio::test]
async fn blocks_private_172() {
    assert!(validate("http://172.16.0.1").await.is_err());
}

#[tokio::test]
async fn blocks_private_192() {
    assert!(validate("http://192.168.1.1").await.is_err());
}

#[tokio::test]
async fn blocks_metadata_endpoint() {
    assert!(
        validate("http://169.254.169.254/latest/meta-data/")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn blocks_zero_address() {
    assert!(validate("http://0.0.0.0").await.is_err());
}

#[tokio::test]
async fn blocks_ipv6_loopback() {
    assert!(validate("http://[::1]:8080").await.is_err());
}

#[tokio::test]
async fn blocks_ipv6_unspecified() {
    assert!(validate("http://[::]:8080").await.is_err());
}

#[tokio::test]
async fn rejects_no_scheme() {
    assert!(validate("not-a-url").await.is_err());
}

// --- multicast / documentation / 6to4 blocks ---

#[tokio::test]
async fn blocks_ipv4_multicast() {
    assert!(validate("http://224.0.0.1").await.is_err());
    assert!(validate("http://239.255.255.250").await.is_err());
}

#[tokio::test]
async fn blocks_ipv6_multicast() {
    assert!(validate("http://[ff02::1]").await.is_err());
}

#[tokio::test]
async fn blocks_ipv6_documentation() {
    assert!(validate("http://[2001:db8::1]").await.is_err());
}

#[tokio::test]
async fn blocks_ipv6_6to4() {
    assert!(validate("http://[2002::1]").await.is_err());
}

// --- validate_and_resolve tests ---

#[tokio::test]
async fn resolve_returns_addrs_for_public_ip() {
    let resolved = validate_and_resolve("http://1.1.1.1/path").await.unwrap();
    assert_eq!(resolved.host, "1.1.1.1");
    assert!(!resolved.addrs.is_empty());
    assert_eq!(
        resolved.addrs[0].ip(),
        std::net::IpAddr::V4(std::net::Ipv4Addr::new(1, 1, 1, 1))
    );
}

#[tokio::test]
async fn resolve_blocks_private_ip() {
    assert!(validate_and_resolve("http://192.168.1.1").await.is_err());
}

#[tokio::test]
async fn resolve_blocks_localhost() {
    assert!(validate_and_resolve("http://127.0.0.1").await.is_err());
}

#[tokio::test]
async fn resolve_returns_addrs_for_domain() {
    let resolved = validate_and_resolve("https://example.com").await;
    assert!(resolved.is_ok());
    let resolved = resolved.unwrap();
    assert_eq!(resolved.host, "example.com");
    assert!(!resolved.addrs.is_empty());
}
