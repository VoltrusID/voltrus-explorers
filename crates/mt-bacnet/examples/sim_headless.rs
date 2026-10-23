//! Headless BACnet/IP simulator for validating exports and integrations.
//!
//! ```text
//! cargo run -p mt-bacnet --example sim_headless -- [port] [device-id]
//! ```
//!
//! Serves a fixed point list with constant values (no variance), so readers can assert exact numbers:
//! analogInput 1 = 21.5 (degC), analogValue 2 = 230.25 (V), binaryInput 3 = active,
//! multiStateValue 4 = 3, analogOutput 5 = 50 (commandable).

use mt_bacnet::{SimDevice, SimPointDef, device};

fn point(key: &str, object_type: &str, instance: u32, base: f64, unit: Option<&str>, is_bool: bool, max: Option<f64>) -> SimPointDef {
    SimPointDef {
        key: key.into(),
        label: key.into(),
        object_type: object_type.into(),
        object_instance: instance,
        base,
        variance: 0.0,
        noise: 0.0,
        period: 60,
        min: None,
        max,
        decimals: 2,
        unit: unit.map(str::to_string),
        is_bool,
        writable: false,
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(47808);
    let device_id: u32 = args.next().and_then(|p| p.parse().ok()).unwrap_or(1234);
    let points = vec![
        point("supply_temp", "analogInput", 1, 21.5, Some("degC"), false, None),
        point("voltage", "analogValue", 2, 230.25, Some("V"), false, None),
        point("fan_status", "binaryInput", 3, 1.0, None, true, None),
        point("mode", "multiStateValue", 4, 3.0, None, false, Some(4.0)),
        point("valve", "analogOutput", 5, 50.0, Some("%"), false, None),
    ];
    let dev: SimDevice = device::start(device_id, port, 999, points, Box::new(|_| {}), Box::new(|_, _| {}))
        .expect("start simulator");
    println!("bacnet sim listening on udp {} device {}", dev.port(), dev.device_id());
    while dev.is_running() {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
