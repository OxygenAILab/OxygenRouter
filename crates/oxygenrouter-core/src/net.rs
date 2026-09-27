//! Address-list matching shared by the key allowlist and the SSRF filter.
//!
//! Both features take a list of rules an operator typed — CIDRs or bare
//! addresses — and ask "is this address in it". There was one implementation,
//! in `oxygenrouter-proxy`; the storage layer needs the same behaviour for a
//! token's `ip_allowlist` and cannot depend on the proxy crate, so the matcher
//! lives here and both callers share it. A second copy would be a second set of
//! edge cases, and an allowlist that disagrees with itself is a bypass.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use std::net::IpAddr;

/// True when `ip` falls in any listed CIDR or equals a listed address.
///
/// A malformed entry matches nothing rather than everything: an allowlist that
/// a typo can widen is worse than one that refuses.
pub fn ip_listed(ip: IpAddr, list: &[String]) -> bool {
    list.iter().any(|entry| match parse_cidr(entry.trim()) {
        Some((network, prefix)) => cidr_contains(network, prefix, ip),
        None => entry.trim().parse::<IpAddr>().map(|e| e == ip).unwrap_or(false),
    })
}

/// Parse `a.b.c.d/len` or `::/len` into a network address and prefix length.
pub fn parse_cidr(entry: &str) -> Option<(IpAddr, u8)> {
    let (addr, prefix) = match entry.split_once('/') {
        Some((a, p)) => (a.parse::<IpAddr>().ok()?, p.parse::<u8>().ok()?),
        None => return None,
    };
    let max = if addr.is_ipv4() { 32 } else { 128 };
    (prefix <= max).then_some((addr, prefix))
}

pub fn cidr_contains(network: IpAddr, prefix: u8, ip: IpAddr) -> bool {
    match (network, ip) {
        (IpAddr::V4(net), IpAddr::V4(ip)) => {
            // A prefix longer than the family allows would shift past the width
            // and panic. `parse_cidr` already rejects that, but this function is
            // on an authorisation path: it must be total for *any* input, and the
            // cross-family arm below can hand it a v6-sized prefix.
            if prefix > 32 {
                return false;
            }
            let mask = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
            (u32::from(net) & mask) == (u32::from(ip) & mask)
        }
        (IpAddr::V6(net), IpAddr::V6(ip)) => {
            if prefix > 128 {
                return false;
            }
            let mask = if prefix == 0 {
                0u128
            } else {
                u128::MAX << (128 - prefix)
            };
            (u128::from(net) & mask) == (u128::from(ip) & mask)
        }
        // A v6 rule applied to a v4 address. The rule can only reach it when the
        // rule's network is itself v4-mapped, and then its prefix must cover the
        // 96-bit `::ffff:` header — 104 bits becomes 8 bits of v4. The previous
        // code forwarded the raw v6 prefix into the v4 arm, so `::ffff:10.0.0.0/104`
        // reached `32 - 104` and panicked. Reproduced by the test below before
        // the fix; reachable from a configured SSRF list, and now from a token
        // allowlist.
        (IpAddr::V6(net), IpAddr::V4(ip)) => match net.to_ipv4_mapped() {
            Some(v4) if prefix >= 96 => {
                cidr_contains(IpAddr::V4(v4), prefix - 96, IpAddr::V4(ip))
            }
            _ => false,
        },
        // A v4 rule applied to a v4-mapped v6 address: compare in v4 space, where
        // the rule's own prefix is already the right width.
        (IpAddr::V4(net), IpAddr::V6(ip)) => match ip.to_ipv4_mapped() {
            Some(v4) => cidr_contains(IpAddr::V4(net), prefix, IpAddr::V4(v4)),
            None => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().expect("test ip")
    }

    /// Moved here from the proxy's own tests when the matcher was shared, so the
    /// behaviour each caller relies on is pinned in one place.
    #[test]
    fn a_list_matches_by_cidr_or_exact_address() {
        assert!(ip_listed(ip("10.1.2.3"), &["10.0.0.0/8".into()]));
        assert!(!ip_listed(ip("11.1.2.3"), &["10.0.0.0/8".into()]));
        assert!(ip_listed(ip("1.2.3.4"), &["1.2.3.4".into()]));
        assert!(ip_listed(ip("2001:db8::1"), &["2001:db8::/32".into()]));
        assert!(!ip_listed(ip("1.2.3.4"), &["10.0.0.0/8".into()]));
        // Whitespace around an entry is tolerated; operators paste lists.
        assert!(ip_listed(ip("1.2.3.4"), &["  1.2.3.4  ".into()]));
        // An empty list admits nothing, which is the caller's signal to skip the
        // check entirely rather than a grant.
        assert!(!ip_listed(ip("1.2.3.4"), &[]));
    }

    /// A malformed rule must not silently widen the list, and must not panic.
    #[test]
    fn a_malformed_rule_matches_nothing() {
        for bad in ["", "  ", "not-an-ip", "10.0.0.0/", "10.0.0.0/99", "999.1.1.1", "/8"] {
            assert!(
                !ip_listed(ip("10.0.0.0"), &[bad.to_string()]),
                "{bad:?} must not match"
            );
        }
        // A good rule beside a bad one still works.
        assert!(ip_listed(
            ip("10.0.0.0"),
            &["garbage".to_string(), "10.0.0.0/8".to_string()]
        ));
    }

    #[test]
    fn v4_mapped_v6_is_judged_by_v4_rules() {
        assert!(ip_listed(ip("::ffff:10.1.2.3"), &["10.0.0.0/8".into()]));
        assert!(ip_listed(ip("10.1.2.3"), &["::ffff:10.0.0.0/104".into()]));
        // A mapped rule that does not cover the v4 space matches nothing, and
        // must not panic doing it.
        assert!(!ip_listed(ip("10.1.2.3"), &["::ffff:10.0.0.0/95".into()]));
        assert!(!ip_listed(ip("10.1.2.3"), &["2001:db8::/32".into()]));
        // A prefix wider than the family must be refused, not shifted.
        assert!(!ip_listed(ip("10.0.0.0"), &["::/128".into()]));
    }
}
