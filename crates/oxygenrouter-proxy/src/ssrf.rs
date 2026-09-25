//! SSRF protection for operator-supplied URLs.
//!
//! A gateway dials whatever URL a channel or a `param()` rule names, so an
//! unguarded fetch is a server-side request forgery primitive: a tenant who can
//! add a channel can point it at `http://169.254.169.254/` (cloud metadata) or
//! `http://127.0.0.1:6379/` (an internal service) and read the reply through the
//! proxy.
//!
//! Mirrors NewAPI's `common/ssrf_protection.go`:
//!
//! * only `http`/`https`;
//! * private, loopback, link-local, multicast and reserved address ranges are
//!   refused unless explicitly allowed (IANA special-purpose registries);
//! * optional domain and IP allow/deny lists;
//! * optional port allow-list;
//! * when `apply_ip_filter_for_domain` is set, a hostname is resolved and every
//!   returned address is checked, which is what closes the DNS-rebinding hole
//!   (a name that resolves to a public IP on the first lookup and a private one
//!   on the second).
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SsrfError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("unsupported protocol `{0}` (only http/https allowed)")]
    UnsupportedScheme(String),
    #[error("invalid port: {0}")]
    InvalidPort(String),
    #[error("port {0} is not allowed")]
    PortNotAllowed(u16),
    #[error("domain not in allowlist: {0}")]
    DomainNotAllowed(String),
    #[error("domain in denylist: {0}")]
    DomainDenied(String),
    #[error("private IP address not allowed: {0}")]
    PrivateIp(String),
    #[error("ip not in allowlist: {0}")]
    IpNotAllowed(String),
    #[error("ip in denylist: {0}")]
    IpDenied(String),
    #[error("DNS resolution failed for {host}: {error}")]
    DnsFailure { host: String, error: String },
}

/// Which side of a filter a list represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    /// Only listed entries are permitted.
    Allowlist,
    /// Everything except listed entries is permitted.
    Denylist,
}

impl Default for FilterMode {
    fn default() -> Self {
        // NewAPI's `fetch_setting` ships both filter modes off, i.e. denylist
        // (`domain_filter_mode: false`, `ip_filter_mode: false`). With empty
        // lists that permits everything, and the private-address rule does the
        // real work.
        Self::Denylist
    }
}

/// SSRF policy.
#[derive(Debug, Clone)]
pub struct SsrfPolicy {
    /// Permit private/loopback/link-local targets. Off by default; operators who
    /// proxy to a LAN model server must turn it on.
    pub allow_private_ip: bool,
    pub domain_mode: FilterMode,
    /// Suffix rules: `example.com` matches `example.com` and `*.example.com`.
    pub domain_list: Vec<String>,
    pub ip_mode: FilterMode,
    /// CIDR or single addresses.
    pub ip_list: Vec<String>,
    /// Allowed ports. Empty means every port is allowed.
    pub allowed_ports: Vec<u16>,
    /// Resolve hostnames and check every address.
    pub apply_ip_filter_for_domain: bool,
    /// Also validate every address the OS resolver returns.
    pub resolve_dns: bool,
}

impl Default for SsrfPolicy {
    fn default() -> Self {
        // Exactly NewAPI's shipped `fetch_setting` defaults, so an unconfigured
        // deployment behaves identically: protection on, private addresses
        // refused, both filters in denylist mode with empty lists, a narrow port
        // allowlist, and hostnames resolved so every address is checked.
        Self {
            allow_private_ip: false,
            domain_mode: FilterMode::Denylist,
            domain_list: Vec::new(),
            ip_mode: FilterMode::Denylist,
            ip_list: Vec::new(),
            allowed_ports: vec![80, 443, 8080, 8443],
            apply_ip_filter_for_domain: true,
            resolve_dns: true,
        }
    }
}

impl SsrfPolicy {
    /// A policy that permits only public HTTP(S) endpoints on standard ports —
    /// the shipped default.
    pub fn public_only() -> Self {
        Self::default()
    }

    /// A policy with no address or port restriction beyond the scheme check.
    /// Used by tests that need a deterministic result without DNS.
    pub fn scheme_only() -> Self {
        Self {
            allow_private_ip: true,
            allowed_ports: Vec::new(),
            resolve_dns: false,
            ..Default::default()
        }
    }

    /// A policy that permits everything. For tests and for a fully trusted
    /// single-operator deployment.
    pub fn permissive() -> Self {
        Self {
            allow_private_ip: true,
            allowed_ports: Vec::new(),
            apply_ip_filter_for_domain: false,
            resolve_dns: false,
            ..Default::default()
        }
    }

    /// Validate a URL before dialing it.
    pub fn validate_url(&self, url_str: &str) -> Result<(), SsrfError> {
        let parsed =
            url::Url::parse(url_str).map_err(|e| SsrfError::InvalidUrl(e.to_string()))?;

        let scheme = parsed.scheme().to_ascii_lowercase();
        if scheme != "http" && scheme != "https" {
            return Err(SsrfError::UnsupportedScheme(scheme));
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| SsrfError::InvalidUrl("URL has no host".to_string()))?
            .to_string();
        let port = parsed.port_or_known_default().ok_or_else(|| {
            SsrfError::InvalidPort(format!("cannot infer a port for scheme `{scheme}`"))
        })?;

        self.validate_network_target(&host, port)
    }

    /// Validate a host and port, resolving the host when configured.
    pub fn validate_network_target(&self, host: &str, port: u16) -> Result<(), SsrfError> {
        let host = host.trim();
        if host.is_empty() {
            return Err(SsrfError::InvalidUrl("empty host".to_string()));
        }

        if !self.allowed_ports.is_empty() && !self.allowed_ports.contains(&port) {
            return Err(SsrfError::PortNotAllowed(port));
        }

        // A literal address is checked directly.
        if let Ok(ip) = host.parse::<IpAddr>() {
            return self.check_ip(host, ip);
        }

        self.check_domain(host)?;

        if self.resolve_dns && (self.apply_ip_filter_for_domain || !self.allow_private_ip) {
            // Every address the name resolves to must pass. Checking all of them
            // (not just the first) is what prevents a name with a public A record
            // and a private AAAA record from slipping through.
            let addrs = (host, port).to_socket_addrs().map_err(|e| SsrfError::DnsFailure {
                host: host.to_string(),
                error: e.to_string(),
            })?;
            let mut checked = 0usize;
            for addr in addrs {
                checked += 1;
                self.check_ip(host, addr.ip())?;
            }
            if checked == 0 {
                return Err(SsrfError::DnsFailure {
                    host: host.to_string(),
                    error: "no addresses returned".to_string(),
                });
            }
        }
        Ok(())
    }

    fn check_domain(&self, host: &str) -> Result<(), SsrfError> {
        let listed = domain_listed(host, &self.domain_list);
        match self.domain_mode {
            FilterMode::Allowlist => {
                // An empty allowlist imposes no domain restriction; the address
                // rules still apply.
                if !self.domain_list.is_empty() && !listed {
                    return Err(SsrfError::DomainNotAllowed(host.to_string()));
                }
            }
            FilterMode::Denylist => {
                if listed {
                    return Err(SsrfError::DomainDenied(host.to_string()));
                }
            }
        }
        Ok(())
    }

    fn check_ip(&self, host: &str, ip: IpAddr) -> Result<(), SsrfError> {
        if is_private_ip(ip) && !self.allow_private_ip {
            return Err(SsrfError::PrivateIp(format!("{host} resolves to {ip}")));
        }

        let listed = ip_listed(ip, &self.ip_list);
        match self.ip_mode {
            FilterMode::Allowlist => {
                if !self.ip_list.is_empty() && !listed {
                    return Err(SsrfError::IpNotAllowed(ip.to_string()));
                }
            }
            FilterMode::Denylist => {
                if listed {
                    return Err(SsrfError::IpDenied(ip.to_string()));
                }
            }
        }
        Ok(())
    }
}

/// True when `host` matches any rule.
///
/// Matching follows NewAPI's `isDomainListed` exactly:
///
/// * a plain entry matches that host and nothing else, so `example.com` does
///   *not* admit `api.example.com` — a subdomain must be named explicitly;
/// * a `*.example.com` entry matches both `example.com` and any subdomain.
///
/// Being stricter than a suffix match matters in allowlist mode: a plain
/// `example.com` entry that also admitted `evil.example.com` would silently
/// widen the allowlist beyond what the operator wrote.
pub fn domain_listed(host: &str, list: &[String]) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    list.iter().any(|rule| {
        let rule = rule.trim().trim_end_matches('.').to_ascii_lowercase();
        if rule.is_empty() {
            return false;
        }
        if host == rule {
            return true;
        }
        match rule.strip_prefix("*.") {
            // The wildcard form also matches the bare suffix, matching NewAPI.
            Some(suffix) => host == suffix || host.ends_with(&format!(".{suffix}")),
            None => false,
        }
    })
}

/// True when `ip` falls in any listed CIDR or equals a listed address.
pub fn ip_listed(ip: IpAddr, list: &[String]) -> bool {
    list.iter().any(|entry| match parse_cidr(entry.trim()) {
        Some((network, prefix)) => cidr_contains(network, prefix, ip),
        None => entry.trim().parse::<IpAddr>().map(|e| e == ip).unwrap_or(false),
    })
}

/// Parse `a.b.c.d/len` or `::/len` into a network address and prefix length.
fn parse_cidr(entry: &str) -> Option<(IpAddr, u8)> {
    let (addr, prefix) = match entry.split_once('/') {
        Some((a, p)) => (a.parse::<IpAddr>().ok()?, p.parse::<u8>().ok()?),
        None => return None,
    };
    let max = if addr.is_ipv4() { 32 } else { 128 };
    (prefix <= max).then_some((addr, prefix))
}

fn cidr_contains(network: IpAddr, prefix: u8, ip: IpAddr) -> bool {
    match (network, ip) {
        (IpAddr::V4(net), IpAddr::V4(ip)) => {
            let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
            (u32::from(net) & mask) == (u32::from(ip) & mask)
        }
        (IpAddr::V6(net), IpAddr::V6(ip)) => {
            let mask = if prefix == 0 {
                0u128
            } else {
                u128::MAX << (128 - prefix)
            };
            (u128::from(net) & mask) == (u128::from(ip) & mask)
        }
        // A v4-mapped v6 address is checked against a v4 rule and vice versa.
        (IpAddr::V4(net), IpAddr::V6(ip)) => match ip.to_ipv4_mapped() {
            Some(v4) => cidr_contains(IpAddr::V4(net), prefix, IpAddr::V4(v4)),
            None => false,
        },
        (IpAddr::V6(net), IpAddr::V4(ip)) => match net.to_ipv4_mapped() {
            Some(v4) => cidr_contains(IpAddr::V4(v4), prefix, IpAddr::V4(ip)),
            None => false,
        },
    }
}

/// True for private, loopback, link-local, multicast, broadcast and reserved
/// addresses, following the IANA special-purpose registries.
pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_private_v4(v4),
        IpAddr::V6(v6) => {
            // An IPv4-mapped address must be judged by its IPv4 rules.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_private_v4(v4);
            }
            v6.is_unspecified()
                || v6.is_loopback()
                || is_unique_local_v6(v6)
                || is_link_local_v6(v6)
                || v6.is_multicast()
                // 2001:db8::/32 documentation, 2001::/23 IETF assignments,
                // 64:ff9b::/96 translation, 100::/64 discard-only.
                || cidr_contains("2001:db8::".parse().unwrap(), 32, IpAddr::V6(v6))
                || cidr_contains("2001::".parse().unwrap(), 23, IpAddr::V6(v6))
                || cidr_contains("64:ff9b::".parse().unwrap(), 96, IpAddr::V6(v6))
                || cidr_contains("100::".parse().unwrap(), 64, IpAddr::V6(v6))
        }
    }
}

fn is_unique_local_v6(v6: Ipv6Addr) -> bool {
    // fc00::/7
    (v6.segments()[0] & 0xfe00) == 0xfc00
}

fn is_link_local_v6(v6: Ipv6Addr) -> bool {
    // fe80::/10
    (v6.segments()[0] & 0xffc0) == 0xfe80
}

fn is_private_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    v4.is_unspecified()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_multicast()
        // 10.0.0.0/8
        || o[0] == 10
        // 100.64.0.0/10 (CGNAT)
        || (o[0] == 100 && (64..=127).contains(&o[1]))
        // 172.16.0.0/12
        || (o[0] == 172 && (16..=31).contains(&o[1]))
        // 192.168.0.0/16
        || (o[0] == 192 && o[1] == 168)
        // 192.0.0.0/24 IETF protocol assignments
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        // 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24 documentation
        || (o[0] == 192 && o[1] == 0 && o[2] == 2)
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)
        // 198.18.0.0/15 benchmarking
        || (o[0] == 198 && (18..=19).contains(&o[1]))
        // 240.0.0.0/4 reserved (is_broadcast covers 255.255.255.255)
        || o[0] >= 240
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("valid ip")
    }

    #[test]
    fn public_addresses_are_allowed_by_default() {
        let policy = SsrfPolicy::public_only();
        assert!(policy.validate_url("https://api.openai.com/v1/chat/completions").is_ok());
        assert!(policy.validate_url("http://93.184.216.34/").is_ok());
    }

    #[test]
    fn the_cloud_metadata_endpoint_is_refused() {
        // The canonical SSRF target: credentials live here on AWS/GCP/Azure.
        let policy = SsrfPolicy::public_only();
        let err = policy.validate_url("http://169.254.169.254/latest/meta-data/");
        assert!(
            matches!(err, Err(SsrfError::PrivateIp(_))),
            "expected a private-IP refusal, got {err:?}"
        );
    }

    #[test]
    fn loopback_and_lan_targets_are_refused() {
        let policy = SsrfPolicy::public_only();
        for url in [
            "http://127.0.0.1:6379/",
            "http://localhost:3000/",
            "http://192.168.1.1/",
            "http://10.0.0.5/",
            "http://172.16.4.4/",
            "http://[::1]:8080/",
            "http://100.64.0.1/",
            "http://0.0.0.0/",
        ] {
            assert!(
                policy.validate_url(url).is_err(),
                "{url} should have been refused"
            );
        }
    }

    #[test]
    fn non_http_schemes_are_refused() {
        let policy = SsrfPolicy::public_only();
        for url in ["file:///etc/passwd", "gopher://x/", "ftp://host/"] {
            assert!(matches!(
                policy.validate_url(url),
                Err(SsrfError::UnsupportedScheme(_))
            ));
        }
    }

    #[test]
    fn private_targets_are_allowed_when_explicitly_permitted() {
        // A deployment that proxies to a LAN model server must be able to opt in.
        let policy = SsrfPolicy {
            allow_private_ip: true,
            ..SsrfPolicy::permissive()
        };
        assert!(policy.validate_url("http://192.168.1.50:8000/v1").is_ok());
        assert!(policy.validate_url("http://127.0.0.1:11434/v1").is_ok());
    }

    #[test]
    fn port_allow_list_is_enforced() {
        let policy = SsrfPolicy {
            allowed_ports: vec![443, 8443],
            // No DNS in a unit test: resolution would make the result depend on
            // the sandbox's network.
            resolve_dns: false,
            ..SsrfPolicy::public_only()
        };
        assert!(policy.validate_url("https://api.example.com/").is_ok());
        assert!(matches!(
            policy.validate_url("https://api.example.com:9999/"),
            Err(SsrfError::PortNotAllowed(9999))
        ));
    }

    #[test]
    fn domain_denylist_blocks_a_host() {
        let policy = SsrfPolicy {
            domain_mode: FilterMode::Denylist,
            // A wildcard is required to cover subdomains; a plain entry would
            // only match the exact host.
            domain_list: vec!["*.evil.example".to_string()],
            resolve_dns: false,
            ..SsrfPolicy::public_only()
        };
        assert!(matches!(
            policy.validate_network_target("api.evil.example", 443),
            Err(SsrfError::DomainDenied(_))
        ));
        assert!(policy.validate_network_target("api.good.example", 443).is_ok());
    }

    #[test]
    fn domain_allowlist_admits_only_listed_hosts() {
        let policy = SsrfPolicy {
            domain_mode: FilterMode::Allowlist,
            domain_list: vec!["*.example.com".to_string()],
            resolve_dns: false,
            ..SsrfPolicy::public_only()
        };
        assert!(policy.validate_network_target("api.example.com", 443).is_ok());
        assert!(policy.validate_network_target("example.com", 443).is_ok());
        assert!(matches!(
            policy.validate_network_target("other.test", 443),
            Err(SsrfError::DomainNotAllowed(_))
        ));
        // A lookalike must not match.
        assert!(matches!(
            policy.validate_network_target("notexample.com", 443),
            Err(SsrfError::DomainNotAllowed(_))
        ));
    }

    #[test]
    fn domain_matching_is_case_and_dot_insensitive() {
        assert!(domain_listed("API.Example.COM", &["api.example.com".into()]));
        assert!(domain_listed("api.example.com.", &["api.example.com".into()]));
        assert!(domain_listed("deep.api.example.com", &["*.example.com".into()]));
        // NewAPI's wildcard rule also matches the bare suffix.
        assert!(domain_listed("example.com", &["*.example.com".into()]));
    }

    #[test]
    fn a_plain_entry_matches_only_that_host() {
        // Parity with NewAPI: a plain entry is an exact match, so it must not
        // silently widen an allowlist to every subdomain.
        assert!(domain_listed("example.com", &["example.com".into()]));
        assert!(!domain_listed("api.example.com", &["example.com".into()]));
        assert!(!domain_listed("evil.example.com", &["example.com".into()]));
        // A wildcard is how a subdomain is admitted.
        assert!(domain_listed("api.example.com", &["*.example.com".into()]));
    }

    #[test]
    fn ip_allowlist_and_cidr_matching() {
        assert!(ip_listed(ip("10.1.2.3"), &["10.0.0.0/8".into()]));
        assert!(!ip_listed(ip("11.1.2.3"), &["10.0.0.0/8".into()]));
        assert!(ip_listed(ip("1.2.3.4"), &["1.2.3.4".into()]));
        assert!(ip_listed(ip("2001:db8::1"), &["2001:db8::/32".into()]));
        assert!(!ip_listed(ip("1.2.3.4"), &["10.0.0.0/8".into()]));
    }

    #[test]
    fn ip_denylist_blocks_a_listed_address() {
        let policy = SsrfPolicy {
            ip_mode: FilterMode::Denylist,
            ip_list: vec!["203.0.113.0/24".to_string()],
            ..SsrfPolicy::permissive()
        };
        assert!(matches!(
            policy.validate_network_target("203.0.113.7", 443),
            Err(SsrfError::IpDenied(_))
        ));
        assert!(policy.validate_network_target("8.8.8.8", 443).is_ok());
    }

    #[test]
    fn private_ranges_are_classified_correctly() {
        for addr in [
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.254",
            "192.168.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.1",
            "198.18.0.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "0.0.0.0",
        ] {
            assert!(is_private_ip(ip(addr)), "{addr} should be private");
        }
        for addr in ["8.8.8.8", "1.1.1.1", "93.184.216.34", "172.32.0.1", "172.15.0.1"] {
            assert!(!is_private_ip(ip(addr)), "{addr} should be public");
        }
    }

    #[test]
    fn ipv6_private_ranges_are_classified_correctly() {
        for addr in ["::1", "::", "fc00::1", "fd12:3456::1", "fe80::1", "ff02::1", "2001:db8::1"] {
            assert!(is_private_ip(ip(addr)), "{addr} should be private");
        }
        assert!(!is_private_ip(ip("2606:4700:4700::1111")));
    }

    #[test]
    fn ipv4_mapped_ipv6_is_judged_by_ipv4_rules() {
        // A common bypass attempt: encode a private v4 address as v6.
        assert!(is_private_ip(ip("::ffff:127.0.0.1")));
        assert!(is_private_ip(ip("::ffff:10.0.0.1")));
        assert!(!is_private_ip(ip("::ffff:8.8.8.8")));

        let policy = SsrfPolicy::public_only();
        assert!(
            policy.validate_url("http://[::ffff:127.0.0.1]/").is_err(),
            "a v4-mapped loopback must be refused"
        );
    }

    #[test]
    fn cidr_parsing_rejects_bad_prefixes() {
        assert!(parse_cidr("10.0.0.0/8").is_some());
        assert!(parse_cidr("10.0.0.0/33").is_none());
        assert!(parse_cidr("2001:db8::/129").is_none());
        assert!(parse_cidr("not-an-ip/8").is_none());
        assert!(parse_cidr("10.0.0.1").is_none());
        assert!(parse_cidr("0.0.0.0/0").is_some());
    }

    #[test]
    fn a_slash_zero_rule_matches_everything() {
        assert!(cidr_contains("0.0.0.0".parse().unwrap(), 0, ip("8.8.8.8")));
        assert!(ip_listed(ip("8.8.8.8"), &["0.0.0.0/0".into()]));
    }

    #[test]
    fn malformed_urls_are_rejected() {
        let policy = SsrfPolicy::public_only();
        assert!(matches!(
            policy.validate_url("not a url"),
            Err(SsrfError::InvalidUrl(_))
        ));
        assert!(matches!(
            policy.validate_url("http://"),
            Err(SsrfError::InvalidUrl(_))
        ));
    }

    #[test]
    fn the_permissive_policy_admits_private_targets() {
        // Needed for the local test upstream and LAN model servers.
        let policy = SsrfPolicy::permissive();
        assert!(policy.validate_url("http://127.0.0.1:3000/v1").is_ok());
        assert!(policy.validate_url("http://192.168.1.9:11434/api/chat").is_ok());
        assert!(policy.validate_url("https://api.example.com/v1").is_ok());
    }
}