//! The object dictionary of a simulated device: objects with sub-entries, their types, access and values. PDO
//! mapping and assignment objects live here too, and process data is packed from and unpacked into entries.

use std::collections::BTreeMap;

use crate::coe::{ACC_RO, ACC_RW, ACC_RW_PREOP, ACC_RXPDO, ACC_TXPDO, OBJ_ARRAY, OBJ_RECORD, OBJ_VAR};

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub data_type: u16,
    pub bits: u16,
    pub access: u16,
    pub value: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Object {
    pub name: String,
    pub code: u8,
    /// Data type of a VAR or ARRAY object (ARRAY: of its elements); 0 for RECORDs.
    pub data_type: u16,
    /// Sub 0 of ARRAY/RECORD objects is the count (UINT8); a VAR has only sub 0.
    pub entries: BTreeMap<u8, Entry>,
}

impl Object {
    pub fn max_sub(&self) -> u8 {
        if self.code == OBJ_VAR {
            0
        } else {
            self.entries.get(&0).and_then(|e| e.value.first().copied()).unwrap_or(0)
        }
    }
}

/// One mapped entry of a PDO: object index and sub, bit length (index 0 is a gap).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapEntry {
    pub index: u16,
    pub sub: u8,
    pub bits: u8,
}

impl MapEntry {
    pub fn from_u32(v: u32) -> MapEntry {
        MapEntry { index: (v >> 16) as u16, sub: (v >> 8) as u8, bits: v as u8 }
    }

    pub fn to_u32(self) -> u32 {
        (self.index as u32) << 16 | (self.sub as u32) << 8 | self.bits as u32
    }
}

#[derive(Debug, Clone, Default)]
pub struct Od {
    pub objects: BTreeMap<u16, Object>,
}

fn bytes_for(bits: u16) -> usize {
    (bits as usize).div_ceil(8).max(1)
}

impl Od {
    pub fn var(&mut self, index: u16, name: &str, data_type: u16, bits: u16, access: u16, value: &[u8]) -> &mut Self {
        let mut v = value.to_vec();
        if crate::coe::data_type_bits(data_type).is_some() {
            v.resize(bytes_for(bits), 0);
        }
        let mut entries = BTreeMap::new();
        entries.insert(0, Entry { name: name.to_string(), data_type, bits, access, value: v });
        self.objects.insert(index, Object { name: name.to_string(), code: OBJ_VAR, data_type, entries });
        self
    }

    pub fn string(&mut self, index: u16, name: &str, value: &str) -> &mut Self {
        self.var(index, name, 0x0009, (value.len() * 8) as u16, ACC_RO, value.as_bytes())
    }

    /// A RECORD (or an ARRAY when every entry shares `array_type`) with sub 0 holding the count.
    pub fn record(&mut self, index: u16, name: &str, array_type: Option<u16>, sub0_access: u16, subs: &[(u8, &str, u16, u16, u16, &[u8])]) -> &mut Self {
        let mut entries = BTreeMap::new();
        let max = subs.iter().map(|s| s.0).max().unwrap_or(0);
        entries.insert(0, Entry { name: "SubIndex 000".into(), data_type: 0x0005, bits: 8, access: sub0_access, value: vec![max] });
        for &(sub, n, dt, bits, acc, val) in subs {
            let mut v = val.to_vec();
            if crate::coe::data_type_bits(dt).is_some() {
                v.resize(bytes_for(bits), 0);
            }
            entries.insert(sub, Entry { name: n.to_string(), data_type: dt, bits, access: acc, value: v });
        }
        let (code, data_type) = match array_type {
            Some(t) => (OBJ_ARRAY, t),
            None => (OBJ_RECORD, 0),
        };
        self.objects.insert(index, Object { name: name.to_string(), code, data_type, entries });
        self
    }

    /// 0x1018 Identity.
    pub fn identity(&mut self, vendor: u32, product: u32, revision: u32, serial: u32) -> &mut Self {
        self.record(
            0x1018,
            "Identity",
            None,
            ACC_RO,
            &[
                (1, "Vendor ID", 0x0007, 32, ACC_RO, &vendor.to_le_bytes()),
                (2, "Product code", 0x0007, 32, ACC_RO, &product.to_le_bytes()),
                (3, "Revision", 0x0007, 32, ACC_RO, &revision.to_le_bytes()),
                (4, "Serial number", 0x0007, 32, ACC_RO, &serial.to_le_bytes()),
            ],
        )
    }

    /// A PDO mapping object (0x16xx or 0x1Axx), writable in PRE-OP.
    pub fn pdo_mapping(&mut self, index: u16, name: &str, entries: &[MapEntry]) -> &mut Self {
        let vals: Vec<[u8; 4]> = entries.iter().map(|e| e.to_u32().to_le_bytes()).collect();
        let names: Vec<String> = (1..=entries.len()).map(|i| format!("SubIndex {i:03}")).collect();
        let subs: Vec<(u8, &str, u16, u16, u16, &[u8])> =
            vals.iter().enumerate().map(|(i, v)| ((i + 1) as u8, names[i].as_str(), 0x0007, 32, ACC_RW_PREOP, &v[..])).collect();
        self.record(index, name, Some(0x0007), ACC_RW_PREOP, &subs)
    }

    /// 0x1C12 / 0x1C13 PDO assignment, writable in PRE-OP.
    pub fn pdo_assign(&mut self, index: u16, name: &str, pdos: &[u16]) -> &mut Self {
        let vals: Vec<[u8; 2]> = pdos.iter().map(|p| p.to_le_bytes()).collect();
        let names: Vec<String> = (1..=pdos.len()).map(|i| format!("SubIndex {i:03}")).collect();
        let subs: Vec<(u8, &str, u16, u16, u16, &[u8])> =
            vals.iter().enumerate().map(|(i, v)| ((i + 1) as u8, names[i].as_str(), 0x0006, 16, ACC_RW_PREOP, &v[..])).collect();
        self.record(index, name, Some(0x0006), ACC_RW_PREOP, &subs)
    }

    /// 0x1C00 sync manager communication types.
    pub fn sm_types(&mut self, types: &[u8]) -> &mut Self {
        let names: Vec<String> = (1..=types.len()).map(|i| format!("SubIndex {i:03}")).collect();
        let vals: Vec<[u8; 1]> = types.iter().map(|t| [*t]).collect();
        let subs: Vec<(u8, &str, u16, u16, u16, &[u8])> =
            vals.iter().enumerate().map(|(i, v)| ((i + 1) as u8, names[i].as_str(), 0x0005, 8, ACC_RO, &v[..])).collect();
        self.record(0x1C00, "Sync manager type", Some(0x0005), ACC_RO, &subs)
    }

    pub fn entry(&self, index: u16, sub: u8) -> Option<&Entry> {
        self.objects.get(&index)?.entries.get(&sub)
    }

    pub fn entry_mut(&mut self, index: u16, sub: u8) -> Option<&mut Entry> {
        self.objects.get_mut(&index)?.entries.get_mut(&sub)
    }

    /// The raw value of an entry as an integer (little endian, up to 8 bytes); 0 if missing.
    pub fn get_u64(&self, index: u16, sub: u8) -> u64 {
        self.entry(index, sub)
            .map(|e| e.value.iter().take(8).enumerate().fold(0u64, |a, (i, &b)| a | (b as u64) << (8 * i)))
            .unwrap_or(0)
    }

    pub fn get_i64(&self, index: u16, sub: u8) -> i64 {
        let Some(e) = self.entry(index, sub) else { return 0 };
        let bits = (e.bits as u32).clamp(1, 64);
        let v = self.get_u64(index, sub);
        let s = 64 - bits;
        ((v << s) as i64) >> s
    }

    pub fn get_bool(&self, index: u16, sub: u8) -> bool {
        self.get_u64(index, sub) & 1 != 0
    }

    /// Sets an entry's value from an integer, truncated to its size.
    pub fn set_u64(&mut self, index: u16, sub: u8, v: u64) {
        if let Some(e) = self.entry_mut(index, sub) {
            let n = e.value.len().min(8);
            e.value[..n].copy_from_slice(&v.to_le_bytes()[..n]);
        }
    }

    pub fn set_i64(&mut self, index: u16, sub: u8, v: i64) {
        self.set_u64(index, sub, v as u64);
    }

    /// The mapping of the PDOs assigned in `assign` (0x1C12 or 0x1C13). `Err` names a broken reference.
    pub fn assigned_mapping(&self, assign: u16) -> Result<Vec<MapEntry>, String> {
        let Some(a) = self.objects.get(&assign) else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for i in 1..=a.max_sub() {
            let pdo = self.get_u64(assign, i) as u16;
            let Some(p) = self.objects.get(&pdo) else { return Err(format!("PDO 0x{pdo:04X} assigned in 0x{assign:04X} does not exist")) };
            for j in 1..=p.max_sub() {
                let m = MapEntry::from_u32(self.get_u64(pdo, j) as u32);
                if m.index != 0 {
                    let Some(e) = self.entry(m.index, m.sub) else {
                        return Err(format!("0x{:04X}:{:02X} mapped in 0x{pdo:04X} does not exist", m.index, m.sub));
                    };
                    let flag = if assign == 0x1C12 { ACC_RXPDO } else { ACC_TXPDO };
                    if e.access & flag == 0 || e.bits != m.bits as u16 {
                        return Err(format!("0x{:04X}:{:02X} cannot be mapped into 0x{pdo:04X}", m.index, m.sub));
                    }
                }
                out.push(m);
            }
        }
        Ok(out)
    }

    /// Packs the entries of `map` into a byte image (inputs going to the master).
    pub fn pack(&self, map: &[MapEntry], out: &mut [u8]) {
        let mut bit = 0usize;
        for m in map {
            if m.index != 0 {
                let v = self.get_u64(m.index, m.sub);
                for b in 0..m.bits as usize {
                    let at = bit + b;
                    if at / 8 >= out.len() {
                        break;
                    }
                    if v >> b & 1 != 0 {
                        out[at / 8] |= 1 << (at % 8);
                    } else {
                        out[at / 8] &= !(1 << (at % 8));
                    }
                }
            }
            bit += m.bits as usize;
        }
    }

    /// Unpacks a byte image (outputs from the master) into the entries of `map`.
    pub fn unpack(&mut self, map: &[MapEntry], data: &[u8]) {
        let mut bit = 0usize;
        for m in map {
            if m.index != 0 {
                let mut v = 0u64;
                for b in 0..m.bits as usize {
                    let at = bit + b;
                    if at / 8 < data.len() && data[at / 8] >> (at % 8) & 1 != 0 {
                        v |= 1 << b;
                    }
                }
                self.set_u64(m.index, m.sub, v);
            }
            bit += m.bits as usize;
        }
    }

    /// Indices for an OD list request.
    pub fn list(&self, list_type: u16) -> Vec<u16> {
        self.objects
            .iter()
            .filter(|(_, o)| match list_type {
                crate::coe::LIST_RXPDO => o.entries.values().any(|e| e.access & ACC_RXPDO != 0),
                crate::coe::LIST_TXPDO => o.entries.values().any(|e| e.access & ACC_TXPDO != 0),
                _ => true,
            })
            .map(|(i, _)| *i)
            .collect()
    }
}

/// Entries that are both readable and writable everywhere: settings.
pub const SETTING: u16 = ACC_RW;

pub fn total_bits(map: &[MapEntry]) -> usize {
    map.iter().map(|m| m.bits as usize).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_pack_unpack() {
        let mut od = Od::default();
        od.record(
            0x6000,
            "AI Inputs Ch.1",
            None,
            ACC_RO,
            &[
                (1, "Underrange", 0x0001, 1, ACC_RO | ACC_TXPDO, &[0]),
                (2, "Overrange", 0x0001, 1, ACC_RO | ACC_TXPDO, &[1]),
                (0x11, "Value", 0x0003, 16, ACC_RO | ACC_TXPDO, &(-2i16).to_le_bytes()),
            ],
        );
        od.pdo_mapping(
            0x1A00,
            "AI TxPDO-Map Ch.1",
            &[
                MapEntry { index: 0x6000, sub: 1, bits: 1 },
                MapEntry { index: 0x6000, sub: 2, bits: 1 },
                MapEntry { index: 0, sub: 0, bits: 14 },
                MapEntry { index: 0x6000, sub: 0x11, bits: 16 },
            ],
        );
        od.pdo_assign(0x1C13, "TxPDO assign", &[0x1A00]);
        let map = od.assigned_mapping(0x1C13).unwrap();
        assert_eq!(total_bits(&map), 32);
        let mut img = [0u8; 4];
        od.pack(&map, &mut img);
        assert_eq!(img, [0b10, 0, 0xFE, 0xFF]);
        od.unpack(&map, &[0b01, 0, 0x05, 0x00]);
        assert!(od.get_bool(0x6000, 1));
        assert_eq!(od.get_i64(0x6000, 0x11), 5);
        assert_eq!(od.objects[&0x1A00].max_sub(), 4);
    }

    #[test]
    fn broken_mapping_is_reported() {
        let mut od = Od::default();
        od.pdo_mapping(0x1600, "Out", &[MapEntry { index: 0x7000, sub: 1, bits: 16 }]);
        od.pdo_assign(0x1C12, "RxPDO assign", &[0x1600]);
        assert!(od.assigned_mapping(0x1C12).unwrap_err().contains("0x7000:01"));
    }
}
