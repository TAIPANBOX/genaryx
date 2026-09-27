//! The hostname half of the residency gate (invariant 14, 2026-09-27).
//!
//! `residency::classify_host` can prove a literal IP or `localhost` local or
//! not without touching the network. A Kubernetes Service name (the way the
//! stack's launchers reach the TokenFuse gateway, `http://tokenfuse-gateway:
//! 4100`) or a Compose service name is neither: it is an operator-named alias
//! for whatever address it resolves to right now, and only a DNS lookup can
//! answer that. Two things follow from treating a hostname as a first-class
//! case instead of refusing it outright:
//!
//! 1. EVERY address a checked hostname resolves to must be local - one public
//!    address among several refuses the whole name, and a name that does not
//!    resolve at all is refused too, never treated as local by default.
//! 2. The check must hold at CONNECTION time, not only once when the
//!    provider is built. A name that resolves privately today and publicly
//!    tomorrow (classic DNS rebinding, or simply a Service's backing pod
//!    changing) is exactly the gap a one-time check leaves open. So the SAME
//!    check is wired into the HTTP client itself as a custom DNS resolver
//!    (`ResidencyDnsResolver`, installed via `reqwest::ClientBuilder::
//!    dns_resolver`): the address reqwest hands to its connector is always
//!    the address this gate just checked, never an address checked earlier
//!    and trusted from then on.
//!
//! `HostnameLookup` is the seam that makes both provable without real DNS:
//! `SystemLookup` asks the OS resolver (`getaddrinfo` via `ToSocketAddrs`,
//! the same call reqwest's own default resolver makes internally, via a
//! blocking task exactly like hyper-util's `GaiResolver` does); tests inject
//! a fixed or sequenced table instead, so every test here is deterministic
//! and needs no network reachability.

use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use crate::residency::is_local_ip;

/// Anything that can turn a hostname into the addresses it resolves to right
/// now. Sync, because the production implementation is a blocking OS call;
/// the reqwest-facing resolver below runs it on a blocking task.
pub trait HostnameLookup: Send + Sync {
    fn lookup(&self, host: &str) -> io::Result<Vec<IpAddr>>;
}

/// The OS resolver, via `ToSocketAddrs` - the same `getaddrinfo` path
/// reqwest's own default (unconfigured) resolver uses.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemLookup;

impl HostnameLookup for SystemLookup {
    fn lookup(&self, host: &str) -> io::Result<Vec<IpAddr>> {
        // Port 0: `ToSocketAddrs` needs a port to form a valid socket address
        // string, but only the addresses matter here - `ResidencyDnsResolver`
        // hands them back with port 0 too, and reqwest fills in the URL's own
        // port (see the `Resolve` trait's own doc comment on this).
        Ok((host, 0u16).to_socket_addrs()?.map(|a| a.ip()).collect())
    }
}

/// One error shape for every way the hostname half of the gate refuses:
/// unresolvable, resolves to nothing, or resolves to even one non-local
/// address. Always names the host, the same operator-readable posture as
/// `ProviderError::NonLocalEndpointRefused`.
#[derive(Debug, Clone, thiserror::Error)]
#[error("hostname `{host}` refused by the residency gate: {reason}")]
pub struct ResidencyRefusal {
    pub host: String,
    pub reason: String,
}

impl ResidencyRefusal {
    fn new(host: &str, reason: impl Into<String>) -> Self {
        Self {
            host: host.to_string(),
            reason: reason.into(),
        }
    }
}

/// Resolve `host` and require EVERY address it comes back with, right now,
/// to be loopback/RFC1918/link-local (either IP family - see
/// `residency::is_local_ip`). One public address among several refuses the
/// whole name; an unresolvable name, or one that resolves to nothing, is
/// refused too - never silently treated as local.
pub fn resolve_all_local(
    lookup: &dyn HostnameLookup,
    host: &str,
) -> Result<Vec<IpAddr>, ResidencyRefusal> {
    let addrs = lookup
        .lookup(host)
        .map_err(|e| ResidencyRefusal::new(host, format!("it did not resolve ({e})")))?;
    if addrs.is_empty() {
        return Err(ResidencyRefusal::new(host, "it resolved to no addresses"));
    }
    if let Some(public) = addrs.iter().find(|a| !is_local_ip(**a)) {
        return Err(ResidencyRefusal::new(
            host,
            format!("it resolves to a non-local address ({public})"),
        ));
    }
    Ok(addrs)
}

/// The reqwest DNS resolver every hostname-checked provider client installs
/// via `ClientBuilder::dns_resolver`. Re-runs `resolve_all_local` on every
/// connection reqwest makes through it - not only the one build-time check
/// the provider constructor performed - so the address checked is always the
/// address dialed. A refusal here fails the connection with a
/// `ResidencyRefusal` as its source, never handing a non-local address to
/// the connector at all.
#[derive(Clone)]
pub struct ResidencyDnsResolver {
    lookup: Arc<dyn HostnameLookup>,
}

impl ResidencyDnsResolver {
    pub fn new(lookup: Arc<dyn HostnameLookup>) -> Self {
        Self { lookup }
    }
}

impl Resolve for ResidencyDnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let lookup = self.lookup.clone();
        let host = name.as_str().to_string();
        Box::pin(async move {
            let host_for_task = host.clone();
            let checked = tokio::task::spawn_blocking(move || {
                resolve_all_local(lookup.as_ref(), &host_for_task)
            })
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                Box::new(io::Error::other(format!(
                    "residency resolver task for `{host}` panicked: {e}"
                )))
            })?
            .map_err(|refusal| -> Box<dyn std::error::Error + Send + Sync> { Box::new(refusal) })?;

            let socket_addrs: Vec<SocketAddr> = checked
                .into_iter()
                .map(|ip| SocketAddr::new(ip, 0))
                .collect();
            let iter: Addrs = Box::new(socket_addrs.into_iter());
            Ok(iter)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fixed answer for every call: `Ok(addrs)` or an error.
    struct FixedLookup(Vec<IpAddr>);
    impl HostnameLookup for FixedLookup {
        fn lookup(&self, _host: &str) -> io::Result<Vec<IpAddr>> {
            Ok(self.0.clone())
        }
    }

    struct ErrLookup;
    impl HostnameLookup for ErrLookup {
        fn lookup(&self, _host: &str) -> io::Result<Vec<IpAddr>> {
            Err(io::Error::other("nxdomain"))
        }
    }

    /// Returns each of `answers` in turn (one per call), then repeats the
    /// last one - used to prove the check re-runs at connection time rather
    /// than trusting a build-time answer forever (DNS rebinding).
    struct SequencedLookup {
        answers: Vec<Vec<IpAddr>>,
        calls: AtomicUsize,
    }
    impl SequencedLookup {
        fn new(answers: Vec<Vec<IpAddr>>) -> Self {
            Self {
                answers,
                calls: AtomicUsize::new(0),
            }
        }
    }
    impl HostnameLookup for SequencedLookup {
        fn lookup(&self, _host: &str) -> io::Result<Vec<IpAddr>> {
            let i = self.calls.fetch_add(1, Ordering::SeqCst);
            let idx = i.min(self.answers.len() - 1);
            Ok(self.answers[idx].clone())
        }
    }

    /// Counts calls, so a test can assert the eligibility gate short-circuits
    /// BEFORE any lookup happens (an ineligible hostname must never resolve).
    #[derive(Default)]
    struct CountingLookup {
        calls: AtomicUsize,
    }
    impl HostnameLookup for CountingLookup {
        fn lookup(&self, _host: &str) -> io::Result<Vec<IpAddr>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))])
        }
    }

    #[test]
    fn a_name_resolving_only_to_private_addresses_is_accepted() {
        let lookup = FixedLookup(vec![
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 20)),
        ]);
        assert!(resolve_all_local(&lookup, "tokenfuse-gateway").is_ok());
    }

    #[test]
    fn a_name_resolving_to_one_private_and_one_public_address_is_refused() {
        let lookup = FixedLookup(vec![
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
            IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
        ]);
        let err = resolve_all_local(&lookup, "tokenfuse-gateway").unwrap_err();
        assert!(err.reason.contains("8.8.8.8"), "{err}");
    }

    /// Mutant: "skip the check for IPv6" (only the IPv4 branch of
    /// `is_local_ip` gets exercised, an IPv6 address falls through
    /// unchecked). A public IPv6 address alone must refuse just as a public
    /// IPv4 one does.
    #[test]
    fn a_name_resolving_to_a_public_ipv6_address_is_refused() {
        let lookup = FixedLookup(vec![IpAddr::V6(Ipv6Addr::new(
            0x2606, 0x4700, 0, 0, 0, 0, 0, 0x1111,
        ))]);
        let err = resolve_all_local(&lookup, "some-svc").unwrap_err();
        assert!(err.reason.contains("2606:4700"), "{err}");
    }

    #[test]
    fn a_name_that_does_not_resolve_is_refused() {
        let err = resolve_all_local(&ErrLookup, "typo-svc").unwrap_err();
        assert!(err.reason.contains("nxdomain"), "{err}");
        assert_eq!(err.host, "typo-svc");
    }

    #[test]
    fn a_name_resolving_to_no_addresses_is_refused() {
        let lookup = FixedLookup(Vec::new());
        let err = resolve_all_local(&lookup, "empty-svc").unwrap_err();
        assert!(err.reason.contains("no addresses"), "{err}");
    }

    /// The resolver-level half of property 2: the SAME hostname resolves
    /// privately on its first (build-time) lookup and publicly on its
    /// second (connection-time) one. `ResidencyDnsResolver::resolve` must
    /// refuse the SECOND call even though the first one, checked the same
    /// way, passed - proving the check is not a one-time, cached verdict.
    #[tokio::test]
    async fn the_resolver_rechecks_on_every_call_and_catches_a_later_public_answer() {
        let lookup = Arc::new(SequencedLookup::new(vec![
            vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5))], // build time: private
            vec![IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))],  // connect time: public
        ]));
        let resolver = ResidencyDnsResolver::new(lookup);

        let name: Name = "rebinding-svc".parse().expect("valid dns name");
        let first = resolver.resolve(name).await;
        assert!(first.is_ok(), "the first (build-time-shaped) call is local");

        let name: Name = "rebinding-svc".parse().expect("valid dns name");
        let second = resolver.resolve(name).await;
        assert!(
            second.is_err(),
            "the second (connection-time) call must be refused, not trusted from the first"
        );
        let msg = second.err().unwrap().to_string();
        assert!(msg.contains("8.8.8.8"), "{msg}");
    }

    #[test]
    fn counting_lookup_records_every_call() {
        let counting = CountingLookup::default();
        assert!(resolve_all_local(&counting, "x").is_ok());
        assert_eq!(counting.calls.load(Ordering::SeqCst), 1);
    }
}
