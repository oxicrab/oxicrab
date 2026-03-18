//! URL validation to prevent SSRF attacks.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

/// Validated URL with pinned DNS resolution.
///
/// Contains the resolved addresses so the caller can pin reqwest's DNS
/// resolution, preventing TOCTOU DNS rebinding attacks.
pub struct ResolvedUrl {
    pub host: String,
    pub addrs: Vec<SocketAddr>,
}

/// Validate URL and return resolved addresses for DNS pinning.
///
/// This is the preferred entry point: it validates the URL AND returns
/// the resolved addresses, so the caller can use `reqwest::ClientBuilder::resolve()`
/// to ensure the IP validated is the IP connected to.
///
/// DNS resolution uses `tokio::net::lookup_host` to avoid blocking the async
/// runtime. Falls back to `spawn_blocking` with `std::net` if tokio lookup fails.
pub async fn validate_and_resolve(url_str: &str) -> Result<ResolvedUrl, String> {
    let parsed = url::Url::parse(url_str).map_err(|e| format!("Invalid URL: {e}"))?;

    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "Only http/https allowed, got '{}'",
            parsed.scheme()
        ));
    }

    // Reject URLs with embedded credentials (user:pass@host) to prevent
    // accidental credential leakage in requests
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URLs with embedded credentials are not allowed".to_string());
    }

    let host = parsed.host().ok_or("URL has no host")?;
    let port = parsed.port_or_known_default().unwrap_or(80);
    let host_str = host.to_string();

    let addrs = match host {
        url::Host::Ipv4(v4) => {
            check_ip_allowed(IpAddr::V4(v4))?;
            vec![SocketAddr::new(IpAddr::V4(v4), port)]
        }
        url::Host::Ipv6(v6) => {
            check_ip_allowed(IpAddr::V6(v6))?;
            vec![SocketAddr::new(IpAddr::V6(v6), port)]
        }
        url::Host::Domain(domain) => {
            let lookup_addr = format!("{domain}:{port}");
            let resolved = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::net::lookup_host(&lookup_addr),
            )
            .await
            .map_err(|_| format!("DNS resolution timed out for domain: {domain}"))?
            .map_err(|_| format!("DNS resolution failed for domain: {domain}"))?;
            let mut addrs: Vec<SocketAddr> = resolved.collect();
            for addr in &addrs {
                check_ip_allowed(addr.ip())?;
            }
            if addrs.is_empty() {
                return Err(format!("DNS resolved no addresses for: {domain}"));
            }
            // Prefer IPv4: many hosts advertise AAAA records but have broken
            // IPv6 connectivity.  Sorting IPv4-first lets the pinned client
            // reach a working address on the first attempt.
            addrs.sort_by_key(|a| matches!(a.ip(), IpAddr::V6(_)));
            addrs
        }
    };

    Ok(ResolvedUrl {
        host: host_str,
        addrs,
    })
}

fn check_ip_allowed(ip: IpAddr) -> Result<(), String> {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4.octets()[0] == 0
            // 0.0.0.0/8
            {
                return Err(format!("Blocked: requests to {v4} are not allowed"));
            }
            // CGNAT / shared address space (RFC 6598) - used by cloud providers internally
            let cgnat_start = Ipv4Addr::new(100, 64, 0, 0);
            let cgnat_end = Ipv4Addr::new(100, 127, 255, 255);
            if v4 >= cgnat_start && v4 <= cgnat_end {
                return Err(format!("Blocked: requests to {v4} are not allowed"));
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() {
                return Err(format!("Blocked: requests to {v6} are not allowed"));
            }
            // Check for IPv4-mapped addresses (::ffff:127.0.0.1 etc)
            if let Some(v4) = v6.to_ipv4_mapped() {
                return check_ip_allowed(IpAddr::V4(v4));
            }
            let segments = v6.segments();
            // fe80::/10 link-local
            if segments[0] & 0xffc0 == 0xfe80 {
                return Err(format!("Blocked: requests to {v6} are not allowed"));
            }
            // fc00::/7 unique local
            if segments[0] & 0xfe00 == 0xfc00 {
                return Err(format!("Blocked: requests to {v6} are not allowed"));
            }
            // 2001:db8::/32 documentation
            if segments[0] == 0x2001 && segments[1] == 0x0db8 {
                return Err(format!("Blocked: requests to {v6} are not allowed"));
            }
            // Teredo tunneling (can embed arbitrary IPv4 addresses)
            if segments[0] == 0x2001 && segments[1] == 0x0000 {
                return Err(format!("Blocked: requests to {v6} are not allowed"));
            }
            // 2002::/16 6to4 tunneling (can embed arbitrary IPv4)
            if segments[0] == 0x2002 {
                return Err(format!("Blocked: requests to {v6} are not allowed"));
            }
            // 64:ff9b::/96 NAT64 well-known prefix (maps IPv4 into last 32 bits)
            if segments[0] == 0x0064
                && segments[1] == 0xff9b
                && segments[2] == 0
                && segments[3] == 0
                && segments[4] == 0
                && segments[5] == 0
            {
                let v4 = std::net::Ipv4Addr::new(
                    (segments[6] >> 8) as u8,
                    segments[6] as u8,
                    (segments[7] >> 8) as u8,
                    segments[7] as u8,
                );
                return check_ip_allowed(IpAddr::V4(v4));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
    async fn blocks_nat64_loopback() {
        assert!(validate("http://[64:ff9b::7f00:1]").await.is_err());
    }

    #[tokio::test]
    async fn blocks_nat64_private() {
        assert!(validate("http://[64:ff9b::a00:1]").await.is_err());
    }

    #[tokio::test]
    async fn blocks_nat64_metadata() {
        assert!(validate("http://[64:ff9b::a9fe:a9fe]").await.is_err());
    }

    #[tokio::test]
    async fn allows_nat64_public() {
        assert!(validate("http://[64:ff9b::101:101]").await.is_ok());
    }

    #[tokio::test]
    async fn resolve_returns_addrs_for_domain() {
        let resolved = validate_and_resolve("https://example.com").await;
        assert!(resolved.is_ok());
        let resolved = resolved.unwrap();
        assert_eq!(resolved.host, "example.com");
        assert!(!resolved.addrs.is_empty());
    }
}
