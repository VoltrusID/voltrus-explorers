//! EtherCAT frames on the wire (ETG.1000.4): an Ethernet II frame with EtherType 0x88A4, a 2-byte EtherCAT header
//! and one or more datagrams, each with a 10-byte header, its data and a 2-byte working counter.

use serde::Serialize;

pub const ETHERTYPE: u16 = 0x88A4;
/// Ethernet header (destination, source, EtherType).
pub const ETH_HEADER: usize = 14;
/// EtherCAT header after the Ethernet header.
pub const ECAT_HEADER: usize = 2;
/// Datagram header: command, index, address, length and flags, IRQ.
pub const DGRAM_HEADER: usize = 10;
pub const WKC_LEN: usize = 2;
/// Most data one datagram can carry in a standard 1514-byte frame.
pub const MAX_DGRAM_DATA: usize = 1498 - DGRAM_HEADER - WKC_LEN;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Cmd {
    Nop,
    Aprd,
    Apwr,
    Aprw,
    Fprd,
    Fpwr,
    Fprw,
    Brd,
    Bwr,
    Brw,
    Lrd,
    Lwr,
    Lrw,
    Armw,
    Frmw,
}

impl Cmd {
    pub fn from_u8(v: u8) -> Option<Cmd> {
        use Cmd::*;
        Some(match v {
            0 => Nop,
            1 => Aprd,
            2 => Apwr,
            3 => Aprw,
            4 => Fprd,
            5 => Fpwr,
            6 => Fprw,
            7 => Brd,
            8 => Bwr,
            9 => Brw,
            10 => Lrd,
            11 => Lwr,
            12 => Lrw,
            13 => Armw,
            14 => Frmw,
            _ => return None,
        })
    }

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn name(self) -> &'static str {
        use Cmd::*;
        match self {
            Nop => "NOP",
            Aprd => "APRD",
            Apwr => "APWR",
            Aprw => "APRW",
            Fprd => "FPRD",
            Fpwr => "FPWR",
            Fprw => "FPRW",
            Brd => "BRD",
            Bwr => "BWR",
            Brw => "BRW",
            Lrd => "LRD",
            Lwr => "LWR",
            Lrw => "LRW",
            Armw => "ARMW",
            Frmw => "FRMW",
        }
    }

    /// Logical (FMMU-mapped) addressing: the 32-bit address is one logical address, not position/station + register.
    pub fn is_logical(self) -> bool {
        matches!(self, Cmd::Lrd | Cmd::Lwr | Cmd::Lrw)
    }

    pub fn reads(self) -> bool {
        use Cmd::*;
        matches!(self, Aprd | Aprw | Fprd | Fprw | Brd | Brw | Lrd | Lrw | Armw | Frmw)
    }

    pub fn writes(self) -> bool {
        use Cmd::*;
        matches!(self, Apwr | Aprw | Fpwr | Fprw | Bwr | Brw | Lwr | Lrw | Armw | Frmw)
    }
}

/// Where a datagram sits inside a frame buffer (offsets from the start of the Ethernet frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DgramSpan {
    pub header: usize,
    pub data: usize,
    pub len: usize,
}

impl DgramSpan {
    pub fn wkc(&self) -> usize {
        self.data + self.len
    }
}

/// Finds the datagrams of an EtherCAT frame. `None` when it is not one (other EtherType, mailbox-gateway or
/// network-variable frames, or a truncated buffer).
pub fn datagrams(frame: &[u8]) -> Option<Vec<DgramSpan>> {
    if frame.len() < ETH_HEADER + ECAT_HEADER || u16::from_be_bytes([frame[12], frame[13]]) != ETHERTYPE {
        return None;
    }
    let hdr = u16::from_le_bytes([frame[14], frame[15]]);
    let total = (hdr & 0x07FF) as usize;
    let kind = hdr >> 12;
    if kind != 1 {
        return None;
    }
    let end = (ETH_HEADER + ECAT_HEADER + total).min(frame.len());
    let mut at = ETH_HEADER + ECAT_HEADER;
    let mut out = Vec::new();
    loop {
        if at + DGRAM_HEADER > end {
            return None;
        }
        let lf = u16::from_le_bytes([frame[at + 6], frame[at + 7]]);
        let len = (lf & 0x07FF) as usize;
        let more = lf & 0x8000 != 0;
        if at + DGRAM_HEADER + len + WKC_LEN > end {
            return None;
        }
        out.push(DgramSpan { header: at, data: at + DGRAM_HEADER, len });
        at += DGRAM_HEADER + len + WKC_LEN;
        if !more {
            break;
        }
    }
    Some(out)
}

/// One datagram, decoded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Datagram {
    pub cmd: Option<Cmd>,
    pub cmd_code: u8,
    pub idx: u8,
    /// Position (auto-increment), station address or 0 for broadcast; the high half of a logical address.
    pub adp: u16,
    /// Register (physical memory) offset; the low half of a logical address.
    pub ado: u16,
    pub circulating: bool,
    pub irq: u16,
    pub data: Vec<u8>,
    pub wkc: u16,
}

impl Datagram {
    pub fn logical(&self) -> u32 {
        (self.ado as u32) << 16 | self.adp as u32
    }
}

pub fn read_datagram(frame: &[u8], span: &DgramSpan) -> Datagram {
    let h = span.header;
    let lf = u16::from_le_bytes([frame[h + 6], frame[h + 7]]);
    Datagram {
        cmd: Cmd::from_u8(frame[h]),
        cmd_code: frame[h],
        idx: frame[h + 1],
        adp: u16::from_le_bytes([frame[h + 2], frame[h + 3]]),
        ado: u16::from_le_bytes([frame[h + 4], frame[h + 5]]),
        circulating: lf & 0x4000 != 0,
        irq: u16::from_le_bytes([frame[h + 8], frame[h + 9]]),
        data: frame[span.data..span.data + span.len].to_vec(),
        wkc: u16::from_le_bytes([frame[span.wkc()], frame[span.wkc() + 1]]),
    }
}

/// Builds an EtherCAT frame from datagrams `(cmd, idx, adp, ado, data)`, padded to the 60-byte Ethernet minimum.
/// The source MAC is `src`; the destination is broadcast, as masters send.
pub fn build_frame(src: [u8; 6], dgrams: &[(Cmd, u8, u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut f = Vec::with_capacity(64);
    f.extend_from_slice(&[0xFF; 6]);
    f.extend_from_slice(&src);
    f.extend_from_slice(&ETHERTYPE.to_be_bytes());
    let body: usize = dgrams.iter().map(|d| DGRAM_HEADER + d.4.len() + WKC_LEN).sum();
    f.extend_from_slice(&((body as u16 & 0x07FF) | 0x1000).to_le_bytes());
    for (i, (cmd, idx, adp, ado, data)) in dgrams.iter().enumerate() {
        f.push(cmd.code());
        f.push(*idx);
        f.extend_from_slice(&adp.to_le_bytes());
        f.extend_from_slice(&ado.to_le_bytes());
        let more = if i + 1 < dgrams.len() { 0x8000 } else { 0 };
        f.extend_from_slice(&((data.len() as u16 & 0x07FF) | more).to_le_bytes());
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(data);
        f.extend_from_slice(&[0, 0]);
    }
    if f.len() < 60 {
        f.resize(60, 0);
    }
    f
}

/// One line per datagram for the frame log: `FPRD 0x1001:0x0130 len 2 wkc 1`.
pub fn describe(d: &Datagram) -> String {
    let name = d.cmd.map(Cmd::name).unwrap_or("?");
    let addr = match d.cmd {
        Some(c) if c.is_logical() => format!("log 0x{:08X}", d.logical()),
        Some(Cmd::Brd | Cmd::Bwr | Cmd::Brw) => format!("reg 0x{:04X}", d.ado),
        Some(Cmd::Aprd | Cmd::Apwr | Cmd::Aprw | Cmd::Armw) => format!("pos {} reg 0x{:04X}", (d.adp as i16).wrapping_neg(), d.ado),
        _ => format!("0x{:04X}:0x{:04X}", d.adp, d.ado),
    };
    let reg = match d.cmd {
        Some(c) if !c.is_logical() => crate::esc::register_name(d.ado).map(|n| format!(" ({n})")).unwrap_or_default(),
        _ => String::new(),
    };
    format!("{name} {addr}{reg} len {} wkc {}", d.data.len(), d.wkc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_finds_datagrams() {
        let f = build_frame([0x10; 6], &[(Cmd::Brd, 1, 0, 0x0000, vec![0; 2]), (Cmd::Fprd, 2, 0x1001, 0x0130, vec![0; 2])]);
        assert_eq!(f.len(), 60);
        assert_eq!(&f[12..14], &[0x88, 0xA4]);
        // Length 2 * (10 + 2 + 2) = 28, type 1.
        assert_eq!(u16::from_le_bytes([f[14], f[15]]), 0x1000 | 28);
        let spans = datagrams(&f).unwrap();
        assert_eq!(spans.len(), 2);
        let d = read_datagram(&f, &spans[1]);
        assert_eq!(d.cmd, Some(Cmd::Fprd));
        assert_eq!((d.adp, d.ado, d.data.len()), (0x1001, 0x0130, 2));
        assert_eq!(describe(&d), "FPRD 0x1001:0x0130 (AL Status) len 2 wkc 0");
    }

    /// A BRD of register 0 (how masters count devices) written out byte by byte from the ETG.1000.4 layout, as one
    /// device with ESC type 0x11 (ET1100) returns it: data OR-ed in, working counter 1.
    #[test]
    fn decodes_handwritten_brd() {
        let mut f = vec![
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x03, 0x01, 0x01, 0x01, 0x01, 0x01, 0x88, 0xA4, // Ethernet
            0x0E, 0x10, // 14 bytes of datagrams, type 1
            0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, // BRD idx 0 adp 0 ado 0 len 2
            0x11, 0x00, // data
            0x01, 0x00, // wkc
        ];
        f.resize(60, 0);
        let spans = datagrams(&f).unwrap();
        let d = read_datagram(&f, &spans[0]);
        assert_eq!(d.cmd, Some(Cmd::Brd));
        assert_eq!(d.data, vec![0x11, 0x00]);
        assert_eq!(d.wkc, 1);
    }

    #[test]
    fn rejects_other_ethertypes_and_truncation() {
        let mut f = build_frame([0; 6], &[(Cmd::Brd, 0, 0, 0, vec![0; 4])]);
        assert!(datagrams(&f[..20]).is_none());
        f[13] = 0x00;
        assert!(datagrams(&f).is_none());
    }
}
