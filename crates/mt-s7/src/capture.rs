//! Packets of Wireshark sample captures (wiki.wireshark.org/SampleCaptures), S7 PDUs from the S7
//! header on (TPKT and COTP stripped), used as conformance fixtures.
//!
//! `STATUS_*` are from `s7comm_reading_plc_status.pcap` (an S7-1500 CPU module, IM 151-8 PN/DP),
//! `BLOCKS_*` from `s7comm_program_blocklist_onlineview.pcap` and `DOWNLOAD_*` from
//! `s7comm_downloading_block_db1.pcap`; the number is the frame number in Wireshark.
//!
//! The client in the captures writes its PDU reference in little-endian byte order, as snap7 does
//! on x86: reference 3 shows as `03 00`. The reference is an opaque token the PLC echoes; tests
//! that compare against a capture pass the reference as [`pdu_ref`] reads it.

/// Hex text to bytes.
pub(crate) fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
}

/// The PDU reference of a captured PDU, as the two bytes at offset 4 read big-endian.
pub(crate) fn pdu_ref(pdu: &[u8]) -> u16 {
    u16::from_be_bytes([pdu[4], pdu[5]])
}

/// Packet 13: SZL 0x0132 index 4 request.
pub(crate) const STATUS_13: &str = "32 07 00 00 03 00 00 08 00 08 00 01 12 04 11 44 01 00 ff 09 00 04 01 32 00 04";

/// Packet 16: SZL 0x0000 index 0 request.
pub(crate) const STATUS_16: &str = "32 07 00 00 04 00 00 08 00 08 00 01 12 04 11 44 01 00 ff 09 00 04 00 00 00 00";

/// Packet 17: SZL 0x0000 first part, more follows.
pub(crate) const STATUS_17: &str = "32 07 00 00 04 00 00 0c 00 da 00 01 12 08 12 84 01 01 05 01 00 00 ff 09 00 d6 00 00 00 00 00 02 00 70 00 00 0f 00 00 02 00 11 01 11 0f 11 00 12 01 12 0f 12 00 13 01 13 00 14 0f 14 00 15 01 15 00 17 01 17 0f 17 00 18 01 18 0f 18 00 19 0f 19 00 1a 0f 1a 00 1b 0f 1b 00 1c 01 1c 0f 1c 00 21 0a 21 0f 21 02 22 00 23 0f 23 00 24 01 24 04 24 05 24 00 25 01 25 02 25 0f 25 01 31 01 32 02 32 00 36 01 36 0f 36 00 37 01 37 0f 37 00 38 01 38 02 38 0f 38 01 39 00 3a 0f 3a 00 74 01 74 0f 74 05 91 0a 91 0c 91 0d 91 00 92 02 92 06 92 0f 92 00 94 01 94 02 94 06 94 07 94 0f 94 00 95 01 95 0f 95 06 96 0c 96 0c 97 0d 97 01 9a 02 9a 0f 9a 0c 9b 00 9c 01 9c 02 9c 03 9c 0f 9c 00 a0 01 a0 0f a0 00 b1 00 b2 00 b3 00 b4 01 b5 02 b5 03 b5";

/// Packet 19: next request, sequence 1.
pub(crate) const STATUS_19: &str = "32 07 00 00 05 00 00 0c 00 04 00 01 12 08 12 44 01 01 00 00 00 00 0a 00 00 00";

/// Packet 20: SZL 0x0000 last part.
pub(crate) const STATUS_20: &str = "32 07 00 00 05 00 00 0c 00 16 00 01 12 08 12 84 01 01 05 00 00 00 ff 09 00 12 04 b5 05 b5 06 b5 07 b5 08 b5 01 b6 02 b6 03 b6 04 b6";

/// Packet 38: SZL 0x0111 index 1 request.
pub(crate) const STATUS_38: &str = "32 07 00 00 0b 00 00 08 00 08 00 01 12 04 11 44 01 00 ff 09 00 04 01 11 00 01";

/// Packet 39: SZL 0x0011 index 1 answer.
pub(crate) const STATUS_39: &str = "32 07 00 00 0b 00 00 0c 00 28 00 01 12 08 12 84 01 01 00 00 00 00 ff 09 00 24 01 11 00 01 00 1c 00 01 00 01 36 45 53 37 20 31 35 31 2d 38 41 42 30 31 2d 30 41 42 30 20 00 c0 00 03 00 01";

/// Packet 41: request for SZL 0x0C91 (not in the CPU).
pub(crate) const STATUS_41: &str = "32 07 00 00 0c 00 00 08 00 08 00 01 12 04 11 44 01 00 ff 09 00 04 0c 91 7f ff";

/// Packet 42: the CPU refuses it.
pub(crate) const STATUS_42: &str = "32 07 00 00 0c 00 00 0c 00 04 00 01 12 08 12 84 01 01 00 00 d4 02 0a 00 00 00";

/// Packet 58: SZL 0x0011 index 0 request.
pub(crate) const STATUS_58: &str = "32 07 00 00 11 00 00 08 00 08 00 01 12 04 11 44 01 00 ff 09 00 04 00 11 00 00";

/// Packet 59: SZL 0x0011 index 0 answer, four records.
pub(crate) const STATUS_59: &str = "32 07 00 00 11 00 00 0c 00 7c 00 01 12 08 12 84 01 02 00 00 00 00 ff 09 00 78 00 11 00 00 00 1c 00 04 00 01 36 45 53 37 20 31 35 31 2d 38 41 42 30 31 2d 30 41 42 30 20 00 c0 00 03 00 01 00 06 36 45 53 37 20 31 35 31 2d 38 41 42 30 31 2d 30 41 42 30 20 00 c0 00 03 00 01 00 07 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 20 00 c0 56 03 02 06 00 81 42 6f 6f 74 20 4c 6f 61 64 65 72 20 20 20 20 20 20 20 20 20 00 00 41 20 09 09";

/// Packet 67: SZL 0x001C index 0 request.
pub(crate) const STATUS_67: &str = "32 07 00 00 14 00 00 08 00 08 00 01 12 04 11 44 01 00 ff 09 00 04 00 1c 00 00";

/// Packet 69: SZL 0x001C first part, more follows.
pub(crate) const STATUS_69: &str = "32 07 00 00 14 00 00 0c 00 da 00 01 12 08 12 84 01 02 06 01 00 00 ff 09 00 d6 00 1c 00 00 00 22 00 0a 00 01 49 4d 31 35 31 2d 38 2d 43 50 55 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 02 49 4d 31 35 31 2d 38 20 50 4e 2f 44 50 20 43 50 55 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 03 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 04 4f 72 69 67 69 6e 61 6c 20 53 69 65 6d 65 6e 73 20 45 71 75 69 70 6d 65 6e 74 00 00 00 00 00 00 00 05 53 20 43 2d 43 36 54 57 37 34 38 38 32 30 31 32 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 07 49 4d 31 35 31 2d 38 20 50 4e 2f 44 50 20 43 50 55 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 08";

/// Packet 71: next request, sequence 2.
pub(crate) const STATUS_71: &str = "32 07 00 00 15 00 00 0c 00 04 00 01 12 08 12 44 01 02 00 00 00 00 0a 00 00 00";

/// Packet 72: SZL 0x001C last part.
pub(crate) const STATUS_72: &str = "32 07 00 00 15 00 00 0c 00 8a 00 01 12 08 12 84 01 02 06 00 00 00 ff 09 00 86 4d 4d 43 20 32 39 30 30 46 43 31 41 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 09 00 2a f6 00 00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 0a 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 0b 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00";

/// Packet 86: SZL 0x0424 request.
pub(crate) const STATUS_86: &str = "32 07 00 00 1a 00 00 08 00 08 00 01 12 04 11 44 01 00 ff 09 00 04 04 24 00 00";

/// Packet 87: SZL 0x0424 answer, RUN.
pub(crate) const STATUS_87: &str = "32 07 00 00 1a 00 00 0c 00 20 00 01 12 08 12 84 01 02 00 00 00 00 ff 09 00 1c 04 24 00 00 00 14 00 01 51 44 ff 08 00 00 00 00 00 00 00 00 14 08 20 12 05 28 34 94";

/// Packet 102: SZL 0x00A0 request.
pub(crate) const STATUS_102: &str = "32 07 00 00 1f 00 00 08 00 08 00 01 12 04 11 44 01 00 ff 09 00 04 00 a0 00 00";

/// Packet 103: SZL 0x00A0 answer, ten entries.
pub(crate) const STATUS_103: &str = "32 07 00 00 1f 00 00 0c 00 d4 00 01 12 08 12 84 01 02 00 00 00 00 ff 09 00 d0 00 a0 00 00 00 14 00 0a 43 02 ff 68 c7 00 00 00 0b 10 77 10 14 08 20 11 53 59 20 24 13 82 1b 64 c7 72 49 4e 0b 10 77 10 14 08 20 11 53 59 20 14 43 01 ff 46 c7 72 49 4e 08 10 77 10 14 08 20 11 53 59 11 64 43 00 ff f3 00 00 00 00 00 00 00 00 14 08 20 11 53 43 49 84 49 4e ff 8f c0 00 00 00 00 00 00 00 14 08 14 22 55 35 68 85 43 02 ff 68 c7 00 00 00 0b 10 77 10 14 08 14 22 31 55 98 65 13 82 1b 64 c7 72 49 4e 0b 10 77 10 14 08 14 22 31 55 98 55 43 01 ff 46 c7 72 49 4e 08 10 77 10 14 08 14 22 31 55 90 15 43 00 ff f3 00 00 00 00 00 00 00 00 14 08 14 22 31 40 30 45 49 4e ff 8f c0 00 00 00 00 00 00 00 14 07 10 22 17 07 68 15";

/// Packet 43: block info of SDB 32768 request.
pub(crate) const BLOCKS_43: &str = "32 07 00 00 04 00 00 08 00 0c 00 01 12 04 11 43 03 00 ff 09 00 08 30 42 33 32 37 36 38 42";

/// Packet 44: block info error.
pub(crate) const BLOCKS_44: &str = "32 07 00 00 04 00 00 0c 00 04 00 01 12 08 12 83 03 00 00 00 d2 09 0a 00 00 00";

/// Packet 58: list all request.
pub(crate) const BLOCKS_58: &str = "32 07 00 00 04 00 00 08 00 04 00 01 12 04 11 43 01 00 0a 00 00 00";

/// Packet 59: list all answer.
pub(crate) const BLOCKS_59: &str = "32 07 00 00 04 00 00 0c 00 20 00 01 12 08 12 83 01 00 00 00 00 00 ff 09 00 1c 30 38 00 01 30 45 00 00 30 43 00 00 30 41 00 01 30 42 00 0c 30 44 00 47 30 46 00 0e";

/// Packet 61: list blocks of type DB request.
pub(crate) const BLOCKS_61: &str = "32 07 00 00 05 00 00 08 00 06 00 01 12 04 11 43 02 00 ff 09 00 02 30 41";

/// Packet 62: list blocks of type DB answer.
pub(crate) const BLOCKS_62: &str = "32 07 00 00 05 00 00 0c 00 08 00 01 12 08 12 83 02 01 00 00 00 00 ff 09 00 04 00 01 22 05";

/// Packet 95: list blocks of type SFC request.
pub(crate) const BLOCKS_95: &str = "32 07 00 00 05 00 00 08 00 06 00 01 12 04 11 43 02 00 ff 09 00 02 30 44";

/// Packet 96: SFC list, first part, more follows.
pub(crate) const BLOCKS_96: &str = "32 07 00 00 05 00 00 0c 00 d4 00 01 12 08 12 83 02 01 04 01 00 00 ff 09 00 d0 00 00 42 01 00 01 42 01 00 02 42 01 00 03 42 01 00 04 42 01 00 05 42 01 00 06 42 01 00 0b 42 01 00 0c 42 01 00 0d 42 01 00 0e 42 01 00 0f 42 01 00 11 42 01 00 12 42 01 00 13 42 01 00 14 42 01 00 15 42 01 00 16 42 01 00 17 42 01 00 18 42 01 00 1c 42 01 00 1d 42 01 00 1e 42 01 00 1f 42 01 00 20 42 01 00 21 42 01 00 22 42 01 00 24 42 01 00 25 42 01 00 26 42 01 00 27 42 01 00 28 42 01 00 29 42 01 00 2a 42 01 00 2b 42 01 00 2c 42 01 00 2e 42 01 00 2f 42 01 00 31 42 01 00 32 42 01 00 33 42 01 00 34 42 01 00 66 42 01 00 37 42 01 00 38 42 01 00 39 42 01 00 3a 42 01 00 3b 42 01 00 40 42 01 00 46 42 01 00 47 42 01 00 48 42 01";

/// Packet 98: next request, sequence 1.
pub(crate) const BLOCKS_98: &str = "32 07 00 00 06 00 00 0c 00 04 00 01 12 08 12 43 02 01 00 00 00 00 0a 00 00 00";

/// Packet 99: SFC list, last part.
pub(crate) const BLOCKS_99: &str = "32 07 00 00 06 00 00 0c 00 50 00 01 12 08 12 83 02 01 04 00 00 00 ff 09 00 4c 00 49 42 01 00 4a 42 01 00 51 42 01 00 52 42 01 00 53 42 01 00 54 42 01 00 63 42 01 00 65 42 01 00 67 42 01 00 69 42 01 00 6a 42 01 00 6b 42 01 00 6c 42 01 00 6d 42 01 00 70 42 01 00 71 42 01 00 72 42 01 00 7e 42 01 00 7f 42 01";

/// Packet 43: start download request for DB1 (file name _0A00001P).
pub(crate) const DOWNLOAD_43: &str = "32 01 00 00 0e 00 00 20 00 00 1a 00 01 00 00 00 00 00 09 5f 30 41 30 30 30 30 31 50 0d 31 30 30 30 35 30 30 30 30 30 34 30 30";

/// Packet 48: block data, part 1 of 3 (block header and first data).
pub(crate) const DOWNLOAD_48: &str = "32 03 00 00 01 00 00 02 00 e2 00 00 1b 01 00 de 00 fb 70 70 01 01 05 0a 00 01 00 00 01 f4 00 00 00 00 02 91 47 60 2b b5 02 91 47 60 2b b5 00 1c 00 00 00 00 01 90 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00";

/// Packet 51: block data, part 2 of 3.
pub(crate) const DOWNLOAD_51: &str = "32 03 00 00 03 00 00 02 00 e2 00 00 1b 01 00 de 00 fb 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 05 01 00 0c 00 08 00 11";

/// Packet 54: block data, part 3 of 3 (block footer).
pub(crate) const DOWNLOAD_54: &str = "32 03 00 00 05 00 00 02 00 3c 00 00 1b 00 00 38 00 fb 04 01 10 0c 01 00 00 c7 00 05 04 01 00 00 01 01 00 00 00 52 54 57 49 00 00 00 00 00 54 65 73 74 48 4d 49 00 44 41 54 41 44 42 00 00 01 00 c0 df 00 00 00 00 00 00 00 00";

/// Packet 58: PI service _INSE, insert block DB1.
pub(crate) const DOWNLOAD_58: &str = "32 01 00 00 0f 00 00 1a 00 00 28 00 00 00 00 00 00 fd 00 0a 01 00 30 41 30 30 30 30 31 50 05 5f 49 4e 53 45";

/// Packet 59: its answer.
pub(crate) const DOWNLOAD_59: &str = "32 03 00 00 0f 00 00 01 00 00 00 00 28";
