//! DNP3 object headers and object formats (IEEE 1815-2012 clauses 4 and 11).
//!
//! An object block is `group, variation, qualifier`, then a range field, then the objects. The
//! qualifier's high nibble is the prefix code (0 none, 1 or 2 or 3 an index of 1, 2 or 4 octets
//! before every object) and its low nibble is the range code (start-stop indices or an object
//! count). Object sizes come from the group/variation, so a parser has to know every object it
//! meets: it cannot skip one it does not recognise.

/// A single DNP3 point value parsed from an outstation response.
#[derive(Debug, Clone, PartialEq)]
pub struct Dnp3PointValue {
    /// Static group of the point. Event objects report the group of the point they update:
    /// g2 is a g1 point, g22 a g20 point, g32 a g30 point.
    pub group: u8,
    /// Variation the outstation actually sent.
    pub variation: u8,
    pub index: u16,
    pub value: f64,
    /// The object was an event, not a static value.
    pub event: bool,
    /// The flags octet (bit 0 ONLINE, 1 RESTART, 2 COMM_LOST, 3 REMOTE_FORCED, 4 LOCAL_FORCED,
    /// then type-specific bits). `None` for objects without one: packed bits and the variations
    /// without flags, which by definition are online.
    pub flags: Option<u8>,
    /// Absolute time stamp of an event object that carries one, in milliseconds since the Unix
    /// epoch (the 48-bit DNP3 time).
    pub time: Option<u64>,
}

/// Bits of a point's flags octet (IEEE 1815-2012 clause 11.9).
pub mod flag {
    pub const ONLINE: u8 = 0x01;
    pub const RESTART: u8 = 0x02;
    pub const COMM_LOST: u8 = 0x04;
    pub const REMOTE_FORCED: u8 = 0x08;
    pub const LOCAL_FORCED: u8 = 0x10;
    /// Binary inputs: CHATTER_FILTER. Analog inputs: OVER_RANGE. Counters: ROLLOVER.
    pub const BIT5: u8 = 0x20;
    /// Analog inputs: REFERENCE_ERR. Counters: DISCONTINUITY.
    pub const BIT6: u8 = 0x40;
    /// Binary inputs and outputs: the state.
    pub const STATE: u8 = 0x80;
}

/// Human-readable name for a group/variation pair.
pub fn group_variation_name(group: u8, variation: u8) -> &'static str {
    match (group, variation) {
        (1, 1) => "Binary Input (packed)",
        (1, 2) => "Binary Input (with flags)",
        (2, 1) => "Binary Input Event",
        (2, 2) => "Binary Input Event (absolute time)",
        (2, 3) => "Binary Input Event (relative time)",
        (3, 1) => "Double-bit Input (packed)",
        (3, 2) => "Double-bit Input (with flags)",
        (4, 1) => "Double-bit Input Event",
        (4, 2) => "Double-bit Input Event (absolute time)",
        (4, 3) => "Double-bit Input Event (relative time)",
        (10, 1) => "Binary Output Status (packed)",
        (10, 2) => "Binary Output Status (with flags)",
        (11, 1) => "Binary Output Event",
        (11, 2) => "Binary Output Event (absolute time)",
        (12, 1) => "Control Relay Output Block",
        (20, 1) => "Counter (uint32 with flags)",
        (20, 2) => "Counter (uint16 with flags)",
        (20, 5) => "Counter (uint32)",
        (20, 6) => "Counter (uint16)",
        (21, 1) => "Frozen Counter (uint32 with flags)",
        (21, 2) => "Frozen Counter (uint16 with flags)",
        (21, 5) => "Frozen Counter (uint32 with flags and time)",
        (21, 6) => "Frozen Counter (uint16 with flags and time)",
        (21, 9) => "Frozen Counter (uint32)",
        (21, 10) => "Frozen Counter (uint16)",
        (22, 1) => "Counter Event (uint32)",
        (22, 2) => "Counter Event (uint16)",
        (22, 5) => "Counter Event (uint32 with time)",
        (22, 6) => "Counter Event (uint16 with time)",
        (30, 1) => "Analog Input (int32 with flags)",
        (30, 2) => "Analog Input (int16 with flags)",
        (30, 3) => "Analog Input (int32)",
        (30, 4) => "Analog Input (int16)",
        (30, 5) => "Analog Input (float32 with flags)",
        (30, 6) => "Analog Input (float64 with flags)",
        (32, 1) => "Analog Input Event (int32)",
        (32, 2) => "Analog Input Event (int16)",
        (32, 3) => "Analog Input Event (int32 with time)",
        (32, 4) => "Analog Input Event (int16 with time)",
        (32, 5) => "Analog Input Event (float32)",
        (32, 6) => "Analog Input Event (float64)",
        (32, 7) => "Analog Input Event (float32 with time)",
        (32, 8) => "Analog Input Event (float64 with time)",
        (40, 1) => "Analog Output Status (int32 with flags)",
        (40, 2) => "Analog Output Status (int16 with flags)",
        (40, 3) => "Analog Output Status (float32 with flags)",
        (40, 4) => "Analog Output Status (float64 with flags)",
        (41, 1) => "Analog Output Block (int32)",
        (41, 2) => "Analog Output Block (int16)",
        (41, 3) => "Analog Output Block (float32)",
        (41, 4) => "Analog Output Block (float64)",
        (42, 1) => "Analog Output Event (int32)",
        (42, 2) => "Analog Output Event (int16)",
        (42, 3) => "Analog Output Event (int32 with time)",
        (42, 4) => "Analog Output Event (int16 with time)",
        (42, 5) => "Analog Output Event (float32)",
        (42, 6) => "Analog Output Event (float64)",
        (42, 7) => "Analog Output Event (float32 with time)",
        (42, 8) => "Analog Output Event (float64 with time)",
        (50, 1) => "Time and Date",
        (52, 1) => "Time Delay (coarse)",
        (52, 2) => "Time Delay (fine)",
        (60, 1) => "Class 0 data",
        (60, 2) => "Class 1 data",
        (60, 3) => "Class 2 data",
        (60, 4) => "Class 3 data",
        (80, 1) => "Internal Indications",
        _ => "Unknown",
    }
}

/// List of common DNP3 group/variation pairs for point configuration UI.
pub fn common_group_variations() -> Vec<(u8, u8, &'static str)> {
    vec![
        (1, 2, "Binary Input"),
        (10, 2, "Binary Output Status"),
        (20, 1, "Counter"),
        (21, 1, "Frozen Counter"),
        (30, 5, "Analog Input (float32)"),
        (30, 1, "Analog Input (int32)"),
        (30, 2, "Analog Input (int16)"),
        (40, 3, "Analog Output Status (float32)"),
        (40, 1, "Analog Output Status (int32)"),
    ]
}

/// The static group an event group updates; other groups map to themselves.
pub fn static_group(group: u8) -> u8 {
    match group {
        2 => 1,
        4 => 3,
        11 => 10,
        22 => 20,
        23 => 21,
        32 => 30,
        33 => 31,
        42 => 40,
        g => g,
    }
}

fn is_event_group(group: u8) -> bool {
    matches!(group, 2 | 4 | 11 | 22 | 23 | 32 | 33 | 42)
}

/// How the objects of a group/variation are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// Bit-packed, `n` bits per point, least significant bit first within each octet.
    Bits(u8),
    /// Fixed number of octets per object.
    Bytes(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Num {
    U16,
    U32,
    I16,
    I32,
    F32,
    F64,
}

impl Num {
    fn width(self) -> usize {
        match self {
            Num::U16 | Num::I16 => 2,
            Num::U32 | Num::I32 | Num::F32 => 4,
            Num::F64 => 8,
        }
    }

    fn decode(self, b: &[u8]) -> f64 {
        match self {
            Num::U16 => u16::from_le_bytes([b[0], b[1]]) as f64,
            Num::I16 => i16::from_le_bytes([b[0], b[1]]) as f64,
            Num::U32 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Num::I32 => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Num::F32 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            Num::F64 => f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
        }
    }
}

/// An analog or counter object: optional leading flags octet, a value, optional 6-octet time.
#[derive(Debug, Clone, Copy)]
struct Numeric {
    flag: bool,
    num: Num,
    time: bool,
}

impl Numeric {
    const fn new(flag: bool, num: Num, time: bool) -> Self {
        Self { flag, num, time }
    }

    fn size(self) -> usize {
        usize::from(self.flag) + self.num.width() + if self.time { 6 } else { 0 }
    }

    fn decode(self, b: &[u8]) -> f64 {
        self.num.decode(&b[usize::from(self.flag)..])
    }
}

fn numeric(group: u8, variation: u8) -> Option<Numeric> {
    use Num::*;
    let n = Numeric::new;
    Some(match (group, variation) {
        // Counters: 20 static, 21 frozen, 22 event, 23 frozen event.
        (20, 1) => n(true, U32, false),
        (20, 2) => n(true, U16, false),
        (20, 5) => n(false, U32, false),
        (20, 6) => n(false, U16, false),
        (21, 1) => n(true, U32, false),
        (21, 2) => n(true, U16, false),
        (21, 5) => n(true, U32, true),
        (21, 6) => n(true, U16, true),
        (21, 9) => n(false, U32, false),
        (21, 10) => n(false, U16, false),
        (22 | 23, 1) => n(true, U32, false),
        (22 | 23, 2) => n(true, U16, false),
        (22 | 23, 5) => n(true, U32, true),
        (22 | 23, 6) => n(true, U16, true),
        // Analog input.
        (30, 1) => n(true, I32, false),
        (30, 2) => n(true, I16, false),
        (30, 3) => n(false, I32, false),
        (30, 4) => n(false, I16, false),
        (30, 5) => n(true, F32, false),
        (30, 6) => n(true, F64, false),
        // Analog input event (32) and analog output event (42).
        (32 | 42, 1) => n(true, I32, false),
        (32 | 42, 2) => n(true, I16, false),
        (32 | 42, 3) => n(true, I32, true),
        (32 | 42, 4) => n(true, I16, true),
        (32 | 42, 5) => n(true, F32, false),
        (32 | 42, 6) => n(true, F64, false),
        (32 | 42, 7) => n(true, F32, true),
        (32 | 42, 8) => n(true, F64, true),
        // Analog output status.
        (40, 1) => n(true, I32, false),
        (40, 2) => n(true, I16, false),
        (40, 3) => n(true, F32, false),
        (40, 4) => n(true, F64, false),
        // Analog output block: value then a trailing status octet (no leading flags).
        (41, 1) => return Some(Numeric { flag: false, num: I32, time: false }),
        (41, 2) => return Some(Numeric { flag: false, num: I16, time: false }),
        (41, 3) => return Some(Numeric { flag: false, num: F32, time: false }),
        (41, 4) => return Some(Numeric { flag: false, num: F64, time: false }),
        _ => return None,
    })
}

fn layout(group: u8, variation: u8) -> Option<Layout> {
    if let Some(num) = numeric(group, variation) {
        // g41 carries a trailing status octet.
        return Some(Layout::Bytes(num.size() + usize::from(group == 41)));
    }
    Some(match (group, variation) {
        (1 | 10, 1) | (80, 1) => Layout::Bits(1),
        (3, 1) => Layout::Bits(2),
        // Binary and double-bit inputs and outputs with flags; events without time.
        (1 | 10, 2) | (2 | 11, 1) | (3, 2) | (4, 1) => Layout::Bytes(1),
        // Events with absolute time: flags + 48-bit time.
        (2 | 11 | 4, 2) => Layout::Bytes(7),
        // Events with relative time: flags + 16-bit time.
        (2 | 11 | 4, 3) => Layout::Bytes(3),
        (12, 1) => Layout::Bytes(11),
        (50, 1) | (50, 3) | (51, 1) | (51, 2) => Layout::Bytes(6),
        (52, 1) | (52, 2) => Layout::Bytes(2),
        _ => return None,
    })
}

/// State bit of a binary flags octet.
fn binary_state(flags: u8) -> f64 {
    if flags & 0x80 != 0 { 1.0 } else { 0.0 }
}

/// Little-endian 48-bit DNP3 time at the start of `bytes`.
fn time48(bytes: &[u8]) -> Option<u64> {
    let bytes = bytes.get(..6)?;
    Some(bytes.iter().rev().fold(0u64, |acc, &b| (acc << 8) | u64::from(b)))
}

/// The flags octet and absolute time of one object, where its variation carries them.
fn item_meta(group: u8, variation: u8, bytes: &[u8]) -> (Option<u8>, Option<u64>) {
    if let Some(num) = numeric(group, variation) {
        if !num.flag {
            return (None, None);
        }
        let time = if num.time { time48(&bytes[1 + num.num.width()..]) } else { None };
        return (bytes.first().copied(), time);
    }
    let time = match (group, variation) {
        (2 | 4 | 11, 2) => time48(&bytes[1..]),
        _ => None,
    };
    (bytes.first().copied(), time)
}

fn decode_item(group: u8, variation: u8, bytes: &[u8]) -> Option<f64> {
    if let Some(num) = numeric(group, variation) {
        return match group {
            // Static analog/counter, events and output status are data points.
            20 | 21 | 22 | 23 | 30 | 32 | 40 | 42 => Some(num.decode(bytes)),
            _ => None,
        };
    }
    match group {
        // Binary input and output status with flags: state in bit 7.
        1 | 2 | 10 | 11 => Some(binary_state(bytes[0])),
        // Double-bit input: state in bits 6-7, 2 = DETERMINED_ON.
        3 | 4 => Some(if (bytes[0] >> 6) & 0x03 == 2 { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Why block parsing stopped before the end of the data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseIssue {
    /// An object whose size is not known here, so nothing after it can be located.
    Unsupported { group: u8, variation: u8 },
    /// The block runs past the end of the fragment.
    Truncated { group: u8, variation: u8 },
    /// A qualifier this parser does not handle.
    BadQualifier { group: u8, variation: u8, qualifier: u8 },
}

impl std::fmt::Display for ParseIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported { group, variation } => {
                write!(f, "unsupported object g{group}v{variation}")
            }
            Self::Truncated { group, variation } => {
                write!(f, "object block g{group}v{variation} is cut off")
            }
            Self::BadQualifier { group, variation, qualifier } => {
                write!(f, "unsupported qualifier {qualifier:#04x} on g{group}v{variation}")
            }
        }
    }
}

/// One object header with its range and the object octets that follow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectBlock<'a> {
    pub group: u8,
    pub variation: u8,
    pub qualifier: u8,
    /// First index for a start-stop range; 0 for count ranges.
    pub start: u32,
    /// Number of objects. 0 for qualifier 0x06 (all objects), which carries none.
    pub count: u32,
    /// Octets of index before each object (0, 1, 2 or 4).
    pub prefix: usize,
    /// The objects exactly as sent, index prefixes included.
    pub body: &'a [u8],
}

/// The blocks found in a fragment, and why parsing stopped early if it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedObjects<'a> {
    pub blocks: Vec<ObjectBlock<'a>>,
    pub issue: Option<ParseIssue>,
}

/// Parse the object blocks that follow an application header.
///
/// `has_data` is true for responses and for requests that carry object data (WRITE, SELECT,
/// OPERATE, DIRECT OPERATE). A READ, FREEZE or ENABLE/DISABLE UNSOLICITED request lists only
/// object headers, so pass false for those.
pub fn parse_object_blocks(mut data: &[u8], has_data: bool) -> ParsedObjects<'_> {
    let mut blocks = Vec::new();
    while !data.is_empty() {
        let result = parse_block(data, has_data);
        match result {
            Ok((block, rest)) => {
                blocks.push(block);
                data = rest;
            }
            Err(issue) => return ParsedObjects { blocks, issue: Some(issue) },
        }
    }
    ParsedObjects { blocks, issue: None }
}

fn le(bytes: &[u8]) -> u32 {
    bytes.iter().rev().fold(0u32, |acc, &b| (acc << 8) | b as u32)
}

fn parse_block(data: &[u8], has_data: bool) -> Result<(ObjectBlock<'_>, &[u8]), ParseIssue> {
    if data.len() < 3 {
        return Err(ParseIssue::Truncated { group: data.first().copied().unwrap_or(0), variation: 0 });
    }
    let (group, variation, qualifier) = (data[0], data[1], data[2]);
    let truncated = ParseIssue::Truncated { group, variation };
    let bad_qualifier = ParseIssue::BadQualifier { group, variation, qualifier };
    let rest = &data[3..];

    let prefix = match qualifier >> 4 {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 4,
        _ => return Err(bad_qualifier),
    };
    // (start, count, octets of range field)
    let (start, count, range_len) = match qualifier & 0x0F {
        0x00 | 0x01 | 0x02 => {
            let width = 1usize << (qualifier & 0x0F);
            if prefix != 0 {
                return Err(bad_qualifier);
            }
            if rest.len() < 2 * width {
                return Err(truncated);
            }
            let (start, stop) = (le(&rest[..width]), le(&rest[width..2 * width]));
            if stop < start {
                return Err(bad_qualifier);
            }
            (start, stop - start + 1, 2 * width)
        }
        0x06 => {
            if prefix != 0 {
                return Err(bad_qualifier);
            }
            (0, 0, 0)
        }
        0x07 | 0x08 | 0x09 => {
            let width = 1usize << ((qualifier & 0x0F) - 7);
            if rest.len() < width {
                return Err(truncated);
            }
            (0, le(&rest[..width]), width)
        }
        _ => return Err(bad_qualifier),
    };
    let rest = &rest[range_len..];

    let body_len = if has_data && count > 0 {
        let size = layout(group, variation).ok_or(ParseIssue::Unsupported { group, variation })?;
        match size {
            Layout::Bits(bits) => {
                if prefix != 0 {
                    return Err(bad_qualifier);
                }
                (count as usize * bits as usize).div_ceil(8)
            }
            Layout::Bytes(n) => (count as usize).checked_mul(prefix + n).ok_or(truncated.clone())?,
        }
    } else {
        0
    };
    if rest.len() < body_len {
        return Err(truncated);
    }
    let (body, rest) = rest.split_at(body_len);
    Ok((
        ObjectBlock { group, variation, qualifier, start, count, prefix, body },
        rest,
    ))
}

impl ObjectBlock<'_> {
    /// The data points in this block. Objects that are not point data (time, CROB, IIN, ...)
    /// produce none.
    pub fn points(&self) -> Vec<Dnp3PointValue> {
        let Some(layout) = layout(self.group, self.variation) else {
            return Vec::new();
        };
        let group = static_group(self.group);
        let event = is_event_group(self.group);
        let mut out = Vec::with_capacity(self.count as usize);
        match layout {
            Layout::Bits(bits) => {
                // Packed bits are g1/g10 (single-bit) and g3 (double-bit) static values.
                let per_octet = 8 / bits as usize;
                for i in 0..self.count as usize {
                    let octet = self.body[i / per_octet];
                    let shift = (i % per_octet) * bits as usize;
                    let raw = (octet >> shift) & ((1u8 << bits) - 1);
                    let value = match (self.group, bits) {
                        (1 | 10, 1) => f64::from(raw),
                        (3, 2) => f64::from(u8::from(raw == 2)),
                        _ => continue,
                    };
                    let Some(index) = u16::try_from(self.start + i as u32).ok() else { break };
                    out.push(Dnp3PointValue {
                        group,
                        variation: self.variation,
                        index,
                        value,
                        event,
                        flags: None,
                        time: None,
                    });
                }
            }
            Layout::Bytes(size) => {
                let stride = self.prefix + size;
                for i in 0..self.count as usize {
                    let item = &self.body[i * stride..(i + 1) * stride];
                    let (index, object) = if self.prefix == 0 {
                        (self.start + i as u32, item)
                    } else {
                        (le(&item[..self.prefix]), &item[self.prefix..])
                    };
                    let Some(value) = decode_item(self.group, self.variation, object) else { return out };
                    let Ok(index) = u16::try_from(index) else { continue };
                    let (flags, time) = item_meta(self.group, self.variation, object);
                    out.push(Dnp3PointValue {
                        group,
                        variation: self.variation,
                        index,
                        value,
                        event,
                        flags,
                        time,
                    });
                }
            }
        }
        out
    }
}

/// Parse every data point in the object bytes of one response fragment. Parsing stops at the
/// first object it cannot size, and that condition is returned next to the points found so far.
pub fn parse_points(objects: &[u8]) -> (Vec<Dnp3PointValue>, Option<ParseIssue>) {
    let parsed = parse_object_blocks(objects, true);
    let points = parsed.blocks.iter().flat_map(ObjectBlock::points).collect();
    (points, parsed.issue)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_state_is_bit_7_of_the_flags_octet() {
        // g1v2, range 0..2: online+state, online, offline-with-state-bit.
        let data = [1, 2, 0x00, 0, 2, 0x81, 0x01, 0x80];
        let (points, issue) = parse_points(&data);
        assert_eq!(issue, None);
        let values: Vec<f64> = points.iter().map(|p| p.value).collect();
        assert_eq!(values, [1.0, 0.0, 1.0]);
        assert!(points.iter().all(|p| p.group == 1 && !p.event));
        assert_eq!(points.iter().map(|p| p.index).collect::<Vec<_>>(), [0, 1, 2]);
    }

    #[test]
    fn packed_binary_is_lsb_first() {
        // g1v1, indices 5..14 = 10 points over 2 octets.
        let data = [1, 1, 0x00, 5, 14, 0b0000_0101, 0b0000_0010];
        let (points, issue) = parse_points(&data);
        assert_eq!(issue, None);
        assert_eq!(points.len(), 10);
        let on: Vec<u16> = points.iter().filter(|p| p.value == 1.0).map(|p| p.index).collect();
        assert_eq!(on, [5, 7, 14]);
    }

    #[test]
    fn analog_variations_with_and_without_flags() {
        // g30v1 int32 + flags, g30v4 int16 no flags, g30v5 float32 + flags, g30v6 float64 + flags.
        let mut data = vec![30, 1, 0x00, 0, 0, 0x01];
        data.extend_from_slice(&(-123456i32).to_le_bytes());
        data.extend_from_slice(&[30, 4, 0x00, 1, 1]);
        data.extend_from_slice(&(-7i16).to_le_bytes());
        data.extend_from_slice(&[30, 5, 0x00, 2, 2, 0x01]);
        data.extend_from_slice(&220.5f32.to_le_bytes());
        data.extend_from_slice(&[30, 6, 0x00, 3, 3, 0x01]);
        data.extend_from_slice(&1.25f64.to_le_bytes());
        let (points, issue) = parse_points(&data);
        assert_eq!(issue, None);
        let values: Vec<f64> = points.iter().map(|p| p.value).collect();
        assert_eq!(values, [-123456.0, -7.0, 220.5, 1.25]);
        assert_eq!(points.iter().map(|p| p.variation).collect::<Vec<_>>(), [1, 4, 5, 6]);
    }

    #[test]
    fn counters_and_two_byte_ranges() {
        // g20v1 uint32 + flags over a 2-octet start/stop range (qualifier 0x01).
        let mut data = vec![20, 1, 0x01, 0x00, 0x01, 0x01, 0x01];
        for v in [1000u32, 70000] {
            data.push(0x01);
            data.extend_from_slice(&v.to_le_bytes());
        }
        let (points, issue) = parse_points(&data);
        assert_eq!(issue, None);
        assert_eq!(points.iter().map(|p| (p.index, p.value)).collect::<Vec<_>>(), [(256, 1000.0), (257, 70000.0)]);
    }

    #[test]
    fn indexed_events_with_two_byte_prefix_and_time() {
        // g32v7 float32 event with flags and time, qualifier 0x28 (2-octet count, 2-octet index).
        let mut data = vec![32, 7, 0x28, 0x02, 0x00];
        for (idx, v) in [(4u16, 1.5f32), (9u16, -2.0f32)] {
            data.extend_from_slice(&idx.to_le_bytes());
            data.push(0x01);
            data.extend_from_slice(&v.to_le_bytes());
            data.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        }
        let (points, issue) = parse_points(&data);
        assert_eq!(issue, None);
        assert_eq!(points.len(), 2);
        assert!(points.iter().all(|p| p.event && p.group == 30));
        assert_eq!((points[0].index, points[0].value), (4, 1.5));
        assert_eq!((points[1].index, points[1].value), (9, -2.0));
    }

    #[test]
    fn one_octet_prefix_events() {
        // g2v1 binary event, qualifier 0x17: count 2, 1-octet index prefix.
        let data = [2, 1, 0x17, 0x02, 3, 0x81, 7, 0x01];
        let (points, issue) = parse_points(&data);
        assert_eq!(issue, None);
        assert_eq!(points.iter().map(|p| (p.group, p.index, p.value, p.event)).collect::<Vec<_>>(), [(1, 3, 1.0, true), (1, 7, 0.0, true)]);
    }

    #[test]
    fn unknown_object_stops_parsing_and_keeps_earlier_points() {
        let data = [1, 2, 0x00, 0, 0, 0x81, 99, 9, 0x00, 0, 0, 0xFF];
        let parsed = parse_object_blocks(&data, true);
        assert_eq!(parsed.blocks.len(), 1);
        assert_eq!(parsed.issue, Some(ParseIssue::Unsupported { group: 99, variation: 9 }));
        let (points, issue) = parse_points(&data);
        assert_eq!(points.len(), 1);
        assert!(issue.unwrap().to_string().contains("g99v9"));
    }

    #[test]
    fn truncated_blocks_are_reported() {
        assert!(matches!(parse_points(&[30, 5, 0x00, 0, 1, 0x01, 0, 0, 0]).1, Some(ParseIssue::Truncated { .. })));
        assert!(matches!(parse_points(&[30, 5]).1, Some(ParseIssue::Truncated { .. })));
        assert!(matches!(parse_points(&[30, 5, 0x00, 5]).1, Some(ParseIssue::Truncated { .. })));
    }

    #[test]
    fn rejected_qualifiers() {
        // Object-size prefixes (4-6) and reserved range codes are not parsed.
        assert!(matches!(parse_points(&[110, 1, 0x57, 1]).1, Some(ParseIssue::BadQualifier { .. })));
        assert!(matches!(parse_points(&[30, 1, 0x0B, 1]).1, Some(ParseIssue::BadQualifier { .. })));
        // A start-stop range cannot carry index prefixes.
        assert!(matches!(parse_points(&[30, 1, 0x10, 0, 0]).1, Some(ParseIssue::BadQualifier { .. })));
        // stop < start
        assert!(matches!(parse_points(&[30, 1, 0x00, 5, 2]).1, Some(ParseIssue::BadQualifier { .. })));
    }

    #[test]
    fn request_headers_carry_no_object_data() {
        let data = [60, 2, 0x06, 60, 3, 0x06, 60, 4, 0x06, 60, 1, 0x06];
        let parsed = parse_object_blocks(&data, false);
        assert_eq!(parsed.issue, None);
        assert_eq!(
            parsed.blocks.iter().map(|b| (b.group, b.variation, b.qualifier)).collect::<Vec<_>>(),
            [(60, 2, 6), (60, 3, 6), (60, 4, 6), (60, 1, 6)]
        );
        // A ranged READ (qualifier 0x00) still has no body.
        let ranged = parse_object_blocks(&[30, 0, 0x00, 0, 9], false);
        assert_eq!(ranged.blocks[0].count, 10);
        assert!(ranged.blocks[0].body.is_empty());
    }

    #[test]
    fn non_point_objects_are_skipped_by_size() {
        // g50v1 time (6 octets), then a g30v5 point: the time must be stepped over correctly.
        let mut data = vec![50, 1, 0x07, 1, 1, 2, 3, 4, 5, 6, 30, 5, 0x00, 0, 0, 0x01];
        data.extend_from_slice(&3.5f32.to_le_bytes());
        let (points, issue) = parse_points(&data);
        assert_eq!(issue, None);
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].value, 3.5);
    }

    #[test]
    fn flags_and_times_come_with_the_objects_that_carry_them() {
        // g2v2 binary event: flags 0x81 (ONLINE + state), time 1 700 000 000 123 ms.
        let mut data = vec![2, 2, 0x17, 1, 9, 0x81];
        data.extend_from_slice(&1_700_000_000_123u64.to_le_bytes()[..6]);
        // g2v3 relative time: flags only.
        data.extend_from_slice(&[2, 3, 0x17, 1, 3, 0x05, 0x10, 0x00]);
        // g1v1 packed: no flags at all.
        data.extend_from_slice(&[1, 1, 0x00, 0, 1, 0b10]);
        // g30v3 int32 without flags.
        data.extend_from_slice(&[30, 3, 0x00, 0, 0]);
        data.extend_from_slice(&42i32.to_le_bytes());
        // g22v5 counter event with flags (ROLLOVER) and time.
        data.extend_from_slice(&[22, 5, 0x17, 1, 2, flag::ONLINE | flag::BIT5]);
        data.extend_from_slice(&9u32.to_le_bytes());
        data.extend_from_slice(&[1, 0, 0, 0, 0, 0]);
        let (points, issue) = parse_points(&data);
        assert_eq!(issue, None);
        let meta: Vec<(u8, u16, Option<u8>, Option<u64>)> =
            points.iter().map(|p| (p.group, p.index, p.flags, p.time)).collect();
        assert_eq!(
            meta,
            [
                (1, 9, Some(0x81), Some(1_700_000_000_123)),
                (1, 3, Some(0x05), None),
                (1, 0, None, None),
                (1, 1, None, None),
                (30, 0, None, None),
                (20, 2, Some(0x21), Some(1)),
            ]
        );
        assert_eq!(points[1].value, 0.0, "COMM_LOST without the state bit is off");
    }

    #[test]
    fn event_groups_map_to_static_groups() {
        assert_eq!(static_group(2), 1);
        assert_eq!(static_group(22), 20);
        assert_eq!(static_group(32), 30);
        assert_eq!(static_group(42), 40);
        assert_eq!(static_group(30), 30);
    }
}
