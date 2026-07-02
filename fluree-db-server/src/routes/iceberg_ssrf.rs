//! SSRF guard for the Iceberg onboarding endpoints.
//!
//! The Iceberg map / browse / preview / generate / validate routes take catalog
//! URLs straight from the request body — which is pass-through-unauthenticated
//! when `admin_auth.mode == None` (the default) — and hand them to an outbound
//! HTTP client. Without a guard, `catalog_uri` / `oauth2_token_url` /
//! `s3_endpoint` are a server-side request forgery vector (e.g.
//! `http://169.254.169.254/latest/meta-data/…` to reach cloud metadata).
//!
//! This module blocks that **regardless of auth mode**: only `http(s)` schemes
//! are allowed, and the host — or, for a DNS name, *every* address it resolves
//! to — must be a public unicast address. Resolving and checking every returned
//! address also blunts DNS-rebinding to an internal address.
//!
//! Residual risk: `reqwest` re-resolves at connect time, so a name that flips
//! from public to private *between* this check and the request could still slip
//! through. Fully closing that needs IP-pinned connections; the checks here stop
//! the practical vectors (literal internal IPs and names that resolve internal).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::error::{Result, ServerError};

/// Guard the request-supplied outbound URLs on an Iceberg connection: the REST
/// `catalog_uri`, the `oauth2_token_url`, and any `s3_endpoint` override. Each,
/// when present, must pass [`guard_outbound_url`].
pub async fn guard_connection_urls(
    catalog_uri: Option<&str>,
    oauth2_token_url: Option<&str>,
    s3_endpoint: Option<&str>,
) -> Result<()> {
    if let Some(u) = catalog_uri {
        guard_outbound_url(u, "catalog_uri").await?;
    }
    if let Some(u) = oauth2_token_url {
        guard_outbound_url(u, "oauth2_token_url").await?;
    }
    if let Some(u) = s3_endpoint {
        guard_outbound_url(u, "s3_endpoint").await?;
    }
    Ok(())
}

/// Validate a single request-supplied outbound URL: an allowed scheme plus a
/// host that neither is, nor resolves to, a private / loopback / link-local /
/// metadata address. `field` names the offending field for the error message.
pub async fn guard_outbound_url(raw: &str, field: &str) -> Result<()> {
    let url = reqwest::Url::parse(raw)
        .map_err(|e| ServerError::bad_request(format!("{field} is not a valid URL: {e}")))?;

    match url.scheme() {
        "https" | "http" => {}
        other => {
            return Err(ServerError::bad_request(format!(
                "{field} scheme '{other}' is not allowed (use https or http)"
            )));
        }
    }

    let host = url
        .host_str()
        .ok_or_else(|| ServerError::bad_request(format!("{field} has no host")))?;

    // Literal IP host: check directly, no DNS.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if ip_is_blocked(ip) {
            return Err(blocked(field, host, ip));
        }
        return Ok(());
    }

    // Hostname: resolve and check EVERY address it maps to — an attacker
    // controls their own DNS, so a name pointing at an internal address (incl.
    // via DNS rebinding) must be rejected. The port is irrelevant to the
    // address check; use the scheme default.
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs = tokio::net::lookup_host((host, port)).await.map_err(|e| {
        ServerError::bad_request(format!("{field} host '{host}' did not resolve: {e}"))
    })?;
    let mut any = false;
    for sa in addrs {
        any = true;
        if ip_is_blocked(sa.ip()) {
            return Err(blocked(field, host, sa.ip()));
        }
    }
    if !any {
        return Err(ServerError::bad_request(format!(
            "{field} host '{host}' did not resolve to any address"
        )));
    }
    Ok(())
}

fn blocked(field: &str, host: &str, ip: IpAddr) -> ServerError {
    ServerError::bad_request(format!(
        "{field} host '{host}' resolves to a blocked \
         (private/loopback/link-local/metadata) address {ip}"
    ))
}

/// Whether an IP must be blocked as a non-public / internal target.
///
/// IPv4: loopback (127/8), private (RFC1918), link-local (169.254/16, which
/// includes the `169.254.169.254` cloud-metadata address), CGNAT shared space
/// (100.64/10), unspecified (0.0.0.0), and broadcast. IPv6: loopback (`::1`),
/// unspecified (`::`), unique-local (`fc00::/7`) and link-local (`fe80::/10`),
/// plus IPv4-mapped addresses unwrapped to their v4 form. (`Ipv6Addr`'s
/// `is_unique_local` / `is_unicast_link_local` are still unstable, so those two
/// ranges are checked by hand.)
fn ip_is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => ipv4_is_blocked(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return ipv4_is_blocked(v4);
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || is_ipv6_unique_local(v6)
                || is_ipv6_link_local(v6)
        }
    }
}

fn ipv4_is_blocked(v4: Ipv4Addr) -> bool {
    v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || is_ipv4_shared(v4)
}

/// RFC 6598 shared address space (100.64.0.0/10, carrier-grade NAT).
fn is_ipv4_shared(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    o[0] == 100 && (o[1] & 0xc0) == 0x40
}

/// `fc00::/7` unique-local addresses.
fn is_ipv6_unique_local(v6: Ipv6Addr) -> bool {
    (v6.octets()[0] & 0xfe) == 0xfc
}

/// `fe80::/10` link-local unicast addresses.
fn is_ipv6_link_local(v6: Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xffc0) == 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_internal_ipv4_ranges() {
        for ip in [
            "169.254.169.254", // cloud metadata (link-local)
            "127.0.0.1",       // loopback
            "10.0.0.1",        // private
            "172.16.5.4",      // private
            "192.168.1.1",     // private
            "100.64.0.1",      // CGNAT shared
            "0.0.0.0",         // unspecified
            "255.255.255.255", // broadcast
        ] {
            assert!(ip_is_blocked(ip.parse().unwrap()), "{ip} must be blocked");
        }
    }

    #[test]
    fn allows_public_ipv4() {
        for ip in ["8.8.8.8", "1.1.1.1", "93.184.216.34"] {
            assert!(!ip_is_blocked(ip.parse().unwrap()), "{ip} must be allowed");
        }
    }

    #[test]
    fn blocks_internal_ipv6_including_mapped() {
        for ip in [
            "::1",                    // loopback
            "::",                     // unspecified
            "fc00::1",                // unique-local
            "fd12:3456::1",           // unique-local
            "fe80::1",                // link-local
            "::ffff:10.0.0.1",        // v4-mapped private
            "::ffff:169.254.169.254", // v4-mapped metadata
        ] {
            assert!(ip_is_blocked(ip.parse().unwrap()), "{ip} must be blocked");
        }
        // A public IPv6 (Cloudflare) is allowed.
        assert!(!ip_is_blocked("2606:4700:4700::1111".parse().unwrap()));
    }

    #[tokio::test]
    async fn guard_blocks_metadata_and_private_literals() {
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/",
            "https://192.168.0.5/catalog",
            "http://[::1]:8080/",
            "http://127.0.0.1:9000/",
        ] {
            assert!(
                guard_outbound_url(url, "catalog_uri").await.is_err(),
                "{url} must be blocked"
            );
        }
    }

    #[tokio::test]
    async fn guard_blocks_non_http_schemes() {
        for url in [
            "file:///etc/passwd",
            "ftp://8.8.8.8/x",
            "gopher://8.8.8.8/",
            "not even a url",
        ] {
            assert!(
                guard_outbound_url(url, "catalog_uri").await.is_err(),
                "{url} must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn guard_allows_public_literal_ip() {
        // A literal public IP needs no DNS, so this stays network-free.
        assert!(
            guard_outbound_url("https://8.8.8.8/v1/config", "catalog_uri")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn guard_blocks_localhost_by_resolution() {
        // `localhost` resolves (via the system resolver / hosts file) to a
        // loopback address and must be blocked through the resolve-and-check
        // path, not just literal-IP matching.
        assert!(
            guard_outbound_url("http://localhost:8181/catalog", "catalog_uri")
                .await
                .is_err()
        );
    }
}
