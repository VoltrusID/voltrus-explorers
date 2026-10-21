//! The simulator's side of SZL reads, block listing, upload and delete: what a connection
//! remembers between requests, and how the answers are cut into PDU-sized parts.
//!
//! A real CPU sends an answer that does not fit the negotiated PDU in several parts, the next one
//! when the client asks for it with the sequence number of the last part (see
//! [`framing`](crate::framing)). The content comes from [`szl`](crate::szl) and
//! [`blocks`](crate::blocks); the simulator's block catalogue comes from its image
//! (`server.rs`).

use std::collections::{HashMap, VecDeque};

use crate::blocks::{self, SimBlock};
use crate::framing::{
    self, Request, Unit, ACK_HEADER_LEN, ERR_BLOCK_NOT_FOUND, ERR_INFO_UNAVAILABLE, ERR_INVALID_BLOCK_TYPE,
    ERR_INVALID_SEQUENCE, GROUP_BLOCKS, GROUP_SZL, HEADER_LEN, UD_DATA_HEADER_LEN, UD_LONG_PARAMS_LEN,
};
use crate::szl;

/// Upload sessions a connection can have open; a real CPU allows a few and then answers 0x8304.
const MAX_UPLOADS: usize = 8;
/// Parameter error: no further parallel upload possible.
const ERR_NO_PARALLEL_UPLOAD: u16 = 0x8304;
/// Sequence number the simulator gives its answers to SZL reads and to block list requests of one
/// type (the PLC in the captures does the same; its answer to list-all has 0).
const SEQ: u8 = 1;

/// The rest of an answer that is being sent in parts.
struct Pending {
    seq: u8,
    data_unit_ref: u8,
    parts: VecDeque<Vec<u8>>,
}

/// Which of the two multi-part answers a request belongs to.
#[derive(Clone, Copy)]
enum Slot {
    Szl,
    BlockList,
}

/// An upload in progress: the block as it is sent, and how much of it is.
struct Upload {
    block: Vec<u8>,
    sent: usize,
}

/// What the simulator keeps per connection.
#[derive(Default)]
pub(crate) struct SimSession {
    szl: Option<Pending>,
    block_list: Option<Pending>,
    uploads: HashMap<u32, Upload>,
    last_upload_id: u32,
    transfers: u8,
}

/// Bytes of payload one user data answer carries in a PDU of `pdu_size`: the PDU less the S7
/// header, the 12 bytes of parameters and the 4-byte data header.
fn user_data_capacity(pdu_size: u16) -> usize {
    (pdu_size as usize).saturating_sub(HEADER_LEN + UD_LONG_PARAMS_LEN + UD_DATA_HEADER_LEN).max(1)
}

/// Cut `payload` into parts of at most `capacity` bytes that do not split a `unit`-byte item.
fn split(payload: &[u8], capacity: usize, unit: usize) -> VecDeque<Vec<u8>> {
    let size = (capacity / unit * unit).max(unit);
    let mut parts: VecDeque<Vec<u8>> = payload.chunks(size).map(<[u8]>::to_vec).collect();
    if parts.is_empty() {
        parts.push_back(Vec::new());
    }
    parts
}

impl SimSession {
    fn pending(&mut self, slot: Slot) -> &mut Option<Pending> {
        match slot {
            Slot::Szl => &mut self.szl,
            Slot::BlockList => &mut self.block_list,
        }
    }

    /// Answer a user data request (function group 3 or 4). `None` if it is not one.
    pub(crate) fn user_data(
        &mut self,
        pdu_ref: u16,
        params: &[u8],
        data: &[u8],
        blocks: &[SimBlock],
        pdu_size: u16,
    ) -> Option<Vec<u8>> {
        let request = framing::parse_request(params, data)?;
        Some(match (request.group, request.subfunction) {
            (GROUP_SZL, szl::SUBFUNCTION_READ) => self.build_szl_resp(pdu_ref, &request, pdu_size),
            (GROUP_BLOCKS, blocks::SUBFUNCTION_LIST_ALL | blocks::SUBFUNCTION_LIST_OF_TYPE) => {
                self.build_block_list_resp(pdu_ref, &request, blocks, pdu_size)
            }
            // Clock, block info and the rest: the CPU in the captures says "information function
            // unavailable" to what it does not offer.
            (group, subfunction) => {
                framing::build_error_response(pdu_ref, group, subfunction, request.seq, ERR_INFO_UNAVAILABLE)
            }
        })
    }

    /// An SZL read: the first request names the SZL, follow-up requests fetch the next parts.
    fn build_szl_resp(&mut self, pdu_ref: u16, request: &Request, pdu_size: u16) -> Vec<u8> {
        let (group, subfunction) = (GROUP_SZL, szl::SUBFUNCTION_READ);
        if request.follow_up {
            return self.next_part(pdu_ref, request, group, subfunction, Slot::Szl);
        }
        self.szl = None;
        let payload = szl::parse_first_request_data(request.data).and_then(|(id, index)| szl::sim_payload(id, index));
        match payload {
            Some(payload) => {
                let parts = split(&payload, user_data_capacity(pdu_size), 1);
                self.answer(pdu_ref, group, subfunction, SEQ, parts, Slot::Szl)
            }
            None => framing::build_error_response(pdu_ref, group, subfunction, SEQ, ERR_INFO_UNAVAILABLE),
        }
    }

    /// A block list request: all types with their counts, or the blocks of one type.
    fn build_block_list_resp(&mut self, pdu_ref: u16, request: &Request, blocks: &[SimBlock], pdu_size: u16) -> Vec<u8> {
        let group = GROUP_BLOCKS;
        if request.subfunction == blocks::SUBFUNCTION_LIST_ALL {
            let unit = Unit { seq: 0, data_unit_ref: 0, more: false };
            return framing::build_response(pdu_ref, group, request.subfunction, &unit, &blocks::list_all_payload(blocks));
        }
        let subfunction = blocks::SUBFUNCTION_LIST_OF_TYPE;
        if request.follow_up {
            return self.next_part(pdu_ref, request, group, subfunction, Slot::BlockList);
        }
        self.block_list = None;
        let error = |code| framing::build_error_response(pdu_ref, group, subfunction, SEQ, code);
        match blocks::parse_list_of_type_request_data(request.data) {
            None => error(ERR_INFO_UNAVAILABLE),
            Some(Err(_)) => error(ERR_INVALID_BLOCK_TYPE),
            Some(Ok(block_type)) => match blocks::list_of_type_payload(blocks, block_type) {
                None => error(ERR_BLOCK_NOT_FOUND),
                Some(payload) => {
                    let parts = split(&payload, user_data_capacity(pdu_size), 4);
                    self.answer(pdu_ref, group, subfunction, SEQ, parts, Slot::BlockList)
                }
            },
        }
    }

    /// Send the first of `parts` and remember the rest for the follow-up requests.
    fn answer(
        &mut self,
        pdu_ref: u16,
        group: u8,
        subfunction: u8,
        seq: u8,
        mut parts: VecDeque<Vec<u8>>,
        slot: Slot,
    ) -> Vec<u8> {
        let first = parts.pop_front().unwrap_or_default();
        let more = !parts.is_empty();
        let mut data_unit_ref = 0;
        if more {
            self.transfers = self.transfers.wrapping_add(1).max(1);
            data_unit_ref = self.transfers;
        }
        *self.pending(slot) = more.then_some(Pending { seq, data_unit_ref, parts });
        framing::build_response(pdu_ref, group, subfunction, &Unit { seq, data_unit_ref, more }, &first)
    }

    /// Answer a follow-up request with the next part, if there is one and the sequence number is
    /// that of the answer in progress.
    fn next_part(&mut self, pdu_ref: u16, request: &Request, group: u8, subfunction: u8, slot: Slot) -> Vec<u8> {
        let pending = self.pending(slot);
        let Some(transfer) = pending.as_mut().filter(|p| p.seq == request.seq) else {
            return framing::build_error_response(pdu_ref, group, subfunction, request.seq, ERR_INVALID_SEQUENCE);
        };
        let part = transfer.parts.pop_front().unwrap_or_default();
        let more = !transfer.parts.is_empty();
        let unit = Unit { seq: transfer.seq, data_unit_ref: transfer.data_unit_ref, more };
        if !more {
            *pending = None;
        }
        framing::build_response(pdu_ref, group, subfunction, &unit, &part)
    }

    /// *Start upload*: `block` is the block the request names, `None` if the PLC does not have it.
    pub(crate) fn start_upload(&mut self, pdu_ref: u16, block: Option<SimBlock>) -> Vec<u8> {
        let Some(block) = block else {
            return framing::ack_pdu(pdu_ref, ERR_BLOCK_NOT_FOUND, &[], &[]);
        };
        if self.uploads.len() >= MAX_UPLOADS {
            return framing::ack_pdu(pdu_ref, ERR_NO_PARALLEL_UPLOAD, &[], &[]);
        }
        let image = blocks::encode_block(&block);
        self.last_upload_id += 1;
        let upload_id = self.last_upload_id;
        let response = blocks::build_start_upload_response(pdu_ref, upload_id, image.len());
        self.uploads.insert(upload_id, Upload { block: image, sent: 0 });
        response
    }

    /// *Upload*: the next part of the block, as much as fits the PDU. An ACK_DATA carries 12
    /// bytes of header, 2 of parameters and 4 of data header around it.
    pub(crate) fn build_upload_resp(&mut self, pdu_ref: u16, params: &[u8], pdu_size: u16) -> Vec<u8> {
        let Some(upload) = blocks::parse_upload_session_request(params).and_then(|id| self.uploads.get_mut(&id)) else {
            return framing::ack_pdu(pdu_ref, ERR_INVALID_SEQUENCE, &[], &[]);
        };
        let capacity = (pdu_size as usize).saturating_sub(ACK_HEADER_LEN + 2 + UD_DATA_HEADER_LEN).max(1);
        let end = (upload.sent + capacity).min(upload.block.len());
        let part = &upload.block[upload.sent..end];
        upload.sent = end;
        blocks::build_upload_response(pdu_ref, part, end < upload.block.len())
    }

    /// *End upload*: forget the session.
    pub(crate) fn end_upload(&mut self, pdu_ref: u16, params: &[u8]) -> Vec<u8> {
        let known = blocks::parse_upload_session_request(params).and_then(|id| self.uploads.remove(&id));
        match known {
            Some(_) => blocks::build_end_upload_response(pdu_ref),
            None => framing::ack_pdu(pdu_ref, ERR_INVALID_SEQUENCE, &[], &[]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::sim_db;
    use crate::capture::*;
    use crate::types::BlockType;

    /// Run a user data request through the session.
    fn ask(session: &mut SimSession, request: Vec<u8>, blocks: &[SimBlock], pdu_size: u16) -> Vec<u8> {
        let param_len = u16::from_be_bytes([request[6], request[7]]) as usize;
        let (params, data) = request[10..].split_at(param_len);
        session.user_data(pdu_ref(&request), params, data, blocks, pdu_size).expect("a user data request")
    }

    /// SZL 0x001C at PDU size 240 is cut where the CPU of the capture cuts it: the first part
    /// carries 214 bytes (8 of SZL header and 206 of records), `STATUS_69`.
    #[test]
    fn szl_is_cut_into_parts_like_the_capture() {
        let mut session = SimSession::default();
        let first = ask(&mut session, szl::build_first_request(7, szl::COMPONENT_IDENTIFICATION, 0), &[], 240);
        let captured = hex(STATUS_69);
        assert_eq!(first.len(), captured.len());
        // Same lengths and parameter header as the capture's part; the data unit reference and
        // the sequence number are the CPU's own choice.
        assert_eq!(first[6..14], captured[6..14]);
        let response = framing::parse_response(&first, GROUP_SZL, szl::SUBFUNCTION_READ).unwrap();
        assert!(response.more);
        assert_eq!(response.payload.len(), 214);
        assert_eq!(response.seq, SEQ);

        let last = ask(&mut session, szl::build_next_request(8, response.seq), &[], 240);
        let response = framing::parse_response(&last, GROUP_SZL, szl::SUBFUNCTION_READ).unwrap();
        assert!(!response.more);
        assert_eq!(response.payload.len(), 8 + 7 * 34 - 214);
        // Nothing is left to ask for.
        let again = ask(&mut session, szl::build_next_request(9, response.seq), &[], 240);
        assert!(framing::parse_response(&again, GROUP_SZL, szl::SUBFUNCTION_READ).unwrap_err().contains("0x8401"));
    }

    #[test]
    fn a_follow_up_needs_the_sequence_number_of_the_answer() {
        let mut session = SimSession::default();
        ask(&mut session, szl::build_first_request(7, szl::COMPONENT_IDENTIFICATION, 0), &[], 240);
        let wrong = ask(&mut session, szl::build_next_request(8, 9), &[], 240);
        assert!(framing::parse_response(&wrong, GROUP_SZL, szl::SUBFUNCTION_READ).is_err());
        // The right one still works.
        let right = ask(&mut session, szl::build_next_request(9, SEQ), &[], 240);
        assert!(framing::parse_response(&right, GROUP_SZL, szl::SUBFUNCTION_READ).is_ok());
    }

    #[test]
    fn an_szl_the_cpu_does_not_have_is_refused_like_the_capture() {
        let mut session = SimSession::default();
        let captured = hex(STATUS_41);
        let answer = ask(&mut session, captured.clone(), &[], 240);
        assert_eq!(answer, hex(STATUS_42));
    }

    #[test]
    fn block_lists_are_cut_at_whole_items() {
        let blocks: Vec<SimBlock> = (1..=120).map(|n| sim_db(n, vec![])).collect();
        let mut session = SimSession::default();
        let mut numbers = Vec::new();
        let mut request = blocks::build_list_of_type_request(1, BlockType::DB);
        let mut parts = 0;
        loop {
            let answer = ask(&mut session, request, &blocks, 240);
            let page = blocks::parse_list_of_type(&answer).unwrap();
            numbers.extend(page.blocks.iter().map(|b| b.number));
            parts += 1;
            if !page.more {
                break;
            }
            request = blocks::build_list_of_type_next_request(1 + parts, page.seq);
        }
        // 214 bytes carry 53 items of 4 bytes.
        assert_eq!(parts, 3);
        assert_eq!(numbers, (1..=120).collect::<Vec<u16>>());
    }

    #[test]
    fn list_of_a_type_without_blocks_and_of_an_unknown_type_are_errors() {
        let mut session = SimSession::default();
        let blocks = [sim_db(1, vec![])];
        let none = ask(&mut session, blocks::build_list_of_type_request(1, BlockType::OB), &blocks, 240);
        assert!(blocks::parse_list_of_type(&none).unwrap_err().contains("0xD209"));
        let mut unknown = blocks::build_list_of_type_request(1, BlockType::OB);
        let last = unknown.len() - 1;
        unknown[last] = 0x39;
        let answer = ask(&mut session, unknown, &blocks, 240);
        assert!(blocks::parse_list_of_type(&answer).unwrap_err().contains("0x0113"));
    }

    #[test]
    fn unsupported_user_data_functions_are_refused() {
        let mut session = SimSession::default();
        // Clock (group 7), read.
        let request = framing::build_request(1, 0x07, 0x01, 0, false, &[0x0A, 0, 0, 0]);
        let answer = ask(&mut session, request, &[], 240);
        let error = framing::parse_response(&answer, 0x07, 0x01).err().unwrap();
        assert!(error.contains("0xD402"), "{error}");
        assert!(session.user_data(1, &[1, 2, 3], &[], &[], 240).is_none());
    }

    #[test]
    fn upload_is_sent_in_pdu_sized_parts() {
        let mut session = SimSession::default();
        let block = sim_db(1, vec![0xAB; 400]);
        let start = session.start_upload(1, Some(block.clone()));
        let id = blocks::parse_start_upload_response(&start).unwrap();

        let mut received = Vec::new();
        let mut sizes = Vec::new();
        loop {
            let answer = session.build_upload_resp(2, &blocks::build_upload_request(2, id)[10..], 240);
            let part = blocks::parse_upload_response(&answer).unwrap();
            sizes.push(part.payload.len());
            received.extend_from_slice(part.payload);
            if !part.more {
                break;
            }
        }
        // 36 + 400 + 56 = 492 bytes, 222 per PDU of 240 (the capture's part size).
        assert_eq!(sizes, [222, 222, 48]);
        assert_eq!(received, blocks::encode_block(&block));
        let end = session.end_upload(3, &blocks::build_end_upload_request(3, id)[10..]);
        blocks::parse_end_upload_response(&end).unwrap();
        // The session is gone.
        let again = session.build_upload_resp(4, &blocks::build_upload_request(4, id)[10..], 240);
        assert!(blocks::parse_upload_response(&again).unwrap_err().contains("0x8401"));
    }

    #[test]
    fn upload_of_a_missing_block_and_too_many_uploads_are_refused() {
        let mut session = SimSession::default();
        let missing = session.start_upload(1, None);
        assert!(blocks::parse_start_upload_response(&missing).unwrap_err().contains("0xD209"));
        for _ in 0..MAX_UPLOADS {
            let start = session.start_upload(1, Some(sim_db(1, vec![])));
            blocks::parse_start_upload_response(&start).unwrap();
        }
        let refused = session.start_upload(1, Some(sim_db(1, vec![])));
        assert!(blocks::parse_start_upload_response(&refused).unwrap_err().contains("parallel upload"));
    }

}
