//! The residency gate (docs/PHASE6.md, itrat-console/13 D13.2): with
//! `allow_non_local_endpoints = false` (the default), a provider `base_url`
//! MUST point at a loopback / private / link-local host, so a sensitive
//! install cannot leak prompts or plane data to a public endpoint by
//! misconfiguration. This is the only egress the copilot has, so pinning it to
//! a local host makes "nothing leaves this box" a testable property, not a
//! promise.
//!
//! Classification is intentionally strict: a bare DNS name (anything other than
//! `localhost`) is treated as NON-local, because we cannot prove it resolves to
//! a private address, and the safe default is to refuse.

use std::net::{Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

/// Returns `true` only when `base_url` parses and its host is a loopback,
/// RFC1918 private, or link-local address (or the literal `localhost`).
/// Everything else - public IPs, arbitrary DNS names, un-parseable input -
/// returns `false`.
pub fn is_local_endpoint(base_url: &str) -> bool {
    let Ok(url) = Url::parse(base_url) else {
        return false;
    };
    match url.host() {
        Some(Host::Ipv4(ip)) => is_local_ipv4(ip),
        Some(Host::Ipv6(ip)) => is_local_ipv6(ip),
        Some(Host::Domain(name)) => {
            name.eq_ignore_ascii_case("localhost")
                || name.eq_ignore_ascii_case("localhost.localdomain")
        }
        None => false,
    }
}

/// Loopback `127.0.0.0/8`, RFC1918 `10/8`+`172.16/12`+`192.168/16`, link-local
/// `169.254/16`. `std` classifies each of these on a stable API.
fn is_local_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

/// Loopback `::1`, link-local `fe80::/10`, and unique-local `fc00::/7` (the
/// IPv6 analogue of RFC1918). `std`'s `is_unique_local`/`is_unicast_link_local`
/// are not stable on this toolchain, so the two ranges are checked by prefix.
fn is_local_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() {
        return true;
    }
    let first = ip.segments()[0];
    let is_link_local = (first & 0xffc0) == 0xfe80; // fe80::/10
    let is_unique_local = (first & 0xfe00) == 0xfc00; // fc00::/7
    is_link_local || is_unique_local
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_and_private_are_local() {
        assert!(is_local_endpoint("http://127.0.0.1:11434/v1")); // Ollama default
        assert!(is_local_endpoint("http://localhost:1234/v1")); // LM Studio default
        assert!(is_local_endpoint("http://10.0.0.5:8000/v1")); // RFC1918 internal vLLM
        assert!(is_local_endpoint("http://172.16.3.4:8000")); // RFC1918
        assert!(is_local_endpoint("http://192.168.1.20:11434/v1")); // RFC1918
        assert!(is_local_endpoint("http://[::1]:11434/v1")); // IPv6 loopback
        assert!(is_local_endpoint("http://[fc00::1]:8000")); // IPv6 unique-local
        assert!(is_local_endpoint("http://[fe80::1]:8000")); // IPv6 link-local
    }

    #[test]
    fn public_endpoints_are_not_local() {
        assert!(!is_local_endpoint("https://api.anthropic.com")); // public DNS
        assert!(!is_local_endpoint("https://openrouter.ai/api/v1")); // public DNS
        assert!(!is_local_endpoint("https://api.openai.com/v1")); // public DNS
        assert!(!is_local_endpoint("http://8.8.8.8/v1")); // public IPv4
        assert!(!is_local_endpoint("http://[2606:4700::1111]/v1")); // public IPv6
        assert!(!is_local_endpoint("http://172.32.0.1/v1")); // just OUTSIDE 172.16/12
    }

    #[test]
    fn garbage_is_not_local() {
        assert!(!is_local_endpoint("not a url"));
        assert!(!is_local_endpoint(""));
        assert!(!is_local_endpoint("ollama")); // bare word, not localhost
    }
}
