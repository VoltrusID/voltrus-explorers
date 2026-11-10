//! What an identifier means under the higher-layer protocols seen on industrial and vehicle buses:
//! SAE J1939 (29-bit, trucks, gensets, off-highway) and CANopen (11-bit, CiA 301 predefined
//! connection set). Labels only; no transport-protocol reassembly.

use serde::Serialize;

use crate::frame::CanFrame;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct J1939Id {
    pub priority: u8,
    pub pgn: u32,
    pub source: u8,
    /// Destination for PDU1 (peer-to-peer) PGNs; `None` for broadcast PDU2.
    pub destination: Option<u8>,
}

/// Splits a 29-bit identifier into J1939 fields.
pub fn j1939(id: u32) -> J1939Id {
    let priority = ((id >> 26) & 0x7) as u8;
    let dp = (id >> 24) & 0x3;
    let pf = (id >> 16) & 0xFF;
    let ps = (id >> 8) & 0xFF;
    let source = (id & 0xFF) as u8;
    if pf < 240 {
        J1939Id { priority, pgn: (dp << 16) | (pf << 8), source, destination: Some(ps as u8) }
    } else {
        J1939Id { priority, pgn: (dp << 16) | (pf << 8) | ps, source, destination: None }
    }
}

/// Names of the J1939 parameter groups most often seen on a bus.
pub fn j1939_pgn_name(pgn: u32) -> Option<&'static str> {
    Some(match pgn {
        59904 => "Request",
        59392 => "Acknowledgement",
        60160 => "TP.DT (transport data)",
        60416 => "TP.CM (transport connection)",
        60928 => "Address Claimed",
        61443 => "EEC2 (electronic engine controller 2)",
        61444 => "EEC1 (electronic engine controller 1)",
        65226 => "DM1 (active diagnostic trouble codes)",
        65227 => "DM2 (previously active DTCs)",
        65248 => "VD (vehicle distance)",
        65253 => "HOURS (engine hours)",
        65262 => "ET1 (engine temperature 1)",
        65263 => "EFL/P1 (engine fluid level/pressure 1)",
        65265 => "CCVS1 (cruise control/vehicle speed 1)",
        65266 => "LFE1 (fuel economy)",
        65269 => "AMB (ambient conditions)",
        65270 => "IC1 (inlet/exhaust conditions 1)",
        65271 => "VEP1 (vehicle electrical power 1)",
        65276 => "DD1 (dash display 1)",
        _ => return None,
    })
}

/// CANopen function of an 11-bit COB-ID in the predefined connection set, with the node id.
pub fn canopen(id: u32) -> Option<(String, u8)> {
    if id > 0x7FF {
        return None;
    }
    if id == 0 {
        return Some(("NMT".into(), 0));
    }
    if id == 0x80 {
        return Some(("SYNC".into(), 0));
    }
    if id == 0x100 {
        return Some(("TIME".into(), 0));
    }
    let node = (id & 0x7F) as u8;
    if node == 0 {
        return None;
    }
    let f = match id & 0x780 {
        0x080 => "EMCY",
        0x180 => "TPDO1",
        0x200 => "RPDO1",
        0x280 => "TPDO2",
        0x300 => "RPDO2",
        0x380 => "TPDO3",
        0x400 => "RPDO3",
        0x480 => "TPDO4",
        0x500 => "RPDO4",
        0x580 => "SDO tx",
        0x600 => "SDO rx",
        0x700 => "Heartbeat",
        _ => return None,
    };
    Some((f.into(), node))
}

/// Which higher layer to label frames with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    #[default]
    None,
    J1939,
    Canopen,
    Obd2,
}

/// A short label for a frame under `layer`, or `None` when it does not apply.
pub fn describe(frame: &CanFrame, layer: Layer) -> Option<String> {
    match layer {
        Layer::None => None,
        Layer::J1939 => {
            if !frame.extended {
                return None;
            }
            let j = j1939(frame.id);
            let name = j1939_pgn_name(j.pgn).map(|n| format!(" {n}")).unwrap_or_default();
            Some(match j.destination {
                Some(d) => format!("PGN {}{name} SA {:02X} DA {:02X} P{}", j.pgn, j.source, d, j.priority),
                None => format!("PGN {}{name} SA {:02X} P{}", j.pgn, j.source, j.priority),
            })
        }
        Layer::Canopen => {
            if frame.extended {
                return None;
            }
            let (f, node) = canopen(frame.id)?;
            Some(match f.as_str() {
                "NMT" => {
                    let cmd = match frame.data.first() {
                        Some(1) => "start",
                        Some(2) => "stop",
                        Some(0x80) => "pre-operational",
                        Some(0x81) => "reset node",
                        Some(0x82) => "reset communication",
                        _ => "?",
                    };
                    let target = frame.data.get(1).map(|n| if *n == 0 { "all nodes".to_string() } else { format!("node {n}") });
                    format!("NMT {cmd} {}", target.unwrap_or_default()).trim_end().to_string()
                }
                "SYNC" | "TIME" => f,
                "Heartbeat" => {
                    let state = match frame.data.first() {
                        Some(0) => "boot-up",
                        Some(4) => "stopped",
                        Some(5) => "operational",
                        Some(0x7F) => "pre-operational",
                        _ => "?",
                    };
                    format!("Heartbeat node {node}: {state}")
                }
                "SDO tx" | "SDO rx" if frame.data.len() >= 4 => {
                    let index = u16::from_le_bytes([frame.data[1], frame.data[2]]);
                    format!("{f} node {node} {index:04X}sub{:02X}", frame.data[3])
                }
                _ => format!("{f} node {node}"),
            })
        }
        Layer::Obd2 => {
            if frame.extended {
                return None;
            }
            match frame.id {
                0x7DF => Some(format!("OBD-II request (all ECUs){}", obd_pid(&frame.data, false))),
                0x7E0..=0x7E7 => Some(format!("OBD-II request to ECU {}{}", frame.id - 0x7E0, obd_pid(&frame.data, false))),
                0x7E8..=0x7EF => {
                    let value = obd_value(&frame.data).map(|(_, v, u)| format!(" = {} {u}", (v * 10.0).round() / 10.0)).unwrap_or_default();
                    Some(format!("OBD-II response from ECU {}{}{value}", frame.id - 0x7E8, obd_pid(&frame.data, true)))
                }
                _ => None,
            }
        }
    }
}

fn obd_pid(data: &[u8], response: bool) -> String {
    if data.len() < 3 {
        return String::new();
    }
    let mode = if response { data[1].wrapping_sub(0x40) } else { data[1] };
    let pid = data[2];
    let name = match (mode, pid) {
        (1, 0x00) => "supported PIDs 01-20",
        (1, 0x04) => "engine load",
        (1, 0x05) => "coolant temperature",
        (1, 0x0C) => "engine speed",
        (1, 0x0D) => "vehicle speed",
        (1, 0x11) => "throttle position",
        (1, 0x2F) => "fuel level",
        _ => "",
    };
    if name.is_empty() { format!(": mode {mode:02X} PID {pid:02X}") } else { format!(": {name}") }
}

/// Physical value of an OBD-II mode 01 response, for the PIDs the simulator answers.
pub fn obd_value(data: &[u8]) -> Option<(&'static str, f64, &'static str)> {
    if data.len() < 4 || data[1] != 0x41 {
        return None;
    }
    let a = data[3] as f64;
    let b = data.get(4).copied().unwrap_or(0) as f64;
    Some(match data[2] {
        0x04 => ("engine load", a * 100.0 / 255.0, "%"),
        0x05 => ("coolant temperature", a - 40.0, "degC"),
        0x0C => ("engine speed", (256.0 * a + b) / 4.0, "rpm"),
        0x0D => ("vehicle speed", a, "km/h"),
        0x11 => ("throttle position", a * 100.0 / 255.0, "%"),
        0x2F => ("fuel level", a * 100.0 / 255.0, "%"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn j1939_fields() {
        // EEC1 from the engine (SA 0x00), priority 3.
        let j = j1939(0x0CF0_0400);
        assert_eq!(j, J1939Id { priority: 3, pgn: 61444, source: 0, destination: None });
        // Request (PDU1) from 0xF9 to 0x00.
        let r = j1939(0x18EA_00F9);
        assert_eq!(r.pgn, 59904);
        assert_eq!(r.destination, Some(0));
        let f = CanFrame::new(0x18FE_EE00, &[0; 8]).unwrap();
        assert_eq!(describe(&f, Layer::J1939).unwrap(), "PGN 65262 ET1 (engine temperature 1) SA 00 P6");
        assert_eq!(describe(&CanFrame::new(0x100, &[]).unwrap(), Layer::J1939), None);
    }

    #[test]
    fn canopen_functions() {
        assert_eq!(canopen(0x181), Some(("TPDO1".into(), 1)));
        assert_eq!(canopen(0x705), Some(("Heartbeat".into(), 5)));
        assert_eq!(canopen(0x780), None);
        let hb = CanFrame::new(0x705, &[5]).unwrap();
        assert_eq!(describe(&hb, Layer::Canopen).unwrap(), "Heartbeat node 5: operational");
        let nmt = CanFrame::new(0, &[1, 0]).unwrap();
        assert_eq!(describe(&nmt, Layer::Canopen).unwrap(), "NMT start all nodes");
        let sdo = CanFrame::new(0x602, &[0x40, 0x18, 0x10, 0x01, 0, 0, 0, 0]).unwrap();
        assert_eq!(describe(&sdo, Layer::Canopen).unwrap(), "SDO rx node 2 1018sub01");
    }

    #[test]
    fn obd() {
        let req = CanFrame::new(0x7DF, &[2, 1, 0x0C, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(describe(&req, Layer::Obd2).unwrap(), "OBD-II request (all ECUs): engine speed");
        let rsp = [4, 0x41, 0x0C, 0x1A, 0xF8, 0, 0, 0];
        assert_eq!(
            describe(&CanFrame::new(0x7E8, &rsp).unwrap(), Layer::Obd2).unwrap(),
            "OBD-II response from ECU 0: engine speed = 1726 rpm"
        );
        assert_eq!(obd_value(&rsp), Some(("engine speed", 1726.0, "rpm")));
    }
}
