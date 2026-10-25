use serde::{Deserialize, Serialize};

/// The standard BACnet/IP UDP port (0xBAC0).
pub const BACNET_PORT: u16 = 47808;

/// Read a BACnet/IP device's input, output and value objects, poll them or
/// subscribe to their changes, and command them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacnetClientSpec {
    /// The device's IP address or host name.
    pub host: String,
    /// The device's UDP port.
    pub port: u16,
    /// Device object instance; `None` asks the device with a directed Who-Is.
    pub device_instance: Option<u32>,
    /// Local address the client binds: `0.0.0.0` for every interface, or one
    /// interface's address.
    pub bind: String,
    /// Local UDP port; 0 takes any free port.
    pub local_port: u16,
    pub poll_ms: u64,
    /// Subscribe to change-of-value notifications where the device supports
    /// them; objects it refuses are polled.
    pub cov: bool,
    /// Command priority of writes (1-16, not 6), and the slot a relinquish clears.
    pub priority: u8,
}

impl Default for BacnetClientSpec {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: BACNET_PORT,
            device_instance: None,
            bind: "0.0.0.0".into(),
            local_port: 0,
            poll_ms: 1000,
            cov: true,
            priority: 8,
        }
    }
}

impl BacnetClientSpec {
    /// Priority 6 is reserved for minimum on/off time (ASHRAE 135 clause 19.2.1.1).
    pub fn valid_priority(priority: u8) -> bool {
        (1..=16).contains(&priority) && priority != 6
    }
}

/// Play a BACnet/IP device on this computer: a plant's objects with simulated
/// values that clients read, subscribe to and command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacnetDeviceSpec {
    /// UDP port the device answers on, on every interface.
    pub port: u16,
    pub device_instance: u32,
    /// Built-in plant template id.
    pub template: String,
}

impl Default for BacnetDeviceSpec {
    fn default() -> Self {
        Self {
            port: BACNET_PORT,
            device_instance: 1001,
            template: "ahu".into(),
        }
    }
}

/// Who-Is: find the BACnet devices that answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacnetDiscoverSpec {
    /// Local address to send from.
    pub bind: String,
    /// Broadcast the Who-Is to this BACnet port on every interface.
    pub broadcast_port: Option<u16>,
    /// Addresses (`host:port`) that also get a Who-Is of their own: devices
    /// on other ports, behind routers, or on this computer.
    pub directed: Vec<String>,
    /// How long to collect I-Am answers.
    pub wait_ms: u64,
}

/// A device that answered a Who-Is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacnetDeviceInfo {
    pub instance: u32,
    pub address: String,
    pub port: u16,
    pub vendor: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_six_is_reserved() {
        assert!(BacnetClientSpec::valid_priority(1));
        assert!(BacnetClientSpec::valid_priority(16));
        assert!(!BacnetClientSpec::valid_priority(0));
        assert!(!BacnetClientSpec::valid_priority(6));
        assert!(!BacnetClientSpec::valid_priority(17));
    }
}
