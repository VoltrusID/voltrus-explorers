//! The Slave Information Interface (SII): the device's EEPROM (ETG.1000.6 section 5.4, ETG.2010). A header of 64
//! words (identity, mailbox sizes) followed by categories (strings, general, FMMU, sync managers, PDOs). Parsed for
//! the device panel and built for the simulated devices.

use serde::Serialize;

/// Word addresses in the header.
pub mod word {
    pub const PDI_CONTROL: usize = 0x00;
    pub const ALIAS: usize = 0x04;
    pub const CHECKSUM: usize = 0x07;
    pub const VENDOR: usize = 0x08;
    pub const PRODUCT: usize = 0x0A;
    pub const REVISION: usize = 0x0C;
    pub const SERIAL: usize = 0x0E;
    pub const BOOT_RX_OFFSET: usize = 0x14;
    pub const STD_RX_OFFSET: usize = 0x18;
    pub const STD_RX_SIZE: usize = 0x19;
    pub const STD_TX_OFFSET: usize = 0x1A;
    pub const STD_TX_SIZE: usize = 0x1B;
    pub const MBX_PROTOCOLS: usize = 0x1C;
    pub const SIZE: usize = 0x3E;
    pub const VERSION: usize = 0x3F;
    pub const CATEGORIES: usize = 0x40;
}

pub mod cat {
    pub const NOP: u16 = 0;
    pub const STRINGS: u16 = 10;
    pub const DATA_TYPES: u16 = 20;
    pub const GENERAL: u16 = 30;
    pub const FMMU: u16 = 40;
    pub const SYNCM: u16 = 41;
    pub const FMMU_EX: u16 = 42;
    pub const SYNC_UNIT: u16 = 43;
    pub const TXPDO: u16 = 50;
    pub const RXPDO: u16 = 51;
    pub const DC: u16 = 60;
    pub const END: u16 = 0xFFFF;
}

/// Mailbox protocol bits (header word 0x1C).
pub const MBX_AOE: u16 = 0x01;
pub const MBX_EOE: u16 = 0x02;
pub const MBX_COE: u16 = 0x04;
pub const MBX_FOE: u16 = 0x08;
pub const MBX_SOE: u16 = 0x10;
pub const MBX_VOE: u16 = 0x20;

/// CoE details in the general category.
pub const COE_SDO: u8 = 0x01;
pub const COE_SDO_INFO: u8 = 0x02;
pub const COE_PDO_ASSIGN: u8 = 0x04;
pub const COE_PDO_CONFIG: u8 = 0x08;
pub const COE_STARTUP_UPLOAD: u8 = 0x10;
pub const COE_COMPLETE_ACCESS: u8 = 0x20;

/// CRC-8 of the first 14 bytes, stored in word 7 (polynomial x^8 + x^2 + x + 1, initial value 0xFF).
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0xFF;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

pub fn mailbox_protocol_names(bits: u16) -> Vec<&'static str> {
    [(MBX_AOE, "AoE"), (MBX_EOE, "EoE"), (MBX_COE, "CoE"), (MBX_FOE, "FoE"), (MBX_SOE, "SoE"), (MBX_VOE, "VoE")]
        .into_iter()
        .filter(|(b, _)| bits & b != 0)
        .map(|(_, n)| n)
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiiGeneral {
    pub group: String,
    pub image: String,
    pub order: String,
    pub name: String,
    pub coe_details: u8,
    pub foe_details: u8,
    pub eoe_details: u8,
    pub soe_channels: u8,
    pub ds402_channels: u8,
    pub flags: u8,
    /// Current drawn from the E-bus in mA (negative: the device feeds the E-bus).
    pub ebus_current: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SmType {
    Unused,
    MailboxOut,
    MailboxIn,
    Outputs,
    Inputs,
}

impl SmType {
    pub fn from_code(v: u8) -> SmType {
        match v {
            1 => SmType::MailboxOut,
            2 => SmType::MailboxIn,
            3 => SmType::Outputs,
            4 => SmType::Inputs,
            _ => SmType::Unused,
        }
    }

    pub fn code(self) -> u8 {
        match self {
            SmType::Unused => 0,
            SmType::MailboxOut => 1,
            SmType::MailboxIn => 2,
            SmType::Outputs => 3,
            SmType::Inputs => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiiSyncManager {
    pub start: u16,
    pub len: u16,
    pub control: u8,
    pub enable: u8,
    pub kind: SmType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiiPdoEntry {
    pub index: u16,
    pub sub: u8,
    pub name: String,
    pub data_type: u8,
    pub bits: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiiPdo {
    pub index: u16,
    /// Sync manager the PDO is assigned to by default (0xFF: not assigned).
    pub sm: u8,
    pub name: String,
    pub entries: Vec<SiiPdoEntry>,
}

impl SiiPdo {
    pub fn bits(&self) -> u32 {
        self.entries.iter().map(|e| e.bits as u32).sum()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sii {
    pub pdi_control: u16,
    pub alias: u16,
    pub checksum_ok: bool,
    pub vendor_id: u32,
    pub product_code: u32,
    pub revision: u32,
    pub serial: u32,
    pub boot_rx: (u16, u16),
    pub boot_tx: (u16, u16),
    pub std_rx: (u16, u16),
    pub std_tx: (u16, u16),
    pub mailbox_protocols: u16,
    /// EEPROM size in bytes.
    pub size: u32,
    pub version: u16,
    pub strings: Vec<String>,
    pub general: Option<SiiGeneral>,
    pub fmmus: Vec<u8>,
    pub sync_managers: Vec<SiiSyncManager>,
    pub tx_pdos: Vec<SiiPdo>,
    pub rx_pdos: Vec<SiiPdo>,
    /// Categories found, `(type, size in words)`, in EEPROM order.
    pub categories: Vec<(u16, u16)>,
}

fn w(b: &[u8], word: usize) -> u16 {
    let i = word * 2;
    if i + 1 < b.len() { u16::from_le_bytes([b[i], b[i + 1]]) } else { 0 }
}

fn dw(b: &[u8], word: usize) -> u32 {
    w(b, word) as u32 | (w(b, word + 1) as u32) << 16
}

impl Sii {
    /// String `idx` of the strings category (1-based; 0 is "no string").
    pub fn string(&self, idx: u8) -> String {
        if idx == 0 { String::new() } else { self.strings.get(idx as usize - 1).cloned().unwrap_or_default() }
    }

    /// The device name as masters show it: the general category's name string, else the order number.
    pub fn display_name(&self) -> String {
        self.general.as_ref().map(|g| if g.name.is_empty() { g.order.clone() } else { g.name.clone() }).unwrap_or_default()
    }

    /// Parses an EEPROM image (any length; a short image yields what it holds).
    pub fn parse(b: &[u8]) -> Sii {
        let mut s = Sii {
            pdi_control: w(b, word::PDI_CONTROL),
            alias: w(b, word::ALIAS),
            checksum_ok: b.len() >= 16 && crc8(&b[..14]) == b[14],
            vendor_id: dw(b, word::VENDOR),
            product_code: dw(b, word::PRODUCT),
            revision: dw(b, word::REVISION),
            serial: dw(b, word::SERIAL),
            boot_rx: (w(b, word::BOOT_RX_OFFSET), w(b, word::BOOT_RX_OFFSET + 1)),
            boot_tx: (w(b, word::BOOT_RX_OFFSET + 2), w(b, word::BOOT_RX_OFFSET + 3)),
            std_rx: (w(b, word::STD_RX_OFFSET), w(b, word::STD_RX_SIZE)),
            std_tx: (w(b, word::STD_TX_OFFSET), w(b, word::STD_TX_SIZE)),
            mailbox_protocols: w(b, word::MBX_PROTOCOLS),
            size: (w(b, word::SIZE) as u32 + 1) * 128,
            version: w(b, word::VERSION),
            ..Default::default()
        };
        // Strings first: the other categories refer to them by index.
        let cats = categories(b);
        s.categories = cats.iter().map(|(t, d)| (*t, (d.len() / 2) as u16)).collect();
        for (t, d) in &cats {
            if *t == cat::STRINGS {
                s.strings = parse_strings(d);
            }
        }
        for (t, d) in &cats {
            match *t {
                cat::GENERAL if d.len() >= 14 => {
                    s.general = Some(SiiGeneral {
                        group: s.string(d[0]),
                        image: s.string(d[1]),
                        order: s.string(d[2]),
                        name: s.string(d[3]),
                        coe_details: d[5],
                        foe_details: d[6],
                        eoe_details: d[7],
                        soe_channels: d[8],
                        ds402_channels: d[9],
                        flags: d[11],
                        ebus_current: i16::from_le_bytes([d[12], d[13]]),
                    })
                }
                cat::FMMU => s.fmmus = d.to_vec(),
                cat::SYNCM => {
                    s.sync_managers = d
                        .chunks_exact(8)
                        .map(|c| SiiSyncManager {
                            start: u16::from_le_bytes([c[0], c[1]]),
                            len: u16::from_le_bytes([c[2], c[3]]),
                            control: c[4],
                            enable: c[6],
                            kind: SmType::from_code(c[7]),
                        })
                        .collect()
                }
                cat::TXPDO => s.tx_pdos = parse_pdos(d, &s),
                cat::RXPDO => s.rx_pdos = parse_pdos(d, &s),
                _ => {}
            }
        }
        s
    }
}

/// The categories of an image, `(type, data)`, until the end marker or the end of the image.
pub fn categories(b: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    let mut at = word::CATEGORIES * 2;
    while at + 4 <= b.len() {
        let t = u16::from_le_bytes([b[at], b[at + 1]]);
        if t == cat::END {
            break;
        }
        let words = u16::from_le_bytes([b[at + 2], b[at + 3]]) as usize;
        let start = at + 4;
        let end = (start + words * 2).min(b.len());
        out.push((t & 0x7FFF, &b[start..end]));
        at = start + words * 2;
    }
    out
}

fn parse_strings(d: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let Some(&n) = d.first() else { return out };
    let mut at = 1;
    for _ in 0..n {
        let Some(&len) = d.get(at) else { break };
        let end = (at + 1 + len as usize).min(d.len());
        out.push(String::from_utf8_lossy(&d[at + 1..end]).into_owned());
        at = end;
    }
    out
}

fn parse_pdos(d: &[u8], s: &Sii) -> Vec<SiiPdo> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= d.len() {
        let index = u16::from_le_bytes([d[at], d[at + 1]]);
        let n = d[at + 2] as usize;
        let sm = d[at + 3];
        let name = s.string(d[at + 5]);
        at += 8;
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            if at + 8 > d.len() {
                break;
            }
            entries.push(SiiPdoEntry {
                index: u16::from_le_bytes([d[at], d[at + 1]]),
                sub: d[at + 2],
                name: s.string(d[at + 3]),
                data_type: d[at + 4],
                bits: d[at + 5],
            });
            at += 8;
        }
        out.push(SiiPdo { index, sm, name, entries });
    }
    out
}

/// Builds an EEPROM image. Strings are collected as categories are added and written first.
#[derive(Debug)]
pub struct SiiBuilder {
    header: [u16; 64],
    strings: Vec<String>,
    cats: Vec<(u16, Vec<u8>)>,
}

impl SiiBuilder {
    pub fn new(vendor: u32, product: u32, revision: u32, serial: u32) -> Self {
        let mut b = SiiBuilder { header: [0; 64], strings: Vec::new(), cats: Vec::new() };
        let mut set32 = |at: usize, v: u32| {
            b.header[at] = v as u16;
            b.header[at + 1] = (v >> 16) as u16;
        };
        set32(word::VENDOR, vendor);
        set32(word::PRODUCT, product);
        set32(word::REVISION, revision);
        set32(word::SERIAL, serial);
        // 2 KB (16 Kbit), SII version 1: what most terminals carry.
        b.header[word::SIZE] = 0x000F;
        b.header[word::VERSION] = 1;
        b
    }

    /// Standard mailbox: master-to-device `(offset, size)` and device-to-master `(offset, size)`.
    pub fn mailbox(mut self, rx: (u16, u16), tx: (u16, u16), protocols: u16) -> Self {
        self.header[word::BOOT_RX_OFFSET] = rx.0;
        self.header[word::BOOT_RX_OFFSET + 1] = rx.1;
        self.header[word::BOOT_RX_OFFSET + 2] = tx.0;
        self.header[word::BOOT_RX_OFFSET + 3] = tx.1;
        self.header[word::STD_RX_OFFSET] = rx.0;
        self.header[word::STD_RX_SIZE] = rx.1;
        self.header[word::STD_TX_OFFSET] = tx.0;
        self.header[word::STD_TX_SIZE] = tx.1;
        self.header[word::MBX_PROTOCOLS] = protocols;
        self
    }

    pub fn alias(mut self, alias: u16) -> Self {
        self.header[word::ALIAS] = alias;
        self
    }

    fn string(&mut self, s: &str) -> u8 {
        if s.is_empty() {
            return 0;
        }
        if let Some(i) = self.strings.iter().position(|x| x == s) {
            return (i + 1) as u8;
        }
        self.strings.push(s.to_string());
        self.strings.len() as u8
    }

    pub fn general(mut self, group: &str, order: &str, name: &str, coe_details: u8, ebus_current: i16) -> Self {
        let mut d = vec![0u8; 32];
        d[0] = self.string(group);
        d[1] = 0;
        d[2] = self.string(order);
        d[3] = self.string(name);
        d[5] = coe_details;
        d[12..14].copy_from_slice(&ebus_current.to_le_bytes());
        d[14] = d[0];
        // Port 0 and 1 EBUS (physical memory description), as on EL terminals.
        d[16] = 0x11;
        self.cats.push((cat::GENERAL, d));
        self
    }

    pub fn fmmus(mut self, usage: &[u8]) -> Self {
        self.cats.push((cat::FMMU, usage.to_vec()));
        self
    }

    pub fn sync_managers(mut self, sms: &[SiiSyncManager]) -> Self {
        let mut d = Vec::new();
        for s in sms {
            d.extend_from_slice(&s.start.to_le_bytes());
            d.extend_from_slice(&s.len.to_le_bytes());
            d.push(s.control);
            d.push(0);
            d.push(s.enable);
            d.push(s.kind.code());
        }
        self.cats.push((cat::SYNCM, d));
        self
    }

    pub fn pdos(mut self, tx: bool, pdos: &[SiiPdo]) -> Self {
        let mut d = Vec::new();
        for p in pdos {
            let name = self.string(&p.name);
            d.extend_from_slice(&p.index.to_le_bytes());
            d.push(p.entries.len() as u8);
            d.push(p.sm);
            d.push(0);
            d.push(name);
            d.extend_from_slice(&0u16.to_le_bytes());
            for e in &p.entries {
                let en = self.string(&e.name);
                d.extend_from_slice(&e.index.to_le_bytes());
                d.push(e.sub);
                d.push(en);
                d.push(e.data_type);
                d.push(e.bits);
                d.extend_from_slice(&0u16.to_le_bytes());
            }
        }
        self.cats.push((if tx { cat::TXPDO } else { cat::RXPDO }, d));
        self
    }

    /// The image, padded to its declared size with 0xFF as an erased EEPROM reads.
    pub fn build(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(2048);
        for h in self.header {
            out.extend_from_slice(&h.to_le_bytes());
        }
        let crc = crc8(&out[..14]);
        out[14] = crc;
        out[15] = 0;
        let push_cat = |out: &mut Vec<u8>, t: u16, mut d: Vec<u8>| {
            if d.len() % 2 == 1 {
                d.push(0);
            }
            out.extend_from_slice(&t.to_le_bytes());
            out.extend_from_slice(&((d.len() / 2) as u16).to_le_bytes());
            out.extend_from_slice(&d);
        };
        if !self.strings.is_empty() {
            let mut d = vec![self.strings.len() as u8];
            for s in &self.strings {
                d.push(s.len() as u8);
                d.extend_from_slice(s.as_bytes());
            }
            push_cat(&mut out, cat::STRINGS, d);
        }
        for (t, d) in self.cats {
            push_cat(&mut out, t, d);
        }
        out.extend_from_slice(&cat::END.to_le_bytes());
        let size = (self.header[word::SIZE] as usize + 1) * 128;
        if out.len() < size {
            out.resize(size, 0xFF);
        }
        out
    }
}

pub fn category_name(t: u16) -> &'static str {
    match t {
        cat::NOP => "NOP",
        cat::STRINGS => "Strings",
        cat::DATA_TYPES => "Data types",
        cat::GENERAL => "General",
        cat::FMMU => "FMMU",
        cat::SYNCM => "SyncM",
        cat::FMMU_EX => "FMMU extended",
        cat::SYNC_UNIT => "Sync unit",
        cat::TXPDO => "TxPDO",
        cat::RXPDO => "RxPDO",
        cat::DC => "Distributed clock",
        0x0800..=0x0FFF => "Vendor specific",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CRC-8, polynomial 0x07, init 0xFF, no reflection: check value of "123456789". The real-device dumps in
    /// `tests/sii_dumps.rs` confirm it against stored header checksums.
    #[test]
    fn crc_check_value() {
        assert_eq!(crc8(b"123456789"), 0xFB);
    }

    #[test]
    fn build_and_parse_round_trip() {
        let img = SiiBuilder::new(2, 0x03F0_3052, 0x0011_0000, 1234)
            .mailbox((0x1000, 128), (0x1080, 128), MBX_COE)
            .general("DigIn", "EL1008", "EL1008 8Ch. Dig. Input 24V, 3ms", COE_SDO | COE_SDO_INFO, 90)
            .fmmus(&[2, 0xFF])
            .sync_managers(&[SiiSyncManager { start: 0x1000, len: 1, control: 0x00, enable: 1, kind: SmType::Inputs }])
            .pdos(
                true,
                &[SiiPdo {
                    index: 0x1A00,
                    sm: 0,
                    name: "Channel 1".into(),
                    entries: vec![SiiPdoEntry { index: 0x6000, sub: 1, name: "Input".into(), data_type: 1, bits: 1 }],
                }],
            )
            .build();
        assert_eq!(img.len(), 2048);
        let s = Sii::parse(&img);
        assert!(s.checksum_ok);
        assert_eq!((s.vendor_id, s.product_code, s.revision, s.serial), (2, 0x03F0_3052, 0x0011_0000, 1234));
        assert_eq!(s.std_rx, (0x1000, 128));
        assert_eq!(s.size, 2048);
        assert_eq!(mailbox_protocol_names(s.mailbox_protocols), vec!["CoE"]);
        let g = s.general.as_ref().unwrap();
        assert_eq!((g.group.as_str(), g.order.as_str(), g.ebus_current), ("DigIn", "EL1008", 90));
        assert_eq!(s.display_name(), "EL1008 8Ch. Dig. Input 24V, 3ms");
        assert_eq!(s.sync_managers[0].kind, SmType::Inputs);
        assert_eq!(s.tx_pdos[0].entries[0].name, "Input");
        assert_eq!(s.tx_pdos[0].bits(), 1);
        assert_eq!(s.fmmus, vec![2, 0xFF]);
    }
}
