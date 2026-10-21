//! SZL ("Systemzustandsliste", system status list) reads: CPU identity, operating mode and
//! diagnostic buffer.
//!
//! An SZL read is a user data request of function group 4, subfunction 1 (see
//! [`framing`](crate::framing)). The data of the first request is `FF 09 00 04 <SZL id> <index>`;
//! the answer starts with an 8-byte SZL header, `<id> <index> <record length> <record count>`,
//! followed by the records. When the answer does not fit one PDU the PLC sends it in parts, and a
//! part can end in the middle of a record. Every request after the first one is a "next" request
//! carrying the sequence number of the last answer (snap7 `opReadSZL`).
//!
//! Which list holds what:
//!
//! | SZL | records |
//! |-----|---------|
//! | `0x0011` | module identification, 28 bytes: index, order number (20 chars), type, two version words |
//! | `0x001C` | component identification, 34 bytes: index, 32 characters of text |
//! | `0x0424` | CPU mode, 20 bytes: event id, `0xFF`, mode byte (previous mode high nibble, current low), ..., time stamp |
//! | `0x00A0` | diagnostic buffer, 20 bytes: event id, priority class, OB number, data id, info 1, info 2, time stamp |
//!
//! snap7 reads the first two for `Cli_GetOrderCode` and `Cli_GetCpuInfo`, the third for
//! `Cli_GetPlcStatus`. snap7 has no diagnostic buffer helper; its record layout is the one
//! Wireshark's S7comm dissector decodes (`s7comm_decode_ud_cpu_diagnostic_message`). All of them
//! were checked against the real answers in `s7comm_reading_plc_status.pcap`.
//!
//! The second half of this file is the simulator's side: the records of a fictional CPU.

use crate::framing::{self, GROUP_SZL, RETURN_OK};
use crate::types::*;

/// SZL 0x0000: the list of the SZL ids the PLC has.
pub(crate) const LIST: u16 = 0x0000;
/// SZL 0x0011: module identification.
pub(crate) const MODULE_IDENTIFICATION: u16 = 0x0011;
/// SZL 0x001C: component identification.
pub(crate) const COMPONENT_IDENTIFICATION: u16 = 0x001C;
/// SZL 0x0424: CPU mode (snap7 `Cli_GetPlcStatus`).
pub(crate) const CPU_MODE: u16 = 0x0424;
/// SZL 0x00A0: the diagnostic buffer, all entries.
pub(crate) const DIAG_BUFFER: u16 = 0x00A0;

/// Subfunction of function group 4: read SZL.
pub(crate) const SUBFUNCTION_READ: u8 = 0x01;

/// Length of the SZL header in front of the records.
const SZL_HEADER_LEN: usize = 8;
/// Length of a record of SZL 0x0011.
const MODULE_RECORD_LEN: usize = 28;
/// Length of a record of SZL 0x001C.
const COMPONENT_RECORD_LEN: usize = 34;
/// Length of a record of SZL 0x0424.
const MODE_RECORD_LEN: usize = 20;
/// Length of a record of SZL 0x00A0.
const DIAG_RECORD_LEN: usize = 20;

/// More than this many bytes of one SZL is not a PLC answering but a runaway loop.
const MAX_SZL_BYTES: usize = 256 * 1024;

// ── Client: requests ──

/// The first request of an SZL read: data `FF 09 00 04 <id> <index>`.
pub(crate) fn build_first_request(pdu_ref: u16, id: u16, index: u16) -> Vec<u8> {
    let mut data = vec![RETURN_OK, 0x09, 0x00, 0x04];
    data.extend_from_slice(&id.to_be_bytes());
    data.extend_from_slice(&index.to_be_bytes());
    framing::build_request(pdu_ref, GROUP_SZL, SUBFUNCTION_READ, 0, false, &data)
}

/// The request for the next part of an answer: `seq` is the sequence number of the last part.
pub(crate) fn build_next_request(pdu_ref: u16, seq: u8) -> Vec<u8> {
    framing::build_request(pdu_ref, GROUP_SZL, SUBFUNCTION_READ, seq, true, &[0x0A, 0x00, 0x00, 0x00])
}

// ── Client: answers ──

/// One SZL as the PLC answered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Szl {
    pub id: u16,
    pub index: u16,
    /// Length of one record in bytes.
    pub record_len: u16,
    /// Number of records the header announces.
    pub record_count: u16,
    /// The records, without the SZL header.
    pub data: Vec<u8>,
}

impl Szl {
    /// The records. A PLC that announces more records than it sent (some firmware answers a
    /// partial list with a count of 0xFFFF) yields the ones that are there.
    pub(crate) fn records(&self) -> impl Iterator<Item = &[u8]> {
        let len = self.record_len as usize;
        let available = if len == 0 { 0 } else { self.data.len() / len };
        self.data
            .chunks_exact(len.max(1))
            .take((self.record_count as usize).min(available))
    }
}

/// What reading the next part of an SZL answer led to.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Progress {
    /// The answer is incomplete: ask for the next part with this sequence number.
    More { seq: u8 },
    /// The answer is complete.
    Done(Szl),
}

/// Puts the parts of an SZL answer together.
#[derive(Default)]
pub(crate) struct Assembler {
    szl: Option<Szl>,
    parts: usize,
}

impl Assembler {
    /// Take the next answer PDU of the read.
    pub(crate) fn push(&mut self, pdu: &[u8]) -> Result<Progress, String> {
        let response = framing::parse_response(pdu, GROUP_SZL, SUBFUNCTION_READ)?;
        self.parts += 1;
        if self.parts > framing::MAX_PARTS {
            return Err(format!("the PLC does not stop sending parts of the SZL ({} so far)", self.parts - 1));
        }
        match &mut self.szl {
            None => {
                let p = response.payload;
                if p.len() < SZL_HEADER_LEN {
                    return Err(format!("the first part of the SZL is {} bytes, its header alone is 8", p.len()));
                }
                self.szl = Some(Szl {
                    id: u16::from_be_bytes([p[0], p[1]]),
                    index: u16::from_be_bytes([p[2], p[3]]),
                    record_len: u16::from_be_bytes([p[4], p[5]]),
                    record_count: u16::from_be_bytes([p[6], p[7]]),
                    data: p[SZL_HEADER_LEN..].to_vec(),
                });
            }
            Some(szl) => {
                szl.data.extend_from_slice(response.payload);
                if szl.data.len() > MAX_SZL_BYTES {
                    return Err(format!("the SZL is larger than {MAX_SZL_BYTES} bytes, giving up"));
                }
            }
        }
        if response.more {
            Ok(Progress::More { seq: response.seq })
        } else {
            Ok(Progress::Done(self.szl.take().expect("set above")))
        }
    }
}

/// Text of a fixed-width field: cut at the first NUL, trailing spaces dropped.
fn text(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    bytes[..end].iter().map(|&b| b as char).collect::<String>().trim_end().to_string()
}

fn check_record_len(szl: &Szl, expected: usize, what: &str) -> Result<(), String> {
    if (szl.record_len as usize) < expected {
        return Err(format!("{what} records are {} bytes, expected {expected}", szl.record_len));
    }
    Ok(())
}

/// The CPU identity: order number and firmware from the module identification (SZL 0x0011), the
/// names and serial number from the component identification (SZL 0x001C).
///
/// snap7 picks the fields at fixed offsets; they are found by record index here because a
/// S7-1500 appends a fourth record to SZL 0x0011 (index 0x0081, the boot loader), which would
/// otherwise be taken for the firmware version.
pub(crate) fn parse_cpu_info(module: &Szl, components: &Szl) -> Result<CpuInfo, String> {
    check_record_len(module, MODULE_RECORD_LEN, "module identification")?;
    check_record_len(components, COMPONENT_RECORD_LEN, "component identification")?;

    let module_record = |index: u16| module.records().find(|r| u16::from_be_bytes([r[0], r[1]]) == index);
    // Redundant H-CPUs put bits of the CPU number into the upper bits of the index.
    let component = |index: u16, width: usize| {
        components
            .records()
            .find(|r| u16::from_be_bytes([r[0], r[1]]) & 0x000F == index)
            .map(|r| text(&r[2..2 + width]))
            .unwrap_or_default()
    };

    let order_code = module_record(0x0001)
        .or_else(|| module.records().next())
        .map(|r| text(&r[2..22]))
        .unwrap_or_default();
    let firmware_version = module_record(0x0007).map(firmware).unwrap_or_default();

    Ok(CpuInfo {
        module_type: component(0x0007, 32),
        serial_number: component(0x0005, 24),
        as_name: component(0x0001, 24),
        module_name: component(0x0002, 24),
        firmware_version,
        order_code,
        copyright: component(0x0004, 26),
    })
}

/// `V3.2.6` from a firmware record: a letter and the first version number share one byte pair,
/// the other two numbers follow (`56 03 02 06`). snap7 takes the last three bytes.
fn firmware(record: &[u8]) -> String {
    let letter = if record[24].is_ascii_alphabetic() { record[24] as char } else { 'V' };
    format!("{letter}{}.{}.{}", record[25], record[26], record[27])
}

/// The CPU mode from the first record of SZL 0x0424.
///
/// Byte 3 of the record holds the mode the CPU is in (low nibble) and the one it left (high
/// nibble); `0x08` is RUN and `0x04` STOP, as in snap7 `opGetPlcStatus`. The other values are
/// the mode IDs Wireshark's dissector lists.
pub(crate) fn parse_cpu_mode(szl: &Szl) -> Result<PlcMode, String> {
    let record = szl.records().next().ok_or("the PLC returned no mode record")?;
    if record.len() < 4 {
        return Err(format!("mode record is {} bytes, expected {MODE_RECORD_LEN}", record.len()));
    }
    let mode = match record[3] & 0x0F {
        0x1..=0x4 => "STOP", // update, memory reset, self initialization, internal
        0x5 | 0x7 => "STARTUP", // complete restart, restart
        0x8 => "RUN",
        0xA => "HOLD",
        0xD => "DEFECT",
        _ => "UNKNOWN",
    };
    Ok(PlcMode { mode: mode.to_string() })
}

/// The diagnostic buffer from SZL 0x00A0.
pub(crate) fn parse_diag_buffer(szl: &Szl) -> Result<DiagBuffer, String> {
    check_record_len(szl, DIAG_RECORD_LEN, "diagnostic buffer")?;
    let entries: Vec<DiagEntry> = szl
        .records()
        .map(|r| {
            let event_number = u16::from_be_bytes([r[0], r[1]]);
            DiagEntry {
                event_number,
                timestamp: format_timestamp(&r[12..20]),
                event_text: event_text(event_number),
                priority_class: r[2],
                ob_number: r[3],
                data_id: u16::from_be_bytes([r[4], r[5]]),
                info1: u16::from_be_bytes([r[6], r[7]]),
                info2: u32::from_be_bytes([r[8], r[9], r[10], r[11]]),
            }
        })
        .collect();
    Ok(DiagBuffer { entry_count: entries.len() as u16, entries })
}

/// One BCD byte as a number, `None` if a nibble is not a digit.
fn bcd(byte: u8) -> Option<u32> {
    let (high, low) = (byte >> 4, byte & 0x0F);
    (high < 10 && low < 10).then_some((high * 10 + low) as u32)
}

/// An S7 time stamp (8 BCD bytes: year, month, day, hour, minute, second, two digits of the
/// milliseconds, then the third digit and the weekday) as `YYYY-MM-DD HH:MM:SS.mmm`.
///
/// Years 90 to 99 are 1990 to 1999, 0 to 89 are 2000 to 2089. A time stamp that is no valid date,
/// such as the zeros of a CPU whose clock was never set, comes back in hex.
pub(crate) fn format_timestamp(ts: &[u8]) -> String {
    let field = |i: usize| ts.get(i).copied().and_then(bcd);
    let fields = (field(0), field(1), field(2), field(3), field(4), field(5), field(6), ts.get(7).map(|b| b >> 4));
    if let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(s), Some(ms_high), Some(ms_low)) = fields {
        if (1..=12).contains(&mo) && (1..=31).contains(&d) && h < 24 && mi < 60 && s < 60 && ms_low < 10 {
            let year = if y < 90 { 2000 + y } else { 1900 + y };
            return format!("{year:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{:03}", ms_high * 10 + ms_low as u32);
        }
    }
    let hex: Vec<String> = ts.iter().map(|b| format!("{b:02X}")).collect();
    format!("invalid ({})", hex.join(" "))
}

/// Name of the event class, the high nibble of an event ID (Wireshark `cpu_diag_msg_eventid_class_names`).
fn event_class(event: u16) -> &'static str {
    match event >> 12 {
        0x1 => "Standard OB events",
        0x2 => "Synchronous errors",
        0x3 => "Asynchronous errors",
        0x4 => "Mode transitions",
        0x5 => "Run-time events",
        0x6 => "Communication events",
        0x7 => "Events for fail-safe and fault-tolerant systems",
        0x8 => "Standardized diagnostic data on modules",
        0x9 => "Predefined user events",
        0xA | 0xB => "Freely definable events",
        0xF => "Events for modules other than CPUs",
        _ => "Reserved",
    }
}

/// What an event ID means, for the events whose text is known (Wireshark
/// `cpu_diag_eventid_fix_names`, which follows the Siemens system software manual).
fn known_event(event: u16) -> Option<&'static str> {
    Some(match event {
        0x1381 => "Request for manual warm restart",
        0x1382 => "Request for automatic warm restart",
        0x1385 => "Request for manual cold restart",
        0x1386 => "Request for automatic cold restart",
        0x4300 => "Backed-up power on",
        0x4301 => "Mode transition from STOP to STARTUP",
        0x4302 => "Mode transition from STARTUP to RUN",
        0x4303 => "STOP caused by stop switch being activated",
        0x4304 => "STOP caused by PG STOP operation or by SFB 20 STOP",
        0x4305 => "HOLD: breakpoint reached",
        0x4306 => "HOLD: breakpoint exited",
        0x4307 => "Memory reset started by PG operation",
        0x4308 => "Memory reset started by switch setting",
        0x4309 => "Memory reset started automatically (power on not backed up)",
        0x430A => "HOLD exited, transition to STOP",
        0x430E => "Memory reset executed",
        _ => return None,
    })
}

fn event_text(event: u16) -> String {
    match known_event(event) {
        Some(text) => text.to_string(),
        None => format!("Event 0x{event:04X} ({})", event_class(event)),
    }
}

// ── Simulator: the records of a fictional CPU ──

/// SZL ids the simulator answers, in the order SZL 0x0000 lists them.
const SIM_SZL_IDS: [u16; 5] = [LIST, MODULE_IDENTIFICATION, COMPONENT_IDENTIFICATION, CPU_MODE, DIAG_BUFFER];

const SIM_ORDER_CODE: &str = "SIM-S7-0000-0000";
const SIM_AS_NAME: &str = "SIMULATOR";
const SIM_MODULE_NAME: &str = "S7-1500 SIM";
const SIM_MODULE_TYPE: &str = "S7 Simulator";
const SIM_SERIAL: &str = "SIM-000001";
const SIM_COPYRIGHT: &str = "mt-s7 simulator";

/// The simulator CPU's clock: Monday 2024-03-18, 20:00:05.120 when it "started". S7 weekdays
/// count from Sunday = 1.
fn sim_time(hour: u8, minute: u8, second: u8, millisecond: u16) -> [u8; 8] {
    let bcd = |v: u8| ((v / 10) << 4) | (v % 10);
    [
        0x24,
        0x03,
        0x18,
        bcd(hour),
        bcd(minute),
        bcd(second),
        bcd((millisecond / 10) as u8),
        (((millisecond % 10) as u8) << 4) | 0x02,
    ]
}

/// Fixed-width text field, NUL padded.
fn padded(text: &str, width: usize, fill: u8) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    bytes.resize(width, fill);
    bytes
}

fn module_record(index: u16, order_code: &str, version: [u8; 3]) -> Vec<u8> {
    let mut r = index.to_be_bytes().to_vec();
    r.extend(padded(order_code, 20, b' '));
    r.extend_from_slice(&0u16.to_be_bytes()); // module type
    r.extend_from_slice(&[if index == 0x0007 { b'V' } else { 0 }, version[0]]);
    r.extend_from_slice(&[version[1], version[2]]);
    r
}

fn component_record(index: u16, text: &str) -> Vec<u8> {
    let mut r = index.to_be_bytes().to_vec();
    r.extend(padded(text, 32, 0));
    r
}

/// The newest diagnostic buffer entry first: event, priority class, OB number, time.
const SIM_DIAG_EVENTS: [(u16, u8, u8, [u8; 3]); 12] = [
    (0x4302, 0x1B, 100, [20, 0, 5]),
    (0x1382, 0x1B, 100, [20, 0, 4]),
    (0x4301, 0x1B, 100, [20, 0, 4]),
    (0x4300, 0x00, 0, [20, 0, 3]),
    (0x4304, 0x00, 0, [19, 12, 40]),
    (0x4302, 0x1B, 100, [18, 40, 12]),
    (0x1382, 0x1B, 100, [18, 40, 11]),
    (0x4301, 0x1B, 100, [18, 40, 11]),
    (0x4304, 0x00, 0, [17, 5, 33]),
    (0x4302, 0x1B, 100, [15, 22, 8]),
    (0x1382, 0x1B, 100, [15, 22, 7]),
    (0x4301, 0x1B, 100, [15, 22, 7]),
];

fn diag_record(event: u16, priority: u8, ob: u8, time: [u8; 3]) -> Vec<u8> {
    let mut r = event.to_be_bytes().to_vec();
    r.extend_from_slice(&[priority, ob]);
    r.extend_from_slice(&[0, 0]); // data id
    r.extend_from_slice(&[0, 0]); // info 1
    r.extend_from_slice(&[0, 0, 0, 0]); // info 2
    r.extend_from_slice(&sim_time(time[0], time[1], time[2], 120));
    r
}

/// The CPU mode record: event ID, `FF`, mode byte (RUN, left no mode), 4 reserved bytes, 4 bytes of
/// startup info, time stamp. The layout of capture packet 87.
fn mode_record() -> Vec<u8> {
    let mut r = 0x4302u16.to_be_bytes().to_vec();
    r.extend_from_slice(&[0xFF, 0x08]);
    r.extend_from_slice(&[0; 8]);
    r.extend_from_slice(&sim_time(20, 0, 5, 120));
    r
}

/// The simulator's answer to an SZL read: `(record length, records)`, or `None` for an SZL or
/// index it does not have.
pub(crate) fn sim_records(id: u16, index: u16) -> Option<(u16, Vec<u8>)> {
    match (id, index) {
        (LIST, 0) => {
            let records = SIM_SZL_IDS.iter().flat_map(|id| id.to_be_bytes()).collect();
            Some((2, records))
        }
        (MODULE_IDENTIFICATION, 0 | 0x0001 | 0x0006 | 0x0007) => {
            let all = [
                module_record(0x0001, SIM_ORDER_CODE, [1, 0, 0]),
                module_record(0x0006, SIM_ORDER_CODE, [1, 0, 0]),
                module_record(0x0007, "", [1, 0, 0]),
            ];
            let wanted = all.into_iter().filter(|r| index == 0 || u16::from_be_bytes([r[0], r[1]]) == index);
            Some((MODULE_RECORD_LEN as u16, wanted.flatten().collect()))
        }
        (COMPONENT_IDENTIFICATION, 0 | 0x0001..=0x0005 | 0x0007 | 0x0008) => {
            let all = [
                component_record(0x0001, SIM_AS_NAME),
                component_record(0x0002, SIM_MODULE_NAME),
                component_record(0x0003, ""),
                component_record(0x0004, SIM_COPYRIGHT),
                component_record(0x0005, SIM_SERIAL),
                component_record(0x0007, SIM_MODULE_TYPE),
                component_record(0x0008, ""),
            ];
            let wanted = all.into_iter().filter(|r| index == 0 || u16::from_be_bytes([r[0], r[1]]) == index);
            Some((COMPONENT_RECORD_LEN as u16, wanted.flatten().collect()))
        }
        (CPU_MODE, 0) => Some((MODE_RECORD_LEN as u16, mode_record())),
        (DIAG_BUFFER, 0) => {
            let records = SIM_DIAG_EVENTS
                .iter()
                .flat_map(|&(event, priority, ob, time)| diag_record(event, priority, ob, time))
                .collect();
            Some((DIAG_RECORD_LEN as u16, records))
        }
        _ => None,
    }
}

/// The whole answer for an SZL read: the SZL header followed by the records. The simulator cuts
/// it into PDU-sized parts.
pub(crate) fn sim_payload(id: u16, index: u16) -> Option<Vec<u8>> {
    let (record_len, records) = sim_records(id, index)?;
    let count = records.len() / record_len as usize;
    let mut payload = Vec::with_capacity(SZL_HEADER_LEN + records.len());
    payload.extend_from_slice(&id.to_be_bytes());
    payload.extend_from_slice(&index.to_be_bytes());
    payload.extend_from_slice(&record_len.to_be_bytes());
    payload.extend_from_slice(&(count as u16).to_be_bytes());
    payload.extend_from_slice(&records);
    Some(payload)
}

/// Decode the data of an SZL request into `(id, index)`: `FF 09 00 04 <id> <index>`.
pub(crate) fn parse_first_request_data(data: &[u8]) -> Option<(u16, u16)> {
    (data.len() >= 8 && data[0] == RETURN_OK).then(|| {
        (u16::from_be_bytes([data[4], data[5]]), u16::from_be_bytes([data[6], data[7]]))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::*;

    /// snap7 `opReadSZL` first request (s7_micro_client.cpp) builds `00 01 12 04 11 44 01 00`
    /// and `FF 09 00 04 <id> <index>`. The captured requests are byte-identical for every id and
    /// index the CPU was asked for, including the ones snap7's `opGetOrderCode` (0x0011),
    /// `opGetCpuInfo` (0x001C) and `opGetPlcStatus` (0x0424) send.
    #[test]
    fn first_request_matches_snap7_and_the_capture() {
        for (packet, id, index) in [
            (STATUS_13, 0x0132, 0x0004),
            (STATUS_16, LIST, 0),
            (STATUS_38, 0x0111, 1),
            (STATUS_41, 0x0C91, 0x7FFF),
            (STATUS_58, MODULE_IDENTIFICATION, 0),
            (STATUS_67, COMPONENT_IDENTIFICATION, 0),
            (STATUS_86, CPU_MODE, 0),
            (STATUS_102, DIAG_BUFFER, 0),
        ] {
            let captured = hex(packet);
            assert_eq!(build_first_request(pdu_ref(&captured), id, index), captured, "SZL 0x{id:04X}");
        }
    }

    /// snap7 builds the next request with `Plen 08, Uk 12`, the sequence number of the last
    /// answer, 12 bytes of parameters and `0A 00 00 00` as data: the captured packets agree.
    #[test]
    fn next_request_matches_snap7_and_the_capture() {
        for (packet, seq) in [(STATUS_19, 1), (STATUS_71, 2)] {
            let captured = hex(packet);
            assert_eq!(build_next_request(pdu_ref(&captured), seq), captured);
        }
    }

    fn read(packets: &[&str]) -> Szl {
        let mut assembler = Assembler::default();
        for (i, packet) in packets.iter().enumerate() {
            match assembler.push(&hex(packet)).unwrap() {
                Progress::More { .. } => assert!(i + 1 < packets.len()),
                Progress::Done(szl) => {
                    assert_eq!(i + 1, packets.len());
                    return szl;
                }
            }
        }
        panic!("the answer never completed");
    }

    #[test]
    fn module_identification_from_the_capture() {
        let szl = read(&[STATUS_59]);
        assert_eq!((szl.id, szl.index, szl.record_len, szl.record_count), (0x0011, 0, 28, 4));
        assert_eq!(szl.records().count(), 4);
        let single = read(&[STATUS_39]);
        assert_eq!((single.index, single.record_count), (1, 1));
        assert_eq!(single.records().next().unwrap().len(), 28);
    }

    /// `STATUS_17` and `STATUS_20`: SZL 0x0000, 112 records of 2 bytes in two parts. The first
    /// part says more follows and gives sequence number 1 for the next request.
    #[test]
    fn a_multi_packet_szl_is_reassembled() {
        let mut assembler = Assembler::default();
        assert_eq!(assembler.push(&hex(STATUS_17)).unwrap(), Progress::More { seq: 1 });
        let Progress::Done(szl) = assembler.push(&hex(STATUS_20)).unwrap() else { panic!("not done") };
        assert_eq!((szl.id, szl.index, szl.record_len, szl.record_count), (0, 0, 2, 0x70));
        assert_eq!(szl.data.len(), 224);
        let ids: Vec<u16> = szl.records().map(|r| u16::from_be_bytes([r[0], r[1]])).collect();
        assert_eq!(ids.len(), 112);
        assert!(ids.contains(&0x0011) && ids.contains(&0x0424) && ids.contains(&0x00A0));
        assert_eq!(*ids.last().unwrap(), 0x04B6);
    }

    /// `STATUS_69` and `STATUS_72`: SZL 0x001C, ten records of 34 bytes. The first part ends after
    /// the index word of record 8, in the middle of a record.
    #[test]
    fn a_szl_cut_inside_a_record_is_reassembled() {
        let mut assembler = Assembler::default();
        assert_eq!(assembler.push(&hex(STATUS_69)).unwrap(), Progress::More { seq: 2 });
        let Progress::Done(szl) = assembler.push(&hex(STATUS_72)).unwrap() else { panic!("not done") };
        assert_eq!((szl.id, szl.record_len, szl.record_count), (0x001C, 34, 10));
        assert_eq!(szl.data.len(), 340);
        let indices: Vec<u16> = szl.records().map(|r| u16::from_be_bytes([r[0], r[1]])).collect();
        assert_eq!(indices, [1, 2, 3, 4, 5, 7, 8, 9, 10, 11]);
    }

    #[test]
    fn cpu_info_from_the_capture() {
        let module = read(&[STATUS_59]);
        let components = read(&[STATUS_69, STATUS_72]);
        let info = parse_cpu_info(&module, &components).unwrap();
        assert_eq!(info.order_code, "6ES7 151-8AB01-0AB0");
        assert_eq!(info.firmware_version, "V3.2.6", "from the firmware record, not the boot loader behind it");
        assert_eq!(info.as_name, "IM151-8-CPU");
        assert_eq!(info.module_name, "IM151-8 PN/DP CPU");
        assert_eq!(info.module_type, "IM151-8 PN/DP CPU");
        assert_eq!(info.serial_number, "S C-C6TW74882012");
        assert_eq!(info.copyright, "Original Siemens Equipment");
    }

    #[test]
    fn cpu_info_rejects_records_that_are_too_short() {
        let mut module = read(&[STATUS_59]);
        let components = read(&[STATUS_69, STATUS_72]);
        module.record_len = 20;
        assert!(parse_cpu_info(&module, &components).unwrap_err().contains("28"));
    }

    #[test]
    fn mode_from_the_capture_is_run() {
        let szl = read(&[STATUS_87]);
        assert_eq!((szl.id, szl.record_len, szl.record_count), (0x0424, 20, 1));
        assert_eq!(parse_cpu_mode(&szl).unwrap().mode, "RUN");
    }

    #[test]
    fn mode_byte_values() {
        let mode = |byte: u8| {
            let mut record = vec![0x51, 0x44, 0xFF, byte];
            record.resize(20, 0);
            parse_cpu_mode(&Szl { id: CPU_MODE, index: 0, record_len: 20, record_count: 1, data: record }).unwrap().mode
        };
        assert_eq!(mode(0x08), "RUN");
        assert_eq!(mode(0x04), "STOP", "snap7 S7CpuStatusStop");
        assert_eq!(mode(0x03), "STOP", "older CPUs: STOP (self initialization)");
        assert_eq!(mode(0x05), "STARTUP");
        assert_eq!(mode(0x07), "STARTUP");
        assert_eq!(mode(0x0A), "HOLD");
        assert_eq!(mode(0x0D), "DEFECT");
        assert_eq!(mode(0x00), "UNKNOWN");
        // The high nibble is the mode the CPU left (STOP) on its way to RUN.
        assert_eq!(mode(0x48), "RUN");
        let empty = Szl { id: CPU_MODE, index: 0, record_len: 20, record_count: 0, data: vec![] };
        assert!(parse_cpu_mode(&empty).is_err());
    }

    /// `STATUS_103`: SZL 0x00A0, ten 20-byte records, newest first.
    #[test]
    fn diagnostic_buffer_from_the_capture() {
        let szl = read(&[STATUS_103]);
        assert_eq!((szl.id, szl.record_len, szl.record_count), (0x00A0, 20, 10));
        let diag = parse_diag_buffer(&szl).unwrap();
        assert_eq!(diag.entry_count, 10);
        let first = &diag.entries[0];
        assert_eq!(first.event_number, 0x4302);
        assert_eq!(first.event_text, "Mode transition from STARTUP to RUN");
        assert_eq!(first.timestamp, "2014-08-20 11:53:59.202");
        assert_eq!(
            (first.priority_class, first.ob_number, first.data_id, first.info1, first.info2),
            (0xFF, 0x68, 0xC700, 0, 0x0B10_7710)
        );
        let second = &diag.entries[1];
        assert_eq!(second.event_number, 0x1382);
        assert_eq!(second.event_text, "Request for automatic warm restart");
        assert_eq!((second.priority_class, second.ob_number), (27, 100), "OB 100 runs at priority class 27");
        assert_eq!(second.timestamp, "2014-08-20 11:53:59.201");
        assert_eq!(diag.entries[3].event_text, "Backed-up power on");
        // The last entry is the oldest.
        assert_eq!(diag.entries[9].timestamp, "2014-07-10 22:17:07.681");
        // An event the table does not know shows its class.
        assert_eq!(diag.entries[4].event_number, 0x494E);
        assert_eq!(diag.entries[4].event_text, "Event 0x494E (Mode transitions)");
    }

    #[test]
    fn timestamps() {
        // Years 90..99 are 19xx, 00..89 are 20xx; the weekday nibble does not disturb the milliseconds.
        assert_eq!(format_timestamp(&hex("99 12 31 23 59 59 99 94")), "1999-12-31 23:59:59.999");
        assert_eq!(format_timestamp(&hex("24 03 18 20 00 05 12 02")), "2024-03-18 20:00:05.120");
        assert_eq!(format_timestamp(&hex("89 01 02 03 04 05 06 71")), "2089-01-02 03:04:05.067");
        assert_eq!(format_timestamp(&[0; 8]), "invalid (00 00 00 00 00 00 00 00)");
        assert_eq!(format_timestamp(&hex("24 13 18 20 00 05 12 02")), "invalid (24 13 18 20 00 05 12 02)");
        assert_eq!(format_timestamp(&hex("2a 03 18 20 00 05 12 02")), "invalid (2A 03 18 20 00 05 12 02)");
    }

    #[test]
    fn announced_count_beyond_the_data_is_clamped() {
        let szl = Szl { id: 0, index: 0, record_len: 2, record_count: 0xFFFF, data: vec![1, 2, 3, 4, 5] };
        assert_eq!(szl.records().count(), 2);
        let none = Szl { id: 0, index: 0, record_len: 0, record_count: 3, data: vec![1, 2] };
        assert_eq!(none.records().count(), 0);
    }

    #[test]
    fn a_first_part_shorter_than_the_szl_header_is_an_error() {
        let unit = framing::Unit { seq: 1, data_unit_ref: 0, more: false };
        let pdu = framing::build_response(1, GROUP_SZL, SUBFUNCTION_READ, &unit, &[1, 2, 3, 4]);
        assert!(Assembler::default().push(&pdu).unwrap_err().contains("header"));
    }

    #[test]
    fn a_runaway_answer_is_cut_off() {
        let unit = framing::Unit { seq: 1, data_unit_ref: 0, more: true };
        let part = vec![0u8; 40_000];
        let mut payload = vec![0, 0, 0, 0, 0, 2, 0xFF, 0xFF];
        payload.extend_from_slice(&part);
        let mut assembler = Assembler::default();
        let mut result = Ok(Progress::More { seq: 1 });
        for _ in 0..8 {
            result = assembler.push(&framing::build_response(1, GROUP_SZL, SUBFUNCTION_READ, &unit, &payload));
            if result.is_err() {
                break;
            }
            payload.truncate(0); // later parts carry only data
            payload.extend_from_slice(&part);
        }
        assert!(result.unwrap_err().contains("giving up"));
    }

    // ── The simulator's records are what the parsers read, and what a CPU sends ──

    fn sim_szl(id: u16) -> Szl {
        let payload = sim_payload(id, 0).unwrap();
        let unit = framing::Unit { seq: 1, data_unit_ref: 0, more: false };
        let pdu = framing::build_response(1, GROUP_SZL, SUBFUNCTION_READ, &unit, &payload);
        let Progress::Done(szl) = Assembler::default().push(&pdu).unwrap() else { panic!() };
        szl
    }

    #[test]
    fn simulator_cpu_identity_parses() {
        let info = parse_cpu_info(&sim_szl(MODULE_IDENTIFICATION), &sim_szl(COMPONENT_IDENTIFICATION)).unwrap();
        assert_eq!(info.order_code, SIM_ORDER_CODE);
        assert_eq!(info.firmware_version, "V1.0.0");
        assert_eq!(info.as_name, SIM_AS_NAME);
        assert_eq!(info.module_name, SIM_MODULE_NAME);
        assert_eq!(info.module_type, SIM_MODULE_TYPE);
        assert_eq!(info.serial_number, SIM_SERIAL);
        assert_eq!(info.copyright, SIM_COPYRIGHT);
        assert_eq!(parse_cpu_mode(&sim_szl(CPU_MODE)).unwrap().mode, "RUN");
        let diag = parse_diag_buffer(&sim_szl(DIAG_BUFFER)).unwrap();
        assert_eq!(diag.entry_count, 12);
        assert_eq!(diag.entries[0].timestamp, "2024-03-18 20:00:05.120");
        assert_eq!(diag.entries[0].event_text, "Mode transition from STARTUP to RUN");
        assert!(diag.entries.iter().all(|e| !e.timestamp.starts_with("invalid")));
    }

    /// The simulator's mode record has the layout of `STATUS_87`, byte for byte up to the time stamp.
    #[test]
    fn simulator_mode_record_follows_the_capture() {
        let (len, record) = sim_records(CPU_MODE, 0).unwrap();
        assert_eq!(len, 20);
        let szl = read(&[STATUS_87]);
        let real = szl.records().next().unwrap();
        assert_eq!(record[2..12], real[2..12]);
        assert_eq!(record.len(), real.len());
    }

    #[test]
    fn simulator_has_what_the_szl_list_says() {
        let (len, list) = sim_records(LIST, 0).unwrap();
        assert_eq!(len, 2);
        for id in list.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])) {
            assert!(sim_records(id, 0).is_some(), "SZL 0x{id:04X}");
        }
        assert!(sim_records(0x0C91, 0x7FFF).is_none());
        assert!(sim_records(CPU_MODE, 1).is_none());
        let (_, one) = sim_records(COMPONENT_IDENTIFICATION, 5).unwrap();
        assert_eq!(one.len(), 34);
    }
}
