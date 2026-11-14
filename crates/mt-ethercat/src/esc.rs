//! The EtherCAT slave controller (ESC) register map (ETG.1000.4 and the ET1100/ET1200 datasheets): names for the
//! register view and the frame log, AL states and AL status codes.

use serde::Serialize;

pub const TYPE: u16 = 0x0000;
pub const REVISION: u16 = 0x0001;
pub const BUILD: u16 = 0x0002;
pub const FMMU_COUNT: u16 = 0x0004;
pub const SM_COUNT: u16 = 0x0005;
pub const RAM_SIZE: u16 = 0x0006;
pub const PORT_DESC: u16 = 0x0007;
pub const FEATURES: u16 = 0x0008;
pub const STATION_ADDR: u16 = 0x0010;
pub const STATION_ALIAS: u16 = 0x0012;
pub const DL_CONTROL: u16 = 0x0100;
pub const DL_STATUS: u16 = 0x0110;
pub const AL_CONTROL: u16 = 0x0120;
pub const AL_STATUS: u16 = 0x0130;
pub const AL_STATUS_CODE: u16 = 0x0134;
pub const PDI_CONTROL: u16 = 0x0140;
pub const ESC_CONFIG: u16 = 0x0141;
pub const RX_ERROR: u16 = 0x0300;
pub const FWD_RX_ERROR: u16 = 0x0308;
pub const PU_ERROR: u16 = 0x030C;
pub const LOST_LINK: u16 = 0x0310;
pub const WATCHDOG_DIVIDER: u16 = 0x0400;
pub const WATCHDOG_PDI: u16 = 0x0410;
pub const WATCHDOG_PD: u16 = 0x0420;
pub const WATCHDOG_STATUS: u16 = 0x0440;
pub const WATCHDOG_COUNTER_PD: u16 = 0x0442;
pub const SII_CONFIG: u16 = 0x0500;
pub const SII_CONTROL: u16 = 0x0502;
pub const SII_ADDRESS: u16 = 0x0504;
pub const SII_DATA: u16 = 0x0508;
pub const FMMU_BASE: u16 = 0x0600;
pub const SM_BASE: u16 = 0x0800;
pub const DC_RECV_TIME: u16 = 0x0900;
pub const DC_SYSTEM_TIME: u16 = 0x0910;
pub const DC_RECV_TIME_PU: u16 = 0x0918;
pub const DC_SYSTEM_OFFSET: u16 = 0x0920;
pub const DC_SYSTEM_DELAY: u16 = 0x0928;
pub const DC_SYSTEM_DIFF: u16 = 0x092C;
pub const DC_CYCLIC_UNIT: u16 = 0x0980;
pub const DC_ACTIVATION: u16 = 0x0981;
pub const DC_SYNC0_CYCLE: u16 = 0x09A0;
/// Start of process RAM: mailboxes and process data live here.
pub const PROCESS_RAM: u16 = 0x1000;

/// FMMU `n` registers (16 bytes each).
pub const fn fmmu(n: u16) -> u16 {
    FMMU_BASE + n * 16
}

/// Sync manager `n` registers (8 bytes each).
pub const fn sm(n: u16) -> u16 {
    SM_BASE + n * 8
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AlState {
    Init,
    PreOp,
    Boot,
    SafeOp,
    Op,
    /// A value that is none of the states above (read from a device in the middle of a change, or broken).
    Unknown,
}

impl AlState {
    pub fn from_code(v: u8) -> AlState {
        match v & 0x0F {
            1 => AlState::Init,
            2 => AlState::PreOp,
            3 => AlState::Boot,
            4 => AlState::SafeOp,
            8 => AlState::Op,
            _ => AlState::Unknown,
        }
    }

    pub fn code(self) -> u8 {
        match self {
            AlState::Init => 1,
            AlState::PreOp => 2,
            AlState::Boot => 3,
            AlState::SafeOp => 4,
            AlState::Op => 8,
            AlState::Unknown => 0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AlState::Init => "INIT",
            AlState::PreOp => "PRE-OP",
            AlState::Boot => "BOOT",
            AlState::SafeOp => "SAFE-OP",
            AlState::Op => "OP",
            AlState::Unknown => "?",
        }
    }
}

/// AL status codes (ETG.1000.6 table 11 and ETG.1020).
pub fn al_status_text(code: u16) -> &'static str {
    match code {
        0x0000 => "No error",
        0x0001 => "Unspecified error",
        0x0002 => "No memory",
        0x0003 => "Invalid device setup",
        0x0011 => "Invalid requested state change",
        0x0012 => "Unknown requested state",
        0x0013 => "Bootstrap not supported",
        0x0014 => "No valid firmware",
        0x0015 => "Invalid mailbox configuration (BOOT)",
        0x0016 => "Invalid mailbox configuration (PRE-OP)",
        0x0017 => "Invalid sync manager configuration",
        0x0018 => "No valid inputs available",
        0x0019 => "No valid outputs",
        0x001A => "Synchronization error",
        0x001B => "Sync manager watchdog",
        0x001C => "Invalid sync manager types",
        0x001D => "Invalid output configuration",
        0x001E => "Invalid input configuration",
        0x001F => "Invalid watchdog configuration",
        0x0020 => "Device needs cold start",
        0x0021 => "Device needs INIT",
        0x0022 => "Device needs PRE-OP",
        0x0023 => "Device needs SAFE-OP",
        0x0024 => "Invalid input mapping",
        0x0025 => "Invalid output mapping",
        0x0026 => "Inconsistent settings",
        0x0027 => "Free-run not supported",
        0x0028 => "Sync mode not supported",
        0x0029 => "Free-run needs 3-buffer mode",
        0x002A => "Background watchdog",
        0x002B => "No valid inputs and outputs",
        0x002C => "Fatal sync error",
        0x002D => "No sync error",
        0x0030 => "Invalid DC SYNC configuration",
        0x0031 => "Invalid DC latch configuration",
        0x0032 => "PLL error",
        0x0033 => "DC sync IO error",
        0x0034 => "DC sync timeout error",
        0x0035 => "DC invalid sync cycle time",
        0x0036 => "DC Sync0 cycle time",
        0x0037 => "DC Sync1 cycle time",
        0x0041 => "MBX_AOE",
        0x0042 => "MBX_EOE",
        0x0043 => "MBX_COE",
        0x0044 => "MBX_FOE",
        0x0045 => "MBX_SOE",
        0x004F => "MBX_VOE",
        0x0050 => "EEPROM no access",
        0x0051 => "EEPROM error",
        0x0060 => "Device restarted locally",
        _ if code >= 0x8000 => "Vendor specific",
        _ => "Reserved",
    }
}

/// A register in the register view: where it is, how long, and what it is called.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct RegisterInfo {
    pub addr: u16,
    pub len: u16,
    pub name: &'static str,
}

/// The registers the register view shows, in address order.
pub const REGISTERS: &[RegisterInfo] = &[
    RegisterInfo { addr: TYPE, len: 1, name: "Type" },
    RegisterInfo { addr: REVISION, len: 1, name: "Revision" },
    RegisterInfo { addr: BUILD, len: 2, name: "Build" },
    RegisterInfo { addr: FMMU_COUNT, len: 1, name: "FMMUs supported" },
    RegisterInfo { addr: SM_COUNT, len: 1, name: "SyncManagers supported" },
    RegisterInfo { addr: RAM_SIZE, len: 1, name: "RAM Size (KB)" },
    RegisterInfo { addr: PORT_DESC, len: 1, name: "Port Descriptor" },
    RegisterInfo { addr: FEATURES, len: 2, name: "ESC Features" },
    RegisterInfo { addr: STATION_ADDR, len: 2, name: "Configured Station Address" },
    RegisterInfo { addr: STATION_ALIAS, len: 2, name: "Configured Station Alias" },
    RegisterInfo { addr: DL_CONTROL, len: 4, name: "DL Control" },
    RegisterInfo { addr: DL_STATUS, len: 2, name: "DL Status" },
    RegisterInfo { addr: AL_CONTROL, len: 2, name: "AL Control" },
    RegisterInfo { addr: AL_STATUS, len: 2, name: "AL Status" },
    RegisterInfo { addr: AL_STATUS_CODE, len: 2, name: "AL Status Code" },
    RegisterInfo { addr: PDI_CONTROL, len: 1, name: "PDI Control" },
    RegisterInfo { addr: ESC_CONFIG, len: 1, name: "ESC Configuration" },
    RegisterInfo { addr: RX_ERROR, len: 8, name: "RX Error Counters" },
    RegisterInfo { addr: FWD_RX_ERROR, len: 4, name: "Forwarded RX Error Counters" },
    RegisterInfo { addr: PU_ERROR, len: 1, name: "ECAT Processing Unit Error Counter" },
    RegisterInfo { addr: 0x030D, len: 1, name: "PDI Error Counter" },
    RegisterInfo { addr: LOST_LINK, len: 4, name: "Lost Link Counters" },
    RegisterInfo { addr: WATCHDOG_DIVIDER, len: 2, name: "Watchdog Divider" },
    RegisterInfo { addr: WATCHDOG_PDI, len: 2, name: "Watchdog Time PDI" },
    RegisterInfo { addr: WATCHDOG_PD, len: 2, name: "Watchdog Time Process Data" },
    RegisterInfo { addr: WATCHDOG_STATUS, len: 2, name: "Watchdog Status Process Data" },
    RegisterInfo { addr: WATCHDOG_COUNTER_PD, len: 1, name: "Watchdog Counter Process Data" },
    RegisterInfo { addr: 0x0443, len: 1, name: "Watchdog Counter PDI" },
    RegisterInfo { addr: SII_CONFIG, len: 1, name: "SII EEPROM Configuration" },
    RegisterInfo { addr: 0x0501, len: 1, name: "SII EEPROM PDI Access State" },
    RegisterInfo { addr: SII_CONTROL, len: 2, name: "SII EEPROM Control/Status" },
    RegisterInfo { addr: SII_ADDRESS, len: 4, name: "SII EEPROM Address" },
    RegisterInfo { addr: SII_DATA, len: 8, name: "SII EEPROM Data" },
    RegisterInfo { addr: DC_RECV_TIME, len: 16, name: "DC Receive Time Ports 0-3" },
    RegisterInfo { addr: DC_SYSTEM_TIME, len: 8, name: "DC System Time" },
    RegisterInfo { addr: DC_RECV_TIME_PU, len: 8, name: "DC Receive Time ECAT Processing Unit" },
    RegisterInfo { addr: DC_SYSTEM_OFFSET, len: 8, name: "DC System Time Offset" },
    RegisterInfo { addr: DC_SYSTEM_DELAY, len: 4, name: "DC System Time Delay" },
    RegisterInfo { addr: DC_SYSTEM_DIFF, len: 4, name: "DC System Time Difference" },
    RegisterInfo { addr: DC_CYCLIC_UNIT, len: 1, name: "DC Cyclic Unit Control" },
    RegisterInfo { addr: DC_ACTIVATION, len: 1, name: "DC Activation" },
    RegisterInfo { addr: DC_SYNC0_CYCLE, len: 4, name: "DC SYNC0 Cycle Time" },
];

/// The name of the register at `addr` (FMMU and SM blocks included), for the frame log.
pub fn register_name(addr: u16) -> Option<String> {
    if let Some(r) = REGISTERS.iter().find(|r| r.addr == addr) {
        return Some(r.name.to_string());
    }
    if (FMMU_BASE..FMMU_BASE + 16 * 16).contains(&addr) {
        return Some(format!("FMMU{}", (addr - FMMU_BASE) / 16));
    }
    if (SM_BASE..SM_BASE + 16 * 8).contains(&addr) {
        let n = (addr - SM_BASE) / 8;
        return Some(match (addr - SM_BASE) % 8 {
            5 => format!("SM{n} Status"),
            6 => format!("SM{n} Activate"),
            _ => format!("SM{n}"),
        });
    }
    if addr >= PROCESS_RAM {
        return Some("Process RAM".to_string());
    }
    None
}

/// DL status (0x0110), per port: physical link, loop closed, communication established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortStatus {
    pub link: bool,
    pub loop_closed: bool,
    pub communication: bool,
}

pub fn dl_status_ports(dl: u16) -> [PortStatus; 4] {
    std::array::from_fn(|p| PortStatus {
        link: dl & (1 << (4 + p)) != 0,
        loop_closed: dl & (1 << (8 + 2 * p)) != 0,
        communication: dl & (1 << (9 + 2 * p)) != 0,
    })
}

/// Port descriptor (0x0007): what each of the four ports is.
pub fn port_kind(desc: u8, port: usize) -> &'static str {
    match (desc >> (2 * port)) & 0b11 {
        0 => "not implemented",
        1 => "not configured",
        2 => "EBUS",
        _ => "MII/RMII",
    }
}

/// The ESC chip by type register, for the device panel.
pub fn esc_type_name(t: u8) -> &'static str {
    match t {
        0x01 => "ESC10/ESC20",
        0x02 => "ESC20",
        0x04 => "IP core",
        0x11 => "ET1100",
        0x12 => "ET1200",
        0x14 => "ET1150",
        0x90 => "Microchip LAN9252",
        0x98 => "Microchip LAN9253",
        0xC0 => "Innovasic fido",
        0xC8 => "Hilscher netX",
        0xD0 => "Renesas",
        0xE0 => "TI Sitara (PRU)",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_ports() {
        assert_eq!(register_name(0x0130).as_deref(), Some("AL Status"));
        assert_eq!(register_name(0x0805).as_deref(), Some("SM0 Status"));
        assert_eq!(register_name(0x0610).as_deref(), Some("FMMU1"));
        // Port 0 and 1 linked; port 0 communicating, port 1 open; ports 2 and 3 closed.
        let ports = dl_status_ports(0b0101_0010_0011_0000 | 0x0200 | 0x0800);
        assert!(ports[0].link && ports[0].communication);
        assert!(ports[1].link && ports[1].communication);
        assert!(!ports[2].link && ports[2].loop_closed);
        assert_eq!(AlState::from_code(0x14), AlState::SafeOp);
        assert_eq!(al_status_text(0x001D), "Invalid output configuration");
        assert_eq!(port_kind(0x0F, 0), "MII/RMII");
        assert_eq!(port_kind(0b1010, 1), "EBUS");
    }
}
