#![allow(dead_code)]
#![allow(clippy::ptr_arg)]

use anyhow::Result;

/// DNP3 CRC-16 (IEEE 1815-2012 Annex C): polynomial
/// x16+x13+x12+x11+x10+x8+x6+x5+x2+1 (0x3D65, reflected 0xA6BC),
/// initial value 0, result complemented, transmitted LSB first.
const CRC_TABLE: [u16; 256] = {
    let mut table = [0u16; 256];
    let mut i = 0usize;
    while i < 256 {
        let mut crc = i as u16;
        let mut j = 0;
        while j < 8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xA6BC } else { crc >> 1 };
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

/// DNP3 data-link CRC-16 over `data`. The caller transmits the result
/// LSB first (`crc.to_le_bytes()`).
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0x0000;
    for &byte in data {
        crc = (crc >> 8) ^ CRC_TABLE[((crc ^ byte as u16) & 0xFF) as usize];
    }
    !crc
}

/// Human-readable one-line description of a DNP3 link frame for traffic capture.
/// Includes direction, addresses, function code, and application-layer summary.
pub fn describe_frame(frame: &Dnp3Frame) -> String {
    let dir = if frame.link_control.dir { "TX" } else { "RX" };
    if frame.data.is_empty() {
        return format!("{dir} LNK dst={} src={} fc={:#04x} | link-service", frame.destination, frame.source, frame.link_control.function);
    }
    // User data is a transport-layer PDU: byte 0 is the transport header (FIR|FIN|sequence),
    // then the application fragment: control octet, function code, and for a response the two
    // IIN octets, then object headers.
    let th = frame.data[0];
    let mut s = format!(
        "{dir} LNK dst={} src={} fc={:#04x} fir={} fin={} seq={} | ",
        frame.destination, frame.source, frame.link_control.function,
        th & 0x80 != 0, th & 0x40 != 0, th & 0x3F
    );
    if frame.data.len() >= 3 {
        let app = frame.data[1];
        let func = frame.data[2];
        let app_name = match func {
            0x00 => "CONFIRM",
            0x01 => "READ",
            0x02 => "WRITE",
            0x03 => "SELECT",
            0x04 => "OPERATE",
            0x05 => "DIRECT_OPERATE",
            0x06 => "DIRECT_OPERATE_NOACK",
            0x07 => "FREEZE",
            0x08 => "FREEZE_NOACK",
            0x09 => "FREEZE_CLEAR",
            0x0A => "FREEZE_CLEAR_NOACK",
            0x0B => "FREEZE_AT_TIME",
            0x0C => "FREEZE_AT_TIME_NOACK",
            0x0D => "COLD_RESTART",
            0x0E => "WARM_RESTART",
            0x14 => "ENABLE_UNSOLICITED",
            0x15 => "DISABLE_UNSOLICITED",
            0x17 => "DELAY_MEASURE",
            0x81 => "RESPONSE",
            0x82 => "UNSOLICITED_RESPONSE",
            _ => "UNKNOWN",
        };
        let mut parts = vec![format!("{app_name} seq={}", app & 0x0F)];
        for (bit, name) in [(0x80, "FIR"), (0x40, "FIN"), (0x20, "CON"), (0x10, "UNS")] {
            if app & bit != 0 {
                parts.push(name.to_string());
            }
        }
        let objects_at = if func >= 0x81 {
            if frame.data.len() >= 5 && (frame.data[3] != 0 || frame.data[4] != 0) {
                parts.push(format!("IIN={:02X}{:02X}", frame.data[3], frame.data[4]));
            }
            5
        } else {
            3
        };
        if frame.data.len() >= objects_at + 3 {
            parts.push(format!("G{}V{}", frame.data[objects_at], frame.data[objects_at + 1]));
        }
        s.push_str(&parts.join(" "));
        s.push_str(&format!(" | {}B", frame.data.len()));
    }
    s
}

/// Link-layer control octet (IEEE 1815-2012 §9.1):
/// DIR (bit 7) | PRM (bit 6) | FCB (bit 5) | FCV (bit 4) | function (bits 3-0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlByte {
    /// Direction: true = master to outstation.
    pub dir: bool,
    /// Primary message (request or user data). False for secondary responses.
    pub prm: bool,
    /// Frame count bit (retired in balanced/TCP operation; transmit 0).
    pub fcb: bool,
    /// Frame count valid. Must be 0 except for confirmed user data.
    pub fcv: bool,
    /// Link function code (0-15).
    pub function: u8,
}

impl ControlByte {
    // Link function codes (IEEE 1815-2012 §9.1).
    pub const RESET_LINK_STATES: u8 = 0x00;
    pub const ACK: u8 = 0x00;
    pub const NACK: u8 = 0x01;
    pub const TEST_LINK_STATES: u8 = 0x02;
    pub const USER_DATA_CONFIRMED: u8 = 0x03;
    pub const USER_DATA_NO_CONFIRM: u8 = 0x04;
    pub const REQUEST_LINK_STATUS: u8 = 0x09;
    pub const LINK_STATUS: u8 = 0x0B;

    /// Primary link service frame from the master (e.g. RESET LINK STATES
    /// is `master_link(RESET_LINK_STATES)` = 0xC0).
    pub const fn master_link(function: u8) -> Self {
        Self { dir: true, prm: true, fcb: false, fcv: false, function }
    }

    /// Master user data, SEND/NO CONFIRM (0xC4).
    pub const fn master_data() -> Self {
        Self::master_link(Self::USER_DATA_NO_CONFIRM)
    }

    /// Secondary link response from the outstation (ACK = 0x00,
    /// LINK STATUS = 0x0B).
    pub const fn outstation_link(function: u8) -> Self {
        Self { dir: false, prm: false, fcb: false, fcv: false, function }
    }

    /// Outstation user data, SEND/NO CONFIRM (0x44).
    pub const fn outstation_data() -> Self {
        Self { dir: false, prm: true, fcb: false, fcv: false, function: Self::USER_DATA_NO_CONFIRM }
    }

    pub fn to_byte(self) -> u8 {
        let dir = if self.dir { 0x80 } else { 0x00 };
        let prm = if self.prm { 0x40 } else { 0x00 };
        let fcb = if self.fcb { 0x20 } else { 0x00 };
        let fcv = if self.fcv { 0x10 } else { 0x00 };
        dir | prm | fcb | fcv | (self.function & 0x0F)
    }

    pub fn from_byte(byte: u8) -> Self {
        Self {
            dir: byte & 0x80 != 0,
            prm: byte & 0x40 != 0,
            fcb: byte & 0x20 != 0,
            fcv: byte & 0x10 != 0,
            function: byte & 0x0F,
        }
    }
}

/// A DNP3 data-link frame (IEEE 1815-2012 §9.2): 10-octet header
/// (start, LENGTH, CONTROL, destination, source, header CRC) followed by
/// user data in blocks of at most 16 octets, each followed by its own CRC.
#[derive(Debug, Clone, PartialEq)]
pub struct Dnp3Frame {
    pub link_control: ControlByte,
    pub destination: u16,
    pub source: u16,
    /// User data carried by the link layer (the transport-layer PDU).
    /// Empty for link service frames (RESET LINK STATES, ACK, ...), which
    /// per IEEE 1815 carry no user data (LENGTH = 5).
    pub data: Vec<u8>,
}

impl Dnp3Frame {
    pub fn encode(&self) -> Vec<u8> {
        debug_assert!(
            self.data.len() <= 250,
            "link user data must fit LENGTH (5 + len <= 255); segment at the transport layer"
        );
        let mut out = Vec::with_capacity(10 + self.data.len() + 2 * self.data.len().div_ceil(16));
        out.push(0x05);
        out.push(0x64);
        out.push((5 + self.data.len()) as u8);
        out.push(self.link_control.to_byte());
        out.extend_from_slice(&self.destination.to_le_bytes());
        out.extend_from_slice(&self.source.to_le_bytes());
        let header_crc = crc16(&out);
        out.extend_from_slice(&header_crc.to_le_bytes());

        for chunk in self.data.chunks(16) {
            let start = out.len();
            out.extend_from_slice(chunk);
            let block_crc = crc16(&out[start..]);
            out.extend_from_slice(&block_crc.to_le_bytes());
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 10 {
            anyhow::bail!("Frame too short: {} bytes", bytes.len());
        }
        if bytes[0] != 0x05 || bytes[1] != 0x64 {
            anyhow::bail!("Invalid start bytes: {:02X} {:02X}", bytes[0], bytes[1]);
        }

        let length = bytes[2] as usize;
        if length < 5 {
            anyhow::bail!("LENGTH {} below minimum 5", length);
        }
        let user_len = length - 5;
        let expected_total = 10 + user_len + 2 * user_len.div_ceil(16);
        if bytes.len() < expected_total {
            anyhow::bail!(
                "Frame truncated: LENGTH {} requires {} bytes, got {}",
                length,
                expected_total,
                bytes.len()
            );
        }

        let header_crc = u16::from_le_bytes([bytes[8], bytes[9]]);
        let computed = crc16(&bytes[0..8]);
        if computed != header_crc {
            anyhow::bail!(
                "Header CRC mismatch: expected {:04X}, got {:04X}",
                computed,
                header_crc
            );
        }

        let mut data = Vec::with_capacity(user_len);
        let mut consumed = 0;
        let mut offset = 10;
        while consumed < user_len {
            let block_len = std::cmp::min(16, user_len - consumed);
            let block = &bytes[offset..offset + block_len];
            let block_crc = u16::from_le_bytes([bytes[offset + block_len], bytes[offset + block_len + 1]]);
            let computed = crc16(block);
            if computed != block_crc {
                anyhow::bail!(
                    "Data block CRC mismatch at offset {}: expected {:04X}, got {:04X}",
                    offset,
                    computed,
                    block_crc
                );
            }
            data.extend_from_slice(block);
            consumed += block_len;
            offset += block_len + 2;
        }

        Ok(Self {
            link_control: ControlByte::from_byte(bytes[3]),
            destination: bytes[4] as u16 | ((bytes[5] as u16) << 8),
            source: bytes[6] as u16 | ((bytes[7] as u16) << 8),
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crc_ieee_reference_frame() {
        // IEEE 1815-2012 example frame: transmitted CRC is E9 21 (0x21E9 as u16).
        let frame = [0x05, 0x64, 0x05, 0xC0, 0x01, 0x00, 0x00, 0x04];
        assert_eq!(crc16(&frame), 0x21E9);
    }

    #[test]
    fn test_crc_empty_and_padding() {
        assert_eq!(crc16(&[]), 0xFFFF);
        assert_ne!(crc16(&[0x05, 0x64]), 0);
    }

    #[test]
    fn test_control_octet_values() {
        assert_eq!(ControlByte::master_link(ControlByte::RESET_LINK_STATES).to_byte(), 0xC0);
        assert_eq!(ControlByte::master_data().to_byte(), 0xC4);
        assert_eq!(ControlByte::master_link(ControlByte::REQUEST_LINK_STATUS).to_byte(), 0xC9);
        assert_eq!(ControlByte::outstation_link(ControlByte::ACK).to_byte(), 0x00);
        assert_eq!(ControlByte::outstation_link(ControlByte::LINK_STATUS).to_byte(), 0x0B);
        assert_eq!(ControlByte::outstation_data().to_byte(), 0x44);
        let parsed = ControlByte::from_byte(0xC4);
        assert!(parsed.dir && parsed.prm && !parsed.fcb && !parsed.fcv);
        assert_eq!(parsed.function, ControlByte::USER_DATA_NO_CONFIRM);
    }

    #[test]
    fn test_encode_reset_link_states() {
        // KEP Supertrol-3 reference: master RESET LINK STATES to addr 4 from addr 1.
        let frame = Dnp3Frame {
            link_control: ControlByte::master_link(ControlByte::RESET_LINK_STATES),
            destination: 4,
            source: 1,
            data: vec![],
        };
        let bytes = frame.encode();
        assert_eq!(bytes, [0x05, 0x64, 0x05, 0xC0, 0x04, 0x00, 0x01, 0x00, 0x1C, 0x4D]);
    }

    #[test]
    fn test_encode_ack_and_link_status() {
        let ack = Dnp3Frame {
            link_control: ControlByte::outstation_link(ControlByte::ACK),
            destination: 1,
            source: 4,
            data: vec![],
        };
        assert_eq!(ack.encode(), [0x05, 0x64, 0x05, 0x00, 0x01, 0x00, 0x04, 0x00, 0x95, 0x9E]);

        let status = Dnp3Frame {
            link_control: ControlByte::outstation_link(ControlByte::LINK_STATUS),
            destination: 1,
            source: 4,
            data: vec![],
        };
        assert_eq!(status.encode(), [0x05, 0x64, 0x05, 0x0B, 0x01, 0x00, 0x04, 0x00, 0xD6, 0xAE]);
    }

    #[test]
    fn test_decode_rejects_bad_header_crc() {
        let mut bytes = [0x05, 0x64, 0x05, 0xC0, 0x04, 0x00, 0x01, 0x00, 0xC0, 0x01];
        assert!(Dnp3Frame::decode(&bytes).is_err());
        bytes[8] = 0x1C;
        bytes[9] = 0x4D;
        assert!(Dnp3Frame::decode(&bytes).is_ok());
    }

    #[test]
    fn test_decode_rejects_truncated_frame() {
        // LENGTH 7 declares 2 user octets + block CRC: 14 octets required.
        // The pre-fix master sent exactly these 12 bytes and hung.
        let bytes = [0x05, 0x64, 0x07, 0xF0, 0x04, 0x00, 0x01, 0x00, 0xC0, 0x01, 0xF8, 0x59];
        let err = Dnp3Frame::decode(&bytes).unwrap_err().to_string();
        assert!(err.contains("truncated"), "unexpected error: {err}");
    }

    #[test]
    fn test_encode_decode_roundtrip_multiblock() {
        // 40 octets of user data -> 3 blocks (16 + 16 + 8).
        let frame = Dnp3Frame {
            link_control: ControlByte::master_data(),
            destination: 100,
            source: 1,
            data: (0..40).collect(),
        };
        let encoded = frame.encode();
        assert_eq!(encoded[2], 45);
        assert_eq!(encoded.len(), 10 + 40 + 6);
        let decoded = Dnp3Frame::decode(&encoded).expect("roundtrip decode should succeed");
        assert_eq!(decoded, frame);
    }

    #[test]
    fn test_link_service_frames_carry_no_user_data() {
        let frame = Dnp3Frame {
            link_control: ControlByte::master_link(ControlByte::RESET_LINK_STATES),
            destination: 4,
            source: 1,
            data: vec![],
        };
        let bytes = frame.encode();
        assert_eq!(bytes.len(), 10);
        assert_eq!(bytes[2], 5);
        let decoded = Dnp3Frame::decode(&bytes).expect("decode should succeed");
        assert!(decoded.data.is_empty());
        assert_eq!(decoded.link_control.function, ControlByte::RESET_LINK_STATES);
    }
}
