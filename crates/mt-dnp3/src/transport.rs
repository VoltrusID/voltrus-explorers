#![allow(dead_code)]
#![allow(clippy::ptr_arg)]

/// DNP3 transport header (1 byte: FIR | FIN | SEQUENCE).
#[derive(Debug, Clone, PartialEq)]
pub struct TransportHeader {
    pub fir: bool,
    pub fin: bool,
    pub sequence: u8,
}

impl TransportHeader {
    pub fn to_byte(&self) -> u8 {
        let fir = if self.fir { 0x80 } else { 0x00 };
        let fin = if self.fin { 0x40 } else { 0x00 };
        fir | fin | (self.sequence & 0x3F)
    }

    pub fn from_byte(byte: u8) -> Self {
        Self {
            fir: byte & 0x80 != 0,
            fin: byte & 0x40 != 0,
            sequence: byte & 0x3F,
        }
    }
}

/// A single transport segment extracted from a data link frame.
#[derive(Debug, Clone)]
pub struct TransportSegment {
    pub fir: bool,
    pub fin: bool,
    pub sequence: u8,
    pub data: Vec<u8>,
}

/// Maximum payload bytes per transport segment. The link layer allows 255
/// bytes total for the user-data block, of which 5 are the link header and
/// 1 is the transport header, leaving 249 bytes of ASDU per frame.
pub const MAX_SEGMENT_PAYLOAD: usize = 249;

/// Split an ASDU into transport segments of at most [`MAX_SEGMENT_PAYLOAD`]
/// bytes, assigning FIR/FIN/sequence per DNP3 transport rules.
pub fn segment_asdu(asdu: &[u8]) -> Vec<TransportSegment> {
    let total = asdu.len().div_ceil(MAX_SEGMENT_PAYLOAD);
    let mut out = Vec::with_capacity(total);
    for (i, chunk) in asdu.chunks(MAX_SEGMENT_PAYLOAD).enumerate() {
        out.push(TransportSegment {
            fir: i == 0,
            fin: i + 1 == total,
            sequence: (i as u8) & 0x3F,
            data: chunk.to_vec(),
        });
    }
    out
}

/// Reassembles multi-segment ASDUs from transport segments.
#[derive(Debug)]
pub struct TransportReassembler {
    buffer: Vec<u8>,
    expected_seq: u8,
    active: bool,
}

impl TransportReassembler {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            expected_seq: 0,
            active: false,
        }
    }

    /// Process a transport segment. Returns complete ASDU data when FIN=true.
    pub fn process(&mut self, segment: TransportSegment) -> Option<Vec<u8>> {
        if segment.fir {
            self.buffer.clear();
            self.active = true;
            self.expected_seq = segment.sequence;
        }

        if !self.active {
            return None;
        }

        if segment.sequence != self.expected_seq {
            tracing::warn!(
                "Transport sequence mismatch: expected {}, got {}",
                self.expected_seq,
                segment.sequence
            );
            self.buffer.clear();
            self.active = false;
            return None;
        }

        self.buffer.extend_from_slice(&segment.data);
        self.expected_seq = (self.expected_seq + 1) & 0x3F;

        if segment.fin {
            let result = self.buffer.clone();
            self.buffer.clear();
            self.active = false;
            return Some(result);
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_segment() {
        let mut reassembler = TransportReassembler::new();
        let segment = TransportSegment {
            fir: true,
            fin: true,
            sequence: 0,
            data: vec![0x01, 0x02, 0x03],
        };
        let result = reassembler.process(segment);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), vec![0x01, 0x02, 0x03]);
    }

    #[test]
    fn test_two_segments() {
        let mut reassembler = TransportReassembler::new();
        let seg1 = TransportSegment {
            fir: true,
            fin: false,
            sequence: 0,
            data: vec![0x01, 0x02],
        };
        let seg2 = TransportSegment {
            fir: false,
            fin: true,
            sequence: 1,
            data: vec![0x03, 0x04],
        };
        assert!(reassembler.process(seg1).is_none());
        let result = reassembler.process(seg2);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), vec![0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn test_three_segments() {
        let mut reassembler = TransportReassembler::new();
        let seg1 = TransportSegment {
            fir: true,
            fin: false,
            sequence: 0,
            data: vec![0x01],
        };
        let seg2 = TransportSegment {
            fir: false,
            fin: false,
            sequence: 1,
            data: vec![0x02],
        };
        let seg3 = TransportSegment {
            fir: false,
            fin: true,
            sequence: 2,
            data: vec![0x03],
        };
        assert!(reassembler.process(seg1).is_none());
        assert!(reassembler.process(seg2).is_none());
        let result = reassembler.process(seg3);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), vec![0x01, 0x02, 0x03]);
    }

    #[test]
    fn test_new_first_segment_resets() {
        let mut reassembler = TransportReassembler::new();
        let seg1 = TransportSegment {
            fir: true,
            fin: false,
            sequence: 0,
            data: vec![0x01],
        };
        reassembler.process(seg1);
        let seg_new = TransportSegment {
            fir: true,
            fin: true,
            sequence: 5,
            data: vec![0x99],
        };
        let result = reassembler.process(seg_new);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), vec![0x99]);
    }

    #[test]
    fn test_transport_header_encode() {
        let header = TransportHeader {
            fir: true,
            fin: true,
            sequence: 3,
        };
        let byte = header.to_byte();
        assert_eq!(byte, 0xC3);
    }

    #[test]
    fn test_transport_header_decode() {
        let header = TransportHeader::from_byte(0x43);
        assert!(!header.fir);
        assert!(header.fin);
        assert_eq!(header.sequence, 3);
    }
}
