//! "Accessible devices": find hosts that answer on the S7 port of a subnet and ask each who it is.
//!
//! Engineering tools find CPUs with PROFINET DCP, a layer-2 broadcast that needs raw sockets.
//! This scan uses plain TCP instead: it works without privileges and across routers, but only
//! finds CPUs whose IP address is already set.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use futures::stream::{self, StreamExt};
use mt_protocol::ProtocolClient;
use mt_s7::{S7Client, S7Config, S7ConnectOptions};
use serde::Serialize;

/// Largest subnet scanned in one go: a /22, 1022 hosts.
const MIN_PREFIX: u8 = 22;
const PROBE_TIMEOUT: Duration = Duration::from_millis(400);
const IDENTIFY_TIMEOUT: Duration = Duration::from_millis(1500);
const PARALLEL_PROBES: usize = 128;

/// Rack/slot pairs tried, in order: S7-1200/1500 (0/0, 0/1), S7-300 (0/2), S7-400 (0/3).
const RACK_SLOTS: [(u8, u8); 4] = [(0, 1), (0, 0), (0, 2), (0, 3)];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FoundDevice {
    pub ip: String,
    pub port: u16,
    /// Rack and slot that accepted a connection, if any did.
    pub rack: Option<u8>,
    pub slot: Option<u8>,
    pub module_type: Option<String>,
    pub order_code: Option<String>,
    pub station_name: Option<String>,
    pub module_name: Option<String>,
    pub firmware: Option<String>,
    /// Why the device answered on the port but could not be identified.
    pub note: Option<String>,
}

/// Parse `192.168.0.0/24`, `192.168.0.10/24` or `192.168.0` (a /24) into the host addresses.
pub fn hosts_of(subnet: &str) -> Result<Vec<Ipv4Addr>, String> {
    let subnet = subnet.trim();
    let (addr, prefix) = match subnet.split_once('/') {
        Some((a, p)) => (a.to_string(), p.parse::<u8>().map_err(|_| format!("bad prefix length in {subnet}"))?),
        None if subnet.split('.').count() == 3 => (format!("{subnet}.0"), 24),
        None => (subnet.to_string(), 32),
    };
    let ip: Ipv4Addr = addr.parse().map_err(|_| format!("{subnet} is not an IPv4 subnet such as 192.168.0.0/24"))?;
    if prefix > 32 {
        return Err(format!("bad prefix length in {subnet}"));
    }
    if prefix < MIN_PREFIX {
        return Err(format!("/{prefix} is too large to scan; use /{MIN_PREFIX} or smaller"));
    }
    if prefix >= 31 {
        return Ok(vec![ip]);
    }
    let mask = u32::MAX << (32 - prefix);
    let network = u32::from(ip) & mask;
    let broadcast = network | !mask;
    Ok((network + 1..broadcast).map(Ipv4Addr::from).collect())
}

/// The /24 of this computer's primary IPv4 address, as a starting suggestion. Opening a UDP
/// socket towards a public address sends nothing; it only makes the OS pick the outgoing interface.
pub fn primary_subnet() -> Option<String> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    match socket.local_addr().ok()? {
        SocketAddr::V4(v4) if !v4.ip().is_unspecified() && !v4.ip().is_loopback() => {
            let [a, b, c, _] = v4.ip().octets();
            Some(format!("{a}.{b}.{c}.0/24"))
        }
        _ => None,
    }
}

async fn port_open(ip: Ipv4Addr, port: u16) -> bool {
    matches!(
        tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect((ip, port))).await,
        Ok(Ok(_))
    )
}

/// Connect with each rack/slot until one is accepted, then read the identity.
pub async fn identify(ip: &str, port: u16) -> FoundDevice {
    let mut found = FoundDevice {
        ip: ip.to_string(),
        port,
        rack: None,
        slot: None,
        module_type: None,
        order_code: None,
        station_name: None,
        module_name: None,
        firmware: None,
        note: None,
    };
    let mut last_error = None;
    for (rack, slot) in RACK_SLOTS {
        let config = S7Config { id: "scan".into(), host: ip.to_string(), port, rack, slot, poll_interval_ms: 1000 };
        let options = S7ConnectOptions { timeout: IDENTIFY_TIMEOUT, ..S7ConnectOptions::default() };
        let client = match S7Client::connect_with_options(config, options).await {
            Ok(c) => c,
            Err(e) => {
                last_error = Some(e.to_string());
                continue;
            }
        };
        found.rack = Some(rack);
        found.slot = Some(slot);
        match client.read_cpu_info().await {
            Ok(cpu) => {
                let non_empty = |s: String| (!s.trim().is_empty()).then(|| s.trim().to_string());
                found.module_type = non_empty(cpu.module_type);
                found.order_code = non_empty(cpu.order_code);
                found.station_name = non_empty(cpu.as_name);
                found.module_name = non_empty(cpu.module_name);
                found.firmware = non_empty(cpu.firmware_version);
            }
            Err(e) => found.note = Some(format!("Connected, but the CPU did not give its identity: {e}")),
        }
        let _ = client.disconnect().await;
        return found;
    }
    found.note = Some(match last_error {
        Some(e) => format!("Port {port} is open but no rack/slot was accepted ({e})"),
        None => format!("Port {port} is open"),
    });
    found
}

pub async fn scan(subnet: &str, port: u16) -> Result<Vec<FoundDevice>, String> {
    let hosts = hosts_of(subnet)?;
    let open: Vec<Ipv4Addr> = stream::iter(hosts)
        .map(|ip| async move { port_open(ip, port).await.then_some(ip) })
        .buffer_unordered(PARALLEL_PROBES)
        .filter_map(|ip| async move { ip })
        .collect()
        .await;
    let mut devices: Vec<FoundDevice> = stream::iter(open)
        .map(|ip| async move { identify(&ip.to_string(), port).await })
        .buffer_unordered(16)
        .collect()
        .await;
    devices.sort_by_key(|d| d.ip.parse::<Ipv4Addr>().map(u32::from).unwrap_or(u32::MAX));
    Ok(devices)
}

#[tauri::command]
pub async fn s7_scan(subnet: String, port: Option<u16>) -> Result<Vec<FoundDevice>, String> {
    scan(&subnet, port.unwrap_or(102)).await
}

#[tauri::command]
pub fn s7_suggest_subnet() -> Option<String> {
    primary_subnet()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subnets_expand_to_their_hosts() {
        let hosts = hosts_of("192.168.0.0/24").unwrap();
        assert_eq!(hosts.len(), 254);
        assert_eq!(hosts[0], Ipv4Addr::new(192, 168, 0, 1));
        assert_eq!(hosts[253], Ipv4Addr::new(192, 168, 0, 254));
        assert_eq!(hosts_of("10.0.5").unwrap().len(), 254);
        assert_eq!(hosts_of("10.0.5.77/30").unwrap(), vec![Ipv4Addr::new(10, 0, 5, 77), Ipv4Addr::new(10, 0, 5, 78)]);
        assert_eq!(hosts_of("10.0.0.9").unwrap(), vec![Ipv4Addr::new(10, 0, 0, 9)]);
        assert_eq!(hosts_of("10.0.0.0/22").unwrap().len(), 1022);
        assert!(hosts_of("10.0.0.0/16").is_err());
        assert!(hosts_of("plc.local/24").is_err());
    }

    #[tokio::test]
    async fn the_simulator_is_found_and_identified() {
        let server = mt_s7::S7Server::start(0, vec![]).await.unwrap();
        let devices = scan("127.0.0.1/32", server.port()).await.unwrap();
        assert_eq!(devices.len(), 1);
        let d = &devices[0];
        assert_eq!((d.rack, d.slot), (Some(0), Some(1)));
        assert_eq!(d.module_type.as_deref(), Some("S7 Simulator"));
        assert_eq!(d.station_name.as_deref(), Some("SIMULATOR"));
        server.stop().await;
    }
}
