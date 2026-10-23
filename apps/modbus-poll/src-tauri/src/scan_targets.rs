//! The hosts a network scan visits, from what the user typed: a prefix (`192.168.1`, the 254 hosts of that /24),
//! a CIDR range (`192.168.1.0/24`, `10.0.4.0/22`) or one address.

use std::net::Ipv4Addr;

/// The most hosts one scan visits. A /22 is 1022 hosts; a /16 would be 65534 connection attempts.
pub const MAX_SCAN_HOSTS: u32 = 1024;

pub fn scan_targets(input: &str) -> Result<Vec<Ipv4Addr>, String> {
    let text = input.trim();
    if text.is_empty() {
        return Err("Enter a range such as 192.168.1 or 192.168.1.0/24".into());
    }
    let bad = || format!("'{text}' is not a range; use a prefix like 192.168.1, a CIDR like 192.168.1.0/24, or one address");

    let (addr_part, prefix_len) = match text.split_once('/') {
        Some((a, n)) => (a, Some(n.trim().parse::<u32>().map_err(|_| bad())?)),
        None => (text, None),
    };
    let octets: Vec<&str> = addr_part.trim().split('.').collect();

    // `a.b.c` is the /24 around it.
    if prefix_len.is_none() && octets.len() == 3 {
        let o = octets.iter().map(|s| s.parse::<u8>()).collect::<Result<Vec<_>, _>>().map_err(|_| bad())?;
        return Ok((1..=254u8).map(|h| Ipv4Addr::new(o[0], o[1], o[2], h)).collect());
    }

    let addr: Ipv4Addr = addr_part.trim().parse().map_err(|_| bad())?;
    let len = prefix_len.unwrap_or(32);
    if len > 32 {
        return Err(format!("A prefix length is 0 to 32, not {len}"));
    }
    let host_bits = 32 - len;
    let count = 1u64 << host_bits;
    if count > u64::from(MAX_SCAN_HOSTS) + 2 {
        let usable = count - 2;
        return Err(format!(
            "/{len} holds {usable} hosts; a scan visits at most {MAX_SCAN_HOSTS}. Use /22 or a smaller range (more bits after the slash)"
        ));
    }
    let mask = if host_bits == 32 { 0 } else { u32::MAX << host_bits };
    let network = u32::from(addr) & mask;
    let (first, last) = match len {
        // A /32 is one host and a /31 is both of its addresses; larger ranges skip the network and broadcast addresses.
        31..=32 => (network, network + (count as u32 - 1)),
        _ => (network + 1, network + (count as u32 - 2)),
    };
    Ok((first..=last).map(Ipv4Addr::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn a_prefix_is_the_254_hosts_of_that_24() {
        let t = scan_targets("192.168.1").unwrap();
        assert_eq!(t.len(), 254);
        assert_eq!((t[0], t[253]), (ip("192.168.1.1"), ip("192.168.1.254")));
    }

    #[test]
    fn a_24_cidr_matches_the_prefix_form() {
        assert_eq!(scan_targets("192.168.1.0/24").unwrap(), scan_targets("192.168.1").unwrap());
        // Any host address inside the range selects the same network.
        assert_eq!(scan_targets("192.168.1.77/24").unwrap(), scan_targets("192.168.1").unwrap());
    }

    #[test]
    fn a_small_cidr_visits_only_its_usable_hosts() {
        let t = scan_targets("127.0.0.0/29").unwrap();
        assert_eq!(t, (1..=6).map(|h| ip(&format!("127.0.0.{h}"))).collect::<Vec<_>>());
        assert_eq!(scan_targets("10.0.0.8/30").unwrap(), vec![ip("10.0.0.9"), ip("10.0.0.10")]);
    }

    #[test]
    fn a_wider_range_spans_the_third_octet() {
        let t = scan_targets("10.0.4.0/22").unwrap();
        assert_eq!(t.len(), 1022);
        assert_eq!((t[0], t[1021]), (ip("10.0.4.1"), ip("10.0.7.254")));
    }

    #[test]
    fn one_address_and_a_point_to_point_pair() {
        assert_eq!(scan_targets("10.1.2.3").unwrap(), vec![ip("10.1.2.3")]);
        assert_eq!(scan_targets("10.1.2.3/32").unwrap(), vec![ip("10.1.2.3")]);
        assert_eq!(scan_targets("10.1.2.2/31").unwrap(), vec![ip("10.1.2.2"), ip("10.1.2.3")]);
    }

    #[test]
    fn too_large_a_range_is_refused_with_the_way_out() {
        let e = scan_targets("10.0.0.0/16").unwrap_err();
        assert!(e.contains("65534") && e.contains("1024") && e.contains("/22"), "{e}");
        assert!(scan_targets("0.0.0.0/0").is_err());
        assert!(scan_targets("10.0.0.0/21").is_err());
    }

    #[test]
    fn nonsense_is_refused() {
        for bad in ["", "  ", "abc", "192.168", "192.168.1.0/33", "192.168.1.0/x", "300.1.1", "1.2.3.4.5"] {
            assert!(scan_targets(bad).is_err(), "{bad:?}");
        }
    }
}
