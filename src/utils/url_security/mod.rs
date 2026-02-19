//! URL validation to prevent SSRF attacks.

use std::net::{IpAddr, SocketAddr};

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
pub fn validate_and_resolve(url_str: &str) -> Result<ResolvedUrl, String> {
    let parsed = url::Url::parse(url_str).map_err(|e| format!("Invalid URL: {}", e))?;

    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "Only http/https allowed, got '{}'",
            parsed.scheme()
        ));
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
            match std::net::ToSocketAddrs::to_socket_addrs(&(domain, port)) {
                Ok(resolved) => {
                    let addrs: Vec<SocketAddr> = resolved.collect();
                    for addr in &addrs {
                        check_ip_allowed(addr.ip())?;
                    }
                    if addrs.is_empty() {
                        return Err(format!("DNS resolved no addresses for: {}", domain));
                    }
                    addrs
                }
                Err(_) => {
                    return Err(format!("DNS resolution failed for domain: {}", domain));
                }
            }
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
mod tests;
