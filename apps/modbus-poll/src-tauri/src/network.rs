use crate::state::ScanResult;

#[tauri::command]
pub async fn scan_network(subnet: String, port: u16) -> Result<Vec<ScanResult>, String> {
    let parts: Vec<&str> = subnet.split('.').collect();
    if parts.len() < 3 {
        return Err("Invalid subnet, expected e.g. 192.168.1".into());
    }
    let base = format!("{}.{}.{}", parts[0], parts[1], parts[2]);

    let mut handles = Vec::new();
    for i in 1..=254u8 {
        let addr = format!("{}.{}", base, i);
        let p = port;
        handles.push(tokio::spawn(async move {
            let target = format!("{}:{}", addr, p);
            let open = tokio::time::timeout(
                std::time::Duration::from_millis(500),
                tokio::net::TcpStream::connect(&target),
            )
            .await
            .is_ok();
            ScanResult {
                ip: addr,
                port_open: open,
                hostname: None,
            }
        }));
    }

    let mut results = Vec::new();
    for h in handles {
        if let Ok(r) = h.await {
            results.push(r);
        }
    }

    // Parse ARP table for hostnames
    let arp_output = std::process::Command::new("arp")
        .arg("-a")
        .output()
        .ok();
    let arp_map: std::collections::HashMap<String, String> = arp_output
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| {
            s.lines()
                .filter_map(|line| {
                    let ip_start = line.find('(')?;
                    let ip_end = line.find(')')?;
                    let ip = line[ip_start + 1..ip_end].to_string();
                    let hostname = line[..ip_start].trim().to_string();
                    let hostname = if hostname == "?" { "".into() } else { hostname };
                    Some((ip, hostname))
                })
                .collect()
        })
        .unwrap_or_default();

    for r in &mut results {
        if let Some(host) = arp_map.get(&r.ip) {
            r.hostname = if host.is_empty() { None } else { Some(host.clone()) };
        }
    }

    results.sort_by(|a, b| b.port_open.cmp(&a.port_open));
    Ok(results)
}

#[tauri::command]
#[cfg(target_os = "macos")]
pub async fn auto_detect_rtu(
    port_name: String,
    unit_id: u8,
) -> Result<Option<serde_json::Value>, String> {
    use tokio_modbus::slave::Slave;

    let baud_rates = [9600u32, 19200, 38400, 115200, 4800, 2400, 1200, 57600];
    let parities = [
        (serialport::Parity::None, "none"),
        (serialport::Parity::Even, "even"),
        (serialport::Parity::Odd, "odd"),
    ];
    let stop_bits_opts = [(1u8, serialport::StopBits::One), (2u8, serialport::StopBits::Two)];

    for &baud in &baud_rates {
        for &(parity, parity_name) in &parities {
            for &(stop, stop_bits) in &stop_bits_opts {
                let builder = tokio_serial::new(&port_name, baud)
                    .parity(parity)
                    .data_bits(serialport::DataBits::Eight)
                    .stop_bits(stop_bits)
                    .timeout(std::time::Duration::from_secs(1));

                let stream = match tokio_serial::SerialStream::open(&builder) {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                let ctx = tokio_modbus::client::rtu::attach_slave(stream, Slave(unit_id));

                let result = tokio::time::timeout(
                    std::time::Duration::from_millis(600),
                    async {
                        let mut ctx = ctx;
                        tokio_modbus::client::Reader::read_holding_registers(&mut ctx, 0, 1).await
                    },
                ).await;

                match result {
                    Ok(Ok(Ok(_))) => {
                        return Ok(Some(serde_json::json!({
                            "baud_rate": baud,
                            "parity": parity_name,
                            "stop_bits": stop,
                            "data_bits": 8,
                        })));
                    }
                    _ => {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        continue;
                    }
                }
            }
        }
    }

    Ok(None)
}
