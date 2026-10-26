//! Bluetooth SIG assigned numbers: the names of GATT services and
//! characteristics, and of the companies behind manufacturer data.

/// The tail every 16-bit UUID shares once expanded to 128 bits (the
/// Bluetooth Base UUID).
const BASE_SUFFIX: &str = "-0000-1000-8000-00805f9b34fb";

/// The 128-bit form of a 16-bit assigned number, as the protocol crate
/// writes UUIDs (lowercase, hyphenated).
pub fn uuid16(short: u16) -> String {
    format!("0000{short:04x}{BASE_SUFFIX}")
}

/// The 16-bit assigned number inside a Bluetooth Base UUID.
pub fn short_of(uuid: &str) -> Option<u16> {
    let uuid = uuid.trim();
    if uuid.len() == 4 {
        return u16::from_str_radix(uuid, 16).ok();
    }
    if uuid.len() == 36
        && uuid.is_ascii()
        && uuid.starts_with("0000")
        && uuid[8..].eq_ignore_ascii_case(BASE_SUFFIX)
    {
        return u16::from_str_radix(&uuid[4..8], 16).ok();
    }
    None
}

/// `2A37` for assigned numbers, the first eight digits of any other UUID.
pub fn short(uuid: &str) -> String {
    match short_of(uuid) {
        Some(short) => format!("{short:04X}"),
        None => uuid
            .chars()
            .take(8)
            .collect::<String>()
            .to_ascii_uppercase(),
    }
}

/// The assigned (or well-known vendor) name of a service or characteristic.
pub fn name(uuid: &str) -> Option<&'static str> {
    match short_of(uuid) {
        Some(short) => assigned(short),
        None => vendor(uuid),
    }
}

fn assigned(short: u16) -> Option<&'static str> {
    Some(match short {
        // Services
        0x1800 => "Generic Access",
        0x1801 => "Generic Attribute",
        0x1802 => "Immediate Alert",
        0x1803 => "Link Loss",
        0x1804 => "Tx Power",
        0x1805 => "Current Time",
        0x1808 => "Glucose",
        0x1809 => "Health Thermometer",
        0x180A => "Device Information",
        0x180D => "Heart Rate",
        0x180F => "Battery",
        0x1810 => "Blood Pressure",
        0x1812 => "Human Interface Device",
        0x1814 => "Running Speed and Cadence",
        0x1815 => "Automation IO",
        0x1816 => "Cycling Speed and Cadence",
        0x1818 => "Cycling Power",
        0x1819 => "Location and Navigation",
        0x181A => "Environmental Sensing",
        0x181B => "Body Composition",
        0x181D => "Weight Scale",
        0x181F => "Continuous Glucose Monitoring",
        0x1822 => "Pulse Oximeter",
        0x1826 => "Fitness Machine",
        0x1827 => "Mesh Provisioning",
        0x1828 => "Mesh Proxy",
        0xFE59 => "Nordic DFU",
        0xFEAA => "Eddystone",
        // Characteristics
        0x2A00 => "Device Name",
        0x2A01 => "Appearance",
        0x2A02 => "Peripheral Privacy Flag",
        0x2A04 => "Preferred Connection Parameters",
        0x2A05 => "Service Changed",
        0x2A06 => "Alert Level",
        0x2A07 => "Tx Power Level",
        0x2A08 => "Date Time",
        0x2A19 => "Battery Level",
        0x2A1C => "Temperature Measurement",
        0x2A1D => "Temperature Type",
        0x2A1E => "Intermediate Temperature",
        0x2A23 => "System ID",
        0x2A24 => "Model Number",
        0x2A25 => "Serial Number",
        0x2A26 => "Firmware Revision",
        0x2A27 => "Hardware Revision",
        0x2A28 => "Software Revision",
        0x2A29 => "Manufacturer Name",
        0x2A2B => "Current Time",
        0x2A35 => "Blood Pressure Measurement",
        0x2A37 => "Heart Rate Measurement",
        0x2A38 => "Body Sensor Location",
        0x2A39 => "Heart Rate Control Point",
        0x2A4D => "Report",
        0x2A50 => "PnP ID",
        0x2A53 => "RSC Measurement",
        0x2A5B => "CSC Measurement",
        0x2A5D => "Sensor Location",
        0x2A63 => "Cycling Power Measurement",
        0x2A6C => "Elevation",
        0x2A6D => "Pressure",
        0x2A6E => "Temperature",
        0x2A6F => "Humidity",
        0x2A76 => "UV Index",
        0x2A77 => "Irradiance",
        0x2A98 => "Weight",
        0x2A9D => "Weight Measurement",
        0x2AA6 => "Central Address Resolution",
        0x2B29 => "Client Supported Features",
        0x2B2A => "Database Hash",
        0x2B3A => "Server Supported Features",
        _ => return None,
    })
}

/// Vendor UUIDs that many development boards expose.
fn vendor(uuid: &str) -> Option<&'static str> {
    const VENDOR: [(&str, &str); 5] = [
        ("6e400001-b5a3-f393-e0a9-e50e24dcca9e", "Nordic UART"),
        ("6e400002-b5a3-f393-e0a9-e50e24dcca9e", "UART RX"),
        ("6e400003-b5a3-f393-e0a9-e50e24dcca9e", "UART TX"),
        // The control service of the simulated environment sensor.
        ("7d2e0001-6c1a-4b7e-9e4f-3a5c8d1b0e21", "Voltrus Control"),
        ("7d2e0002-6c1a-4b7e-9e4f-3a5c8d1b0e21", "Setpoint"),
    ];
    VENDOR
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(uuid.trim()))
        .map(|(_, name)| *name)
}

/// The company behind a manufacturer data identifier.
pub fn company(id: u16) -> Option<&'static str> {
    Some(match id {
        0x0006 => "Microsoft",
        0x000F => "Broadcom",
        0x004C => "Apple",
        0x0059 => "Nordic Semiconductor",
        0x0075 => "Samsung",
        0x0087 => "Garmin",
        0x00E0 => "Google",
        0x00D2 => "Dialog Semiconductor",
        0x0131 => "Cypress Semiconductor",
        0x0157 => "Huami",
        0x01DA => "Logitech",
        0x02E5 => "Espressif",
        0x038F => "Xiaomi",
        0x0499 => "Ruuvi Innovations",
        0x006B => "Polar Electro",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_uuids_expand_and_contract() {
        let heart_rate = uuid16(0x2A37);
        assert_eq!(heart_rate, "00002a37-0000-1000-8000-00805f9b34fb");
        assert_eq!(short_of(&heart_rate), Some(0x2A37));
        assert_eq!(short_of(&heart_rate.to_ascii_uppercase()), Some(0x2A37));
        assert_eq!(short_of("2a19"), Some(0x2A19));
        assert_eq!(short(&heart_rate), "2A37");
        // Not on the Base UUID: no short form, and the first digits instead.
        let uart = "6e400003-b5a3-f393-e0a9-e50e24dcca9e";
        assert_eq!(short_of(uart), None);
        assert_eq!(short(uart), "6E400003");
        assert_eq!(short_of("00002a37-0000-1000-8000-00805f9b34fc"), None);
        assert_eq!(short_of("zz"), None);
    }

    #[test]
    fn names_of_services_characteristics_and_companies() {
        assert_eq!(name(&uuid16(0x180D)), Some("Heart Rate"));
        assert_eq!(name(&uuid16(0x2A19)), Some("Battery Level"));
        assert_eq!(
            name("6E400002-B5A3-F393-E0A9-E50E24DCCA9E"),
            Some("UART RX")
        );
        assert_eq!(name(&uuid16(0xBEEF)), None);
        assert_eq!(company(0x004C), Some("Apple"));
        assert_eq!(company(0xFFFF), None);
    }
}
