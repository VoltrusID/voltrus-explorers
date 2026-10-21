//! Block management: listing, upload and delete.
//!
//! **Listing** is a user data request of function group 3 (see [`framing`](crate::framing)).
//! Subfunction 1 returns how many blocks of each type the PLC has; subfunction 2 lists the blocks
//! of one type, four bytes each (number, flag byte, language), in as many parts as the answer
//! needs. The block type in these telegrams is the ASCII character of the sub block type, preceded
//! by `0` (`30 41` is a DB), see [`BlockType::code`].
//!
//! **Upload** is three jobs, as snap7's `opUpload` does it: *start upload* (function 0x1D) names
//! the block with a file name, `_0A00001A` for DB1, and returns an upload id; *upload* (0x1E)
//! returns the block in parts until the PLC says no more follow; *end upload* (0x1F) closes the
//! session. The block comes back as stored in load memory: a 36-byte header, the MC7 code or the
//! data of a DB, and a 56-byte footer.
//!
//! **Delete** is a PI service, job function 0x28 with the service name `_DELE`
//! (`opDelete`; the answer is an ACK_DATA with the single parameter byte 0x28).
//!
//! Layouts are those of snap7 (`s7_types.h`, `s7_micro_client.cpp`) and of the Wireshark sample
//! captures `s7comm_program_blocklist_onlineview.pcap` (listing) and
//! `s7comm_downloading_block_db1.pcap` (the block data layout, and the `_INSE` PI service whose
//! telegram `_DELE` shares). No capture of an upload or a delete was available; those requests are
//! derived from snap7 alone.
//!
//! The second half of this file is the simulator's side.

use crate::framing::{self, GROUP_BLOCKS, RETURN_OK};
use crate::types::*;

/// Subfunction of function group 3: list all block types with their counts.
pub(crate) const SUBFUNCTION_LIST_ALL: u8 = 0x01;
/// Subfunction of function group 3: list the blocks of one type.
pub(crate) const SUBFUNCTION_LIST_OF_TYPE: u8 = 0x02;

/// ASCII `0` in front of the block type code in user data telegrams.
const ZERO: u8 = 0x30;
/// Size of the header of an uploaded block (snap7 `TS7CompactBlockInfo`).
pub(crate) const BLOCK_HEADER_LEN: usize = 36;
/// Size of the footer of an uploaded block (snap7 `TS7BlockFooter`).
const BLOCK_FOOTER_LEN: usize = 56;
/// More blocks of one type than this is not a PLC answering but a runaway loop.
pub(crate) const MAX_BLOCKS_PER_TYPE: usize = 0x10000;
/// More bytes of one block than this is not a PLC ending an upload but a runaway loop. No block
/// in a PLC comes near it.
pub(crate) const MAX_BLOCK_BYTES: usize = 16 * 1024 * 1024;

/// File system letter in the file name of an upload: the active (working memory) module. snap7
/// asks for `A` when it uploads; `P` (passive) is what STEP 7 puts in download and insert names.
const FILE_SYSTEM_UPLOAD: u8 = b'A';
/// File system letter snap7 puts into the delete request: active as well as passive module.
const FILE_SYSTEM_DELETE: u8 = b'B';

// ── Client: listing ──

/// List all block types: data `0A 00 00 00`.
pub(crate) fn build_list_all_request(pdu_ref: u16) -> Vec<u8> {
    framing::build_request(pdu_ref, GROUP_BLOCKS, SUBFUNCTION_LIST_ALL, 0, false, &[0x0A, 0x00, 0x00, 0x00])
}

/// The block counts of a list-all answer, in the order the PLC sent them. Types this crate does
/// not know are left out.
pub(crate) fn parse_list_all(pdu: &[u8]) -> Result<Vec<(BlockType, u16)>, String> {
    let response = framing::parse_response(pdu, GROUP_BLOCKS, SUBFUNCTION_LIST_ALL)?;
    if response.payload.is_empty() {
        return Err("the block count list is empty".into());
    }
    Ok(response
        .payload
        .chunks_exact(4)
        .filter(|item| item[0] == ZERO)
        .filter_map(|item| Some((BlockType::from_code(item[1])?, u16::from_be_bytes([item[2], item[3]]))))
        .collect())
}

/// List the blocks of one type: data `FF 09 00 02 30 <type code>`.
pub(crate) fn build_list_of_type_request(pdu_ref: u16, block_type: BlockType) -> Vec<u8> {
    let data = [RETURN_OK, 0x09, 0x00, 0x02, ZERO, block_type.code()];
    framing::build_request(pdu_ref, GROUP_BLOCKS, SUBFUNCTION_LIST_OF_TYPE, 0, false, &data)
}

/// The request for the next part of a list of one type.
pub(crate) fn build_list_of_type_next_request(pdu_ref: u16, seq: u8) -> Vec<u8> {
    framing::build_request(pdu_ref, GROUP_BLOCKS, SUBFUNCTION_LIST_OF_TYPE, seq, true, &[0x0A, 0x00, 0x00, 0x00])
}

/// One block in a list of one type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ListedBlock {
    pub number: u16,
    pub flags: u8,
    pub language: u8,
}

/// One part of the list of a block type.
#[derive(Debug)]
pub(crate) struct TypeListPage {
    pub blocks: Vec<ListedBlock>,
    /// More parts follow; ask for them with `seq`.
    pub more: bool,
    pub seq: u8,
}

/// Decode one answer of a list-of-type request.
pub(crate) fn parse_list_of_type(pdu: &[u8]) -> Result<TypeListPage, String> {
    let response = framing::parse_response(pdu, GROUP_BLOCKS, SUBFUNCTION_LIST_OF_TYPE)?;
    let blocks = response
        .payload
        .chunks_exact(4)
        .map(|item| ListedBlock { number: u16::from_be_bytes([item[0], item[1]]), flags: item[2], language: item[3] })
        .collect();
    Ok(TypeListPage { blocks, more: response.more, seq: response.seq })
}

/// Name of a block language code (Wireshark `blocklanguage_names`).
pub(crate) fn language_name(code: u8) -> String {
    match code {
        0x00 => "Not defined".into(),
        0x01 => "AWL".into(),
        0x02 => "KOP".into(),
        0x03 => "FUP".into(),
        0x04 => "SCL".into(),
        0x05 => "DB".into(),
        0x06 => "GRAPH".into(),
        0x07 => "SDB".into(),
        0x08 => "CPU-DB".into(),
        0x11 => "SDB (after overall reset)".into(),
        0x12 => "SDB (routing)".into(),
        0x29 => "ENCRYPT".into(),
        other => format!("0x{other:02X}"),
    }
}

// ── Client: upload ──

/// The file name of a block, 9 characters: `_`, the two-character sub block type, the block
/// number as 5 digits and the file system letter (`_0A00001A`).
fn file_name(block_type: BlockType, number: u16, file_system: u8) -> Vec<u8> {
    let mut name = format!("_{}{number:05}", block_type.file_prefix()).into_bytes();
    name.push(file_system);
    name
}

/// *Start upload*: function 0x1D, seven zero bytes (status, two unknown, upload id 0), the length
/// of the file name (9) and the file name.
pub(crate) fn build_start_upload_request(pdu_ref: u16, block_type: BlockType, number: u16) -> Vec<u8> {
    let mut params = vec![S7_START_UPLOAD, 0, 0, 0, 0, 0, 0, 0, 0x09];
    params.extend(file_name(block_type, number, FILE_SYSTEM_UPLOAD));
    framing::job_pdu(pdu_ref, &params, &[])
}

/// The upload id from the answer to *start upload*.
///
/// Parameters: function 0x1D, status, two unknown bytes, the upload id (4 bytes), the length of
/// the block as a counted ASCII string.
pub(crate) fn parse_start_upload_response(pdu: &[u8]) -> Result<u32, String> {
    let (params, _) = framing::split_ack(pdu)?;
    if params.len() < 8 || params[0] != S7_START_UPLOAD {
        return Err(format!("unexpected start upload answer {params:02X?}"));
    }
    Ok(u32::from_be_bytes([params[4], params[5], params[6], params[7]]))
}

/// A request that names an upload session: the function, three zero bytes and the upload id as 4
/// bytes. snap7 (`Uk6[6]`, `Upload_ID`) zero-fills all but the last byte, which is the same for
/// ids below 256.
fn upload_session_request(pdu_ref: u16, function: u8, upload_id: u32) -> Vec<u8> {
    let mut params = vec![function, 0, 0, 0];
    params.extend_from_slice(&upload_id.to_be_bytes());
    framing::job_pdu(pdu_ref, &params, &[])
}

/// *Upload*: function 0x1E and the upload id.
pub(crate) fn build_upload_request(pdu_ref: u16, upload_id: u32) -> Vec<u8> {
    upload_session_request(pdu_ref, S7_UPLOAD, upload_id)
}

/// *End upload*: function 0x1F and the upload id.
pub(crate) fn build_end_upload_request(pdu_ref: u16, upload_id: u32) -> Vec<u8> {
    upload_session_request(pdu_ref, S7_END_UPLOAD, upload_id)
}

/// One part of an uploaded block.
#[derive(Debug)]
pub(crate) struct UploadPart<'a> {
    /// The PLC has more parts; ask again.
    pub more: bool,
    pub payload: &'a [u8],
}

/// Decode the answer to *upload*: parameters `1E <status>` (status 1 while more follows), data
/// `<length (2)> 00 FB <payload>` where the length is the payload length.
pub(crate) fn parse_upload_response(pdu: &[u8]) -> Result<UploadPart<'_>, String> {
    let (params, data) = framing::split_ack(pdu)?;
    if params.len() < 2 || params[0] != S7_UPLOAD {
        return Err(format!("unexpected upload answer {params:02X?}"));
    }
    if data.len() < 4 {
        return Err(format!("upload data is {} bytes, its header alone is 4", data.len()));
    }
    Ok(UploadPart { more: params[1] != 0, payload: &data[4..] })
}

/// Check the answer to *end upload*: an ACK_DATA with the single parameter byte 0x1F.
pub(crate) fn parse_end_upload_response(pdu: &[u8]) -> Result<(), String> {
    let (params, _) = framing::split_ack(pdu)?;
    if params.first() != Some(&S7_END_UPLOAD) {
        return Err(format!("unexpected end upload answer {params:02X?}"));
    }
    Ok(())
}

/// Check an uploaded block against its own header: it has to hold the 36-byte header, and all of
/// the load memory size the header announces (bytes 8 to 11).
pub(crate) fn check_block(block: &[u8]) -> Result<(), String> {
    if block.len() < BLOCK_HEADER_LEN {
        return Err(format!("the block is {} bytes, shorter than its {BLOCK_HEADER_LEN}-byte header", block.len()));
    }
    let announced = u32::from_be_bytes([block[8], block[9], block[10], block[11]]) as usize;
    if block.len() < announced {
        return Err(format!("only {} of the {announced} bytes of the block arrived", block.len()));
    }
    Ok(())
}

// ── Client: delete ──

/// *Delete*: PI service request, function 0x28:
///
/// ```text
/// 28  00 00 00 00 00 00 FD  00 0A  01 00 30 <type> <5 digits> 42  05 5F 44 45 4C 45
/// fn  unknown (7)           len 10 one block, '0', block, 'B'     5, "_DELE"
/// ```
pub(crate) fn build_delete_request(pdu_ref: u16, block_type: BlockType, number: u16) -> Vec<u8> {
    let mut params = vec![S7_PLC_CONTROL, 0, 0, 0, 0, 0, 0, 0xFD, 0x00, 0x0A, 0x01, 0x00, ZERO, block_type.code()];
    params.extend_from_slice(format!("{number:05}").as_bytes());
    params.push(FILE_SYSTEM_DELETE);
    params.push(0x05);
    params.extend_from_slice(b"_DELE");
    framing::job_pdu(pdu_ref, &params, &[])
}

/// Check the answer to a delete: an ACK_DATA without error and the parameter byte 0x28.
pub(crate) fn parse_delete_response(pdu: &[u8]) -> Result<(), String> {
    let (params, _) = framing::split_ack(pdu)?;
    if params.first() != Some(&S7_PLC_CONTROL) {
        return Err(format!("unexpected delete answer {params:02X?}"));
    }
    Ok(())
}

// ── Simulator: blocks of a fictional CPU ──

/// Block language codes of the simulator's blocks.
const LANG_AWL: u8 = 0x01;
const LANG_FUP: u8 = 0x03;
const LANG_DB: u8 = 0x05;
const LANG_SDB: u8 = 0x07;

/// A block the simulator holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SimBlock {
    pub block_type: BlockType,
    pub number: u16,
    pub flags: u8,
    pub language: u8,
    /// MC7 code, or the data of a DB.
    pub data: Vec<u8>,
}

/// A DB with the given data.
pub(crate) fn sim_db(number: u16, data: Vec<u8>) -> SimBlock {
    SimBlock { block_type: BlockType::DB, number, flags: 0x22, language: LANG_DB, data }
}

/// The blocks every simulated CPU has, whatever the image holds. They cannot be deleted.
///
/// Their code is a single `BE` (block end) instruction, `65 00`.
pub(crate) fn sim_fixed_blocks() -> Vec<SimBlock> {
    let block = |block_type, number, flags, language| SimBlock { block_type, number, flags, language, data: vec![0x65, 0x00] };
    vec![
        block(BlockType::OB, 1, 0x22, LANG_FUP),
        block(BlockType::OB, 100, 0x22, LANG_AWL),
        block(BlockType::FB, 1, 0x22, LANG_AWL),
        block(BlockType::FC, 1, 0x22, LANG_AWL),
        block(BlockType::SDB, 0, 0x22, LANG_SDB),
        block(BlockType::SFC, 20, 0x42, LANG_AWL),
        block(BlockType::SFB, 4, 0x42, LANG_AWL),
    ]
}

/// The answer to list-all: for each block type in the PLC's order, `30 <code> <count (2)>`.
pub(crate) fn list_all_payload(blocks: &[SimBlock]) -> Vec<u8> {
    BlockType::ALL
        .iter()
        .flat_map(|&block_type| {
            let count = blocks.iter().filter(|b| b.block_type == block_type).count() as u16;
            let mut item = vec![ZERO, block_type.code()];
            item.extend_from_slice(&count.to_be_bytes());
            item
        })
        .collect()
}

/// The answer to list-of-type: `<number (2)> <flags> <language>` per block, by number. `None` if
/// there is no block of the type.
pub(crate) fn list_of_type_payload(blocks: &[SimBlock], block_type: BlockType) -> Option<Vec<u8>> {
    let mut of_type: Vec<&SimBlock> = blocks.iter().filter(|b| b.block_type == block_type).collect();
    if of_type.is_empty() {
        return None;
    }
    of_type.sort_by_key(|b| b.number);
    Some(
        of_type
            .iter()
            .flat_map(|b| {
                let mut item = b.number.to_be_bytes().to_vec();
                item.extend_from_slice(&[b.flags, b.language]);
                item
            })
            .collect(),
    )
}

/// The block type of a list-of-type request: data `FF 09 00 02 30 <code>`.
pub(crate) fn parse_list_of_type_request_data(data: &[u8]) -> Option<Result<BlockType, u8>> {
    if data.len() < 6 || data[0] != RETURN_OK || data[4] != ZERO {
        return None;
    }
    Some(BlockType::from_code(data[5]).ok_or(data[5]))
}

/// Sub block type of a block in its header.
fn sub_block_type(block_type: BlockType) -> u8 {
    match block_type {
        BlockType::OB => 0x08,
        BlockType::DB => 0x0A,
        BlockType::SDB => 0x0B,
        BlockType::FC => 0x0C,
        BlockType::SFC => 0x0D,
        BlockType::FB => 0x0E,
        BlockType::SFB => 0x0F,
    }
}

/// The simulator's block time stamps: milliseconds since midnight (20:00) and days since
/// 1984-01-01 (2024-03-18).
const SIM_CODE_TIME: [u8; 6] = [0x04, 0x4A, 0xA2, 0x00, 0x39, 0x5F];

/// A block as a PLC holds it in load memory: the 36-byte header of snap7's
/// `TS7CompactBlockInfo`, the data, the 56-byte footer of `TS7BlockFooter`. The header announces
/// the whole length (data + 92) and the length of the data.
pub(crate) fn encode_block(block: &SimBlock) -> Vec<u8> {
    let total = BLOCK_HEADER_LEN + block.data.len() + BLOCK_FOOTER_LEN;
    let mut b = Vec::with_capacity(total);
    b.extend_from_slice(&[0x70, 0x70]); // constant of every block header
    b.push(0x01);
    b.push(0x01); // block flags
    b.push(block.language);
    b.push(sub_block_type(block.block_type));
    b.extend_from_slice(&block.number.to_be_bytes());
    b.extend_from_slice(&(total as u32).to_be_bytes()); // size in load memory
    b.extend_from_slice(&[0; 4]); // security
    b.extend_from_slice(&SIM_CODE_TIME[..4]); // code time, ms
    b.extend_from_slice(&SIM_CODE_TIME[4..]); // code time, days
    b.extend_from_slice(&SIM_CODE_TIME[..4]); // interface time, ms
    b.extend_from_slice(&SIM_CODE_TIME[4..]); // interface time, days
    b.extend_from_slice(&[0; 2]); // SBB length
    b.extend_from_slice(&[0; 2]); // additional length
    b.extend_from_slice(&[0; 2]); // local data length
    b.extend_from_slice(&(block.data.len().min(0xFFFF) as u16).to_be_bytes()); // MC7 length
    debug_assert_eq!(b.len(), BLOCK_HEADER_LEN);

    b.extend_from_slice(&block.data);

    b.extend_from_slice(&[0; 20]);
    for text in ["SIM7", "mt-s7", &format!("{}{}", block.block_type, block.number)] {
        let mut field = text.as_bytes().to_vec();
        field.resize(8, 0);
        b.extend_from_slice(&field);
    }
    b.extend_from_slice(&[0x01, 0x00]); // version
    b.extend_from_slice(&[0; 2]); // checksum, not computed
    b.extend_from_slice(&[0; 8]);
    b
}

/// What a *start upload* request asks for: block type and number from the file name.
pub(crate) fn parse_start_upload_request(params: &[u8]) -> Option<(BlockType, u16)> {
    // 1D, 7 bytes, length 9, then `_ 0 <type> 5 digits <file system>`.
    if params.len() < 18 || params[0] != S7_START_UPLOAD || params[8] != 0x09 || params[9] != b'_' || params[10] != ZERO {
        return None;
    }
    let digits = std::str::from_utf8(&params[12..17]).ok()?;
    Some((BlockType::from_code(params[11])?, digits.parse().ok()?))
}

/// The upload id of an *upload* or *end upload* request.
pub(crate) fn parse_upload_session_request(params: &[u8]) -> Option<u32> {
    (params.len() >= 8).then(|| u32::from_be_bytes([params[4], params[5], params[6], params[7]]))
}

/// A delete request: the block it names, `None` if the PI service is not `_DELE` of one block.
pub(crate) fn parse_delete_request(params: &[u8]) -> Option<(BlockType, u16)> {
    if params.len() < 26
        || params[0] != S7_PLC_CONTROL
        || params[10] != 0x01
        || params[12] != ZERO
        || params[20] != 0x05
        || &params[21..26] != b"_DELE"
    {
        return None;
    }
    let digits = std::str::from_utf8(&params[14..19]).ok()?;
    Some((BlockType::from_code(params[13])?, digits.parse().ok()?))
}

/// The answer to *start upload*: parameters `1D 00 00 00 <upload id (4)> 07 <7 digits>`, the
/// block's length in load memory as counted ASCII.
pub(crate) fn build_start_upload_response(pdu_ref: u16, upload_id: u32, block_len: usize) -> Vec<u8> {
    let mut params = vec![S7_START_UPLOAD, 0, 0, 0];
    params.extend_from_slice(&upload_id.to_be_bytes());
    params.push(0x07);
    params.extend_from_slice(format!("{block_len:07}").as_bytes());
    framing::ack_pdu(pdu_ref, 0, &params, &[])
}

/// One part of the answer to *upload*: parameters `1E <1 while more follows>`, data
/// `<payload length (2)> 00 FB <payload>`.
pub(crate) fn build_upload_response(pdu_ref: u16, payload: &[u8], more: bool) -> Vec<u8> {
    let mut data = (payload.len() as u16).to_be_bytes().to_vec();
    data.extend_from_slice(&[0x00, 0xFB]);
    data.extend_from_slice(payload);
    framing::ack_pdu(pdu_ref, 0, &[S7_UPLOAD, u8::from(more)], &data)
}

/// The answer to *end upload*.
pub(crate) fn build_end_upload_response(pdu_ref: u16) -> Vec<u8> {
    framing::ack_pdu(pdu_ref, 0, &[S7_END_UPLOAD], &[])
}

/// The answer to a delete: the PI service byte, or the error the PLC gives.
pub(crate) fn build_delete_response(pdu_ref: u16, result: Result<(), u16>) -> Vec<u8> {
    match result {
        Ok(()) => framing::ack_pdu(pdu_ref, 0, &[S7_PLC_CONTROL], &[]),
        Err(error) => framing::ack_pdu(pdu_ref, error, &[], &[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::*;

    #[test]
    fn block_type_codes_match_snap7() {
        // s7_types.h: Block_OB 0x38, Block_DB 0x41, Block_SDB 0x42, Block_FC 0x43, Block_SFC 0x44,
        // Block_FB 0x45, Block_SFB 0x46; SubBlk_OB 0x08 ... SubBlk_SFB 0x0F.
        let expected = [
            (BlockType::OB, 0x38, "08"),
            (BlockType::DB, 0x41, "0A"),
            (BlockType::SDB, 0x42, "0B"),
            (BlockType::FC, 0x43, "0C"),
            (BlockType::SFC, 0x44, "0D"),
            (BlockType::FB, 0x45, "0E"),
            (BlockType::SFB, 0x46, "0F"),
        ];
        for (block_type, code, prefix) in expected {
            assert_eq!(block_type.code(), code, "{block_type}");
            assert_eq!(block_type.file_prefix(), prefix, "{block_type}");
            assert_eq!(BlockType::from_code(code), Some(block_type));
            assert_eq!(sub_block_type(block_type), u8::from_str_radix(prefix, 16).unwrap());
        }
        assert_eq!(BlockType::from_code(0x39), None);
        // The PLC lists the types in this order.
        let order: Vec<u8> = BlockType::ALL.iter().map(BlockType::code).collect();
        assert_eq!(order, [0x38, 0x45, 0x43, 0x41, 0x42, 0x44, 0x46]);
    }

    // ── Listing ──

    /// snap7 `opListBlocks`: params `00 01 12 04 11 43 01 00`, data `0A 00 00 00`; capture
    /// `BLOCKS_58` is byte-identical.
    #[test]
    fn list_all_request_matches_snap7_and_the_capture() {
        let captured = hex(BLOCKS_58);
        assert_eq!(build_list_all_request(pdu_ref(&captured)), captured);
    }

    /// `BLOCKS_59`: one OB, no FB, no FC, one DB, 12 SDBs, 71 SFCs and 14 SFBs.
    #[test]
    fn list_all_answer_from_the_capture() {
        let counts = parse_list_all(&hex(BLOCKS_59)).unwrap();
        assert_eq!(
            counts,
            [
                (BlockType::OB, 1),
                (BlockType::FB, 0),
                (BlockType::FC, 0),
                (BlockType::DB, 1),
                (BlockType::SDB, 12),
                (BlockType::SFC, 71),
                (BlockType::SFB, 14),
            ]
        );
    }

    /// snap7 `opListBlocksOfType`: first request params `00 01 12 04 11 43 02 00`, data
    /// `FF 09 00 02 30 <type>`; next request params `00 01 12 08 12 43 02 <seq> 00 00 00 00`,
    /// data `0A 00 00 00`. Captures `BLOCKS_61`, `BLOCKS_95` and `BLOCKS_98`.
    #[test]
    fn list_of_type_requests_match_snap7_and_the_capture() {
        let db = hex(BLOCKS_61);
        assert_eq!(build_list_of_type_request(pdu_ref(&db), BlockType::DB), db);
        let sfc = hex(BLOCKS_95);
        assert_eq!(build_list_of_type_request(pdu_ref(&sfc), BlockType::SFC), sfc);
        let next = hex(BLOCKS_98);
        assert_eq!(build_list_of_type_next_request(pdu_ref(&next), 1), next);
    }

    /// `BLOCKS_62`: DB 1, flag byte 0x22, language 5 (DB).
    #[test]
    fn list_of_type_answer_from_the_capture() {
        let page = parse_list_of_type(&hex(BLOCKS_62)).unwrap();
        assert_eq!(page.blocks, [ListedBlock { number: 1, flags: 0x22, language: 5 }]);
        assert!(!page.more);
    }

    /// `BLOCKS_96` and `BLOCKS_99`: the 71 SFCs in two parts, 52 and 19 blocks.
    #[test]
    fn list_of_type_in_several_parts_from_the_capture() {
        let first = parse_list_of_type(&hex(BLOCKS_96)).unwrap();
        assert!(first.more);
        assert_eq!(first.seq, 1, "the next request carries this sequence number");
        assert_eq!(first.blocks.len(), 52);
        assert_eq!(first.blocks[0], ListedBlock { number: 0, flags: 0x42, language: 1 });
        let last = parse_list_of_type(&hex(BLOCKS_99)).unwrap();
        assert!(!last.more);
        assert_eq!(last.blocks.len(), 19);
        let numbers: Vec<u16> = first.blocks.iter().chain(&last.blocks).map(|b| b.number).collect();
        assert_eq!(numbers.len(), 71);
        assert_eq!(numbers[0..6], [0, 1, 2, 3, 4, 5]);
        assert_eq!(*numbers.last().unwrap(), 0x7F);
    }

    #[test]
    fn language_names() {
        assert_eq!(language_name(1), "AWL");
        assert_eq!(language_name(5), "DB");
        assert_eq!(language_name(7), "SDB");
        assert_eq!(language_name(0x29), "ENCRYPT");
        assert_eq!(language_name(0x77), "0x77");
    }

    // ── Upload ──

    /// snap7 `opUpload`, start upload: function 0x1D, `Uk6[6]` and `Upload_ID` all zero, `Len_1`
    /// 9, then `_` `0` `<block type>` five digits `A` (TReqFunStartUploadParams, 18 bytes). The
    /// block type there is `Block_DB` (0x41), so DB 1 is `_0A00001A`.
    #[test]
    fn start_upload_request_is_the_snap7_template() {
        let request = build_start_upload_request(0x0102, BlockType::DB, 1);
        assert_eq!(
            request,
            hex("32 01 00 00 01 02 00 12 00 00  1d 00 00 00 00 00 00 00 09  5f 30 41 30 30 30 30 31 41")
        );
        assert_eq!(&request[19..], b"_0A00001A");
        let ob = build_start_upload_request(1, BlockType::OB, 100);
        assert_eq!(&ob[19..], b"_0800100A");
        let sdb = build_start_upload_request(1, BlockType::SDB, 65535);
        assert_eq!(&sdb[19..], b"_0B65535A");
    }

    /// The file name layout and the digits are those of the captured start download request
    /// `DOWNLOAD_43`, `_0A00001P`; only the file system letter differs.
    #[test]
    fn file_name_layout_matches_the_capture() {
        let captured = hex(DOWNLOAD_43);
        let name_in_capture = &captured[captured.windows(9).position(|w| w == b"_0A00001P").unwrap()..][..9];
        assert_eq!(name_in_capture, b"_0A00001P");
        assert_eq!(file_name(BlockType::DB, 1, b'P'), name_in_capture);
    }

    /// snap7 `TReqFunUploadParams` / `TReqFunEndUploadParams`: function, six zero bytes, upload id.
    #[test]
    fn upload_and_end_upload_requests_are_the_snap7_templates() {
        assert_eq!(build_upload_request(5, 0x2A), hex("32 01 00 00 00 05 00 08 00 00  1e 00 00 00 00 00 00 2a"));
        assert_eq!(build_end_upload_request(6, 0x2A), hex("32 01 00 00 00 06 00 08 00 00  1f 00 00 00 00 00 00 2a"));
        // The whole 4-byte id is echoed.
        assert_eq!(build_upload_request(5, 0x0102_0304)[10..], hex("1e 00 00 00 01 02 03 04")[..]);
    }

    /// Start upload answer, hand-assembled from snap7's `TResFunStartUploadParams` (function,
    /// 6 bytes, upload id, 3 + 5 bytes: the length string): ACK_DATA with 16 bytes of parameters.
    #[test]
    fn start_upload_answer_gives_the_upload_id() {
        let pdu = hex("32 03 00 00 00 04 00 10 00 00 00 00  1d 00 00 00 00 00 00 2a 07 30 30 30 30 35 30 30");
        assert_eq!(parse_start_upload_response(&pdu).unwrap(), 0x2A);
        assert_eq!(build_start_upload_response(4, 0x2A, 500), pdu);
        assert!(parse_start_upload_response(&hex("32 03 00 00 00 04 00 00 00 00 83 04")).unwrap_err().contains("0x8304"));
    }

    /// The three data packets of the block DB1 in `s7comm_downloading_block_db1.pcap`
    /// (`DOWNLOAD_48`, `DOWNLOAD_51`, `DOWNLOAD_54`) carry the block in load memory layout, split
    /// at 222, 222 and 56 bytes: `<length> 00 FB <payload>`. Wrapped as the PLC wraps an upload
    /// answer (parameters `1E <more>`, which the download uses as `1B <more>`), the client puts the
    /// block back together.
    #[test]
    fn upload_in_several_parts_gives_the_block_of_the_capture() {
        let captures = [DOWNLOAD_48, DOWNLOAD_51, DOWNLOAD_54].map(hex);
        let mut block = Vec::new();
        for (i, captured) in captures.iter().enumerate() {
            let payload = &captured[12 + 2 + 4..];
            let more = i + 1 < captures.len();
            // Same bytes as the capture, but for the function code.
            let mut expected = captured.clone();
            expected[12] = S7_UPLOAD;
            assert_eq!(build_upload_response(pdu_ref(captured), payload, more), expected, "part {i}");

            let part = parse_upload_response(&expected).unwrap();
            assert_eq!(part.more, more);
            block.extend_from_slice(part.payload);
        }
        assert_eq!(block.len(), 500);
        check_block(&block).unwrap();
        // Header: sub block type 0x0A (DB), block number 1, load memory size 500, MC7 length 400.
        assert_eq!(block[5], 0x0A);
        assert_eq!(u16::from_be_bytes([block[6], block[7]]), 1);
        assert_eq!(u32::from_be_bytes([block[8], block[9], block[10], block[11]]), 500);
        assert_eq!(u16::from_be_bytes([block[34], block[35]]), 400);
        // Footer: author, family and name of the block.
        assert_eq!(&block[block.len() - 36..block.len() - 28], b"TWI\0\0\0\0\0");
        assert_eq!(&block[block.len() - 28..block.len() - 20], b"TestHMI\0");
        assert_eq!(&block[block.len() - 20..block.len() - 12], b"DATADB\0\0");
    }

    #[test]
    fn a_truncated_block_is_rejected() {
        let captured = hex(DOWNLOAD_48);
        let first_part = &captured[18..];
        let error = check_block(first_part).unwrap_err();
        assert!(error.contains("of the 500 bytes"), "{error}");
        assert!(check_block(&[0; 10]).unwrap_err().contains("36-byte header"));
    }

    #[test]
    fn upload_answers_that_are_not_uploads_are_errors() {
        // A download answer (function 0x1B) is not an upload answer.
        assert!(parse_upload_response(&hex(DOWNLOAD_48)).unwrap_err().contains("unexpected"));
        // Error in the ACK_DATA header.
        let refused = hex("32 03 00 00 00 05 00 00 00 00 83 04");
        assert!(parse_upload_response(&refused).unwrap_err().contains("parallel upload"));
        // No data.
        assert!(parse_upload_response(&hex("32 03 00 00 00 05 00 02 00 00 00 00 1e 00")).is_err());
        assert!(parse_end_upload_response(&hex("32 03 00 00 00 06 00 01 00 00 00 00 1f")).is_ok());
        assert!(parse_end_upload_response(&hex("32 03 00 00 00 06 00 01 00 00 00 00 1e")).is_err());
    }

    // ── Delete ──

    /// snap7 `opDelete` (`TReqControlBlockParams`, 26 bytes): function 0x28, `Uk7` = 00 ×6, 0xFD,
    /// `Len_1` 10, one block, 0, `0`, block type, five digits, `SFun` 0x42, `Len_2` 5, `_DELE`.
    /// The `_INSE` telegram of the captured download (`DOWNLOAD_58`) has the same layout with
    /// `P` and `_INSE`.
    #[test]
    fn delete_request_is_the_snap7_template() {
        let request = build_delete_request(0x0102, BlockType::DB, 1);
        assert_eq!(
            request,
            hex("32 01 00 00 01 02 00 1a 00 00  28 00 00 00 00 00 00 fd 00 0a 01 00 30 41 30 30 30 30 31 42 05 5f 44 45 4c 45")
        );
        assert_eq!(request.len(), 10 + 26);
        let insert = hex(DOWNLOAD_58);
        let mut as_delete = insert.clone();
        as_delete[10 + 19] = FILE_SYSTEM_DELETE;
        as_delete[10 + 21..].copy_from_slice(b"_DELE");
        assert_eq!(build_delete_request(pdu_ref(&insert), BlockType::DB, 1), as_delete);
    }

    /// The answer to a PI service is an ACK_DATA with the parameter byte 0x28 (`DOWNLOAD_59`).
    #[test]
    fn delete_answer_from_the_capture() {
        let captured = hex(DOWNLOAD_59);
        assert!(parse_delete_response(&captured).is_ok());
        assert_eq!(build_delete_response(pdu_ref(&captured), Ok(())), captured);
        let password = parse_delete_response(&build_delete_response(1, Err(0xD241))).unwrap_err();
        assert!(password.contains("password"), "{password}");
        assert!(parse_delete_response(&hex("32 03 00 00 00 06 00 01 00 00 00 00 29")).is_err());
    }

    // ── The simulator ──

    #[test]
    fn simulator_requests_parse_back() {
        let start = build_start_upload_request(1, BlockType::FB, 12);
        assert_eq!(parse_start_upload_request(&start[10..]), Some((BlockType::FB, 12)));
        let delete = build_delete_request(1, BlockType::SFC, 20);
        assert_eq!(parse_delete_request(&delete[10..]), Some((BlockType::SFC, 20)));
        assert_eq!(parse_upload_session_request(&build_upload_request(1, 77)[10..]), Some(77));
        assert_eq!(parse_list_of_type_request_data(&[0xFF, 9, 0, 2, 0x30, 0x41]), Some(Ok(BlockType::DB)));
        assert_eq!(parse_list_of_type_request_data(&[0xFF, 9, 0, 2, 0x30, 0x39]), Some(Err(0x39)));
        // PI services other than _DELE of one block are not deletes.
        let mut stop = vec![0x28, 0, 0, 0, 0, 0, 0, 0xFD, 0, 0x0A, 1, 0, 0x30, 0x41, b'0', b'0', b'0', b'0', b'1', 0x42, 5];
        stop.extend_from_slice(b"_INSE");
        assert_eq!(parse_delete_request(&stop), None);
        assert_eq!(parse_delete_request(&hex("29 00 00 00 00 00 09 50 5f 50 52 4f 47 52 41 4d")), None);
    }

    #[test]
    fn simulator_lists_by_type() {
        let mut blocks = sim_fixed_blocks();
        blocks.push(sim_db(7, vec![1, 2, 3]));
        blocks.push(sim_db(2, vec![]));
        let counts = list_all_payload(&blocks);
        let framed = framing::build_response(1, GROUP_BLOCKS, SUBFUNCTION_LIST_ALL, &framing::Unit { seq: 0, data_unit_ref: 0, more: false }, &counts);
        let parsed = parse_list_all(&framed).unwrap();
        assert_eq!(parsed.len(), 7);
        assert!(parsed.contains(&(BlockType::DB, 2)));
        assert!(parsed.contains(&(BlockType::OB, 2)));
        assert!(parsed.contains(&(BlockType::SFB, 1)));

        let dbs = list_of_type_payload(&blocks, BlockType::DB).unwrap();
        assert_eq!(dbs, hex("00 02 22 05  00 07 22 05"));
        assert!(list_of_type_payload(&[], BlockType::DB).is_none());
    }

    #[test]
    fn simulator_block_has_the_load_memory_layout() {
        let block = encode_block(&sim_db(3, vec![9; 10]));
        assert_eq!(block.len(), 36 + 10 + 56);
        check_block(&block).unwrap();
        assert_eq!(block[..2], [0x70, 0x70]);
        assert_eq!(block[4], LANG_DB);
        assert_eq!(block[5], 0x0A);
        assert_eq!(u16::from_be_bytes([block[6], block[7]]), 3);
        assert_eq!(u32::from_be_bytes([block[8], block[9], block[10], block[11]]), 102);
        assert_eq!(u16::from_be_bytes([block[34], block[35]]), 10);
        assert_eq!(block[36..46], [9; 10]);
        // The same field offsets as the block of the capture.
        let captured = hex(DOWNLOAD_48);
        assert_eq!(block[..2], captured[18..20]);
        assert_eq!(block[4..8], [0x05, 0x0A, 0x00, 0x03]);
        assert_eq!(captured[18 + 4..18 + 8], [0x05, 0x0A, 0x00, 0x01]);
    }
}
