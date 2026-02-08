//! URL validation to prevent SSRF attacks.

use std::net::IpAddr;

/// Validate that a URL is safe to fetch (no SSRF to internal services).
///
/// Blocks:
/// - Non-http(s) schemes
/// - Loopback addresses (127.0.0.0/8, ::1)
/// - Private networks (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16)
/// - Link-local (169.254.0.0/16, fe80::/10)
/// - Cloud metadata endpoints (169.254.169.254)
/// - Unspecified addresses (0.0.0.0, ::)
pub fn validate_url(url_str: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url_str).map_err(|e| format!("Invalid URL: {}", e))?;

    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "Only http/https allowed, got '{}'",
            parsed.scheme()
        ));
    }

    let host = parsed.host().ok_or("URL has no host")?;

    match host {
        url::Host::Ipv4(v4) => check_ip_allowed(IpAddr::V4(v4))?,
        url::Host::Ipv6(v6) => check_ip_allowed(IpAddr::V6(v6))?,
        url::Host::Domain(domain) => {
            // Hostname — resolve via DNS to check the actual IP
            // Use std::net for synchronous resolution (sufficient for validation)
            match std::net::ToSocketAddrs::to_socket_addrs(&(domain, 80)) {
                Ok(addrs) => {
                    for addr in addrs {
                        check_ip_allowed(addr.ip())?;
                    }
                }
                Err(_) => {
                    // DNS resolution failed — allow through (will fail at fetch time)
                }
            }
        }
    }

    Ok(())
}

fn check_ip_allowed(ip: IpAddr) -> Result<(), String> {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.octets()[0] == 0
            // 0.0.0.0/8
            {
                return Err(format!("Blocked: requests to {} are not allowed", v4));
            }
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() || v6.is_unspecified() {
                return Err(format!("Blocked: requests to {} are not allowed", v6));
            }
            // Check for IPv4-mapped addresses (::ffff:127.0.0.1 etc)
            if let Some(v4) = v6.to_ipv4_mapped() {
                return check_ip_allowed(IpAddr::V4(v4));
            }
            // fe80::/10 link-local
            let segments = v6.segments();
            if segments[0] & 0xffc0 == 0xfe80 {
                return Err(format!("Blocked: requests to {} are not allowed", v6));
            }
            // fc00::/7 unique local
            if segments[0] & 0xfe00 == 0xfc00 {
                return Err(format!("Blocked: requests to {} are not allowed", v6));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
