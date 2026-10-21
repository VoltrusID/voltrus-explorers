//! Wire-level codecs for the simulated device: ASN.1 tags and primitive
//! values (ASHRAE 135 clause 20.2), NPDU (clause 6.2), BVLC (Annex J) and PDU
//! framing (clause 20.1). Everything is hand-rolled so the simulator's bytes do
//! not depend on how a client library happens to encode them.

use std::net::SocketAddr;

use bacnet_rs::object::ObjectIdentifier;

// ── BVLC function codes (Annex J.2) ──
pub(super) const BVLC_RESULT: u8 = 0x00;
pub(super) const BVLC_WRITE_BDT: u8 = 0x01;
pub(super) const BVLC_READ_BDT: u8 = 0x02;
pub(super) const BVLC_REGISTER_FOREIGN_DEVICE: u8 = 0x05;
pub(super) const BVLC_READ_FDT: u8 = 0x06;
pub(super) const BVLC_DELETE_FDT_ENTRY: u8 = 0x08;
pub(super) const BVLC_DISTRIBUTE_BROADCAST_TO_NETWORK: u8 = 0x09;
pub(super) const BVLC_FORWARDED_NPDU: u8 = 0x04;
pub(super) const BVLC_ORIGINAL_UNICAST: u8 = 0x0A;
pub(super) const BVLC_ORIGINAL_BROADCAST: u8 = 0x0B;

// ── PDU types (clause 20.1, high nibble of the first APDU octet) ──
pub(super) const PDU_CONFIRMED_REQUEST: u8 = 0;
pub(super) const PDU_UNCONFIRMED_REQUEST: u8 = 1;
pub(super) const PDU_SIMPLE_ACK: u8 = 2;
pub(super) const PDU_ERROR: u8 = 5;
pub(super) const PDU_REJECT: u8 = 6;
pub(super) const PDU_ABORT: u8 = 7;

// ── Reject reasons (BACnetRejectReason, clause 18.8 / 21) ──
pub(super) const REJ_INVALID_PARAMETER_DATA_TYPE: u8 = 3;
pub(super) const REJ_INVALID_TAG: u8 = 4;
pub(super) const REJ_MISSING_REQUIRED_PARAMETER: u8 = 5;
pub(super) const REJ_PARAMETER_OUT_OF_RANGE: u8 = 6;
pub(super) const REJ_TOO_MANY_ARGUMENTS: u8 = 7;
pub(super) const REJ_UNRECOGNIZED_SERVICE: u8 = 9;

// ── Abort reasons (BACnetAbortReason, clause 18.9 / 21) ──
pub(super) const ABORT_SEGMENTATION_NOT_SUPPORTED: u8 = 4;

/// Largest APDU the simulator accepts and advertises (max-APDU code 5).
pub(super) const MAX_APDU_ACCEPTED: usize = 1476;

/// BACnet Error PDU class/code pair (clause 18.1 / 21 BACnetErrorClass, BACnetErrorCode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SimError {
    pub class: u32,
    pub code: u32,
}

pub(super) const ERR_UNKNOWN_OBJECT: SimError = SimError { class: 1, code: 31 }; // object / unknown-object
pub(super) const ERR_UNKNOWN_PROPERTY: SimError = SimError { class: 2, code: 32 }; // property / unknown-property
pub(super) const ERR_WRITE_ACCESS_DENIED: SimError = SimError { class: 2, code: 40 }; // property / write-access-denied
pub(super) const ERR_INVALID_DATA_TYPE: SimError = SimError { class: 2, code: 9 }; // property / invalid-data-type
pub(super) const ERR_VALUE_OUT_OF_RANGE: SimError = SimError { class: 2, code: 37 }; // property / value-out-of-range
pub(super) const ERR_INVALID_ARRAY_INDEX: SimError = SimError { class: 2, code: 42 }; // property / invalid-array-index
pub(super) const ERR_NOT_AN_ARRAY: SimError = SimError { class: 2, code: 50 }; // property / property-is-not-an-array
pub(super) const ERR_SERVICE_REQUEST_DENIED: SimError = SimError { class: 5, code: 29 }; // services / service-request-denied

// ── Network addressing ──

/// A BACnet network address as carried in the NPDU (network number + MAC address).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NetAddr {
    pub net: u16,
    pub adr: Vec<u8>,
}

/// Where a request came from, and so where its reply must go: the BACnet/IP
/// address to send to plus the remote-network address (SNET/SADR) the request
/// carried when it was routed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Peer {
    pub addr: SocketAddr,
    pub net: Option<NetAddr>,
}

/// A datagram the device loop must transmit.
#[derive(Debug)]
pub(super) struct Reply {
    pub to: SocketAddr,
    pub frame: Vec<u8>,
}

/// Decoded NPDU header (clause 6.2).
#[derive(Debug)]
pub(super) struct NpduHeader {
    pub network_message: bool,
    pub priority: u8,
    pub dest: Option<NetAddr>,
    pub source: Option<NetAddr>,
    /// Octets the header occupies (the APDU / network message follows).
    pub len: usize,
}

fn read_net_addr(b: &[u8], pos: usize) -> Option<(NetAddr, usize)> {
    let net = u16::from_be_bytes([*b.get(pos)?, *b.get(pos + 1)?]);
    let len = usize::from(*b.get(pos + 2)?);
    let adr = b.get(pos + 3..pos + 3 + len)?.to_vec();
    Some((NetAddr { net, adr }, pos + 3 + len))
}

/// Parse an NPDU header (version 1 only). Returns `None` for truncated or
/// invalid headers (wrong version, SLEN of zero).
pub(super) fn parse_npdu(b: &[u8]) -> Option<NpduHeader> {
    if *b.first()? != 0x01 {
        return None;
    }
    let control = *b.get(1)?;
    let mut pos = 2;
    let dest = if control & 0x20 != 0 {
        let (a, p) = read_net_addr(b, pos)?;
        pos = p;
        Some(a)
    } else {
        None
    };
    let source = if control & 0x08 != 0 {
        let (a, p) = read_net_addr(b, pos)?;
        if a.adr.is_empty() {
            return None; // SLEN = 0 is not a valid source address
        }
        pos = p;
        Some(a)
    } else {
        None
    };
    if dest.is_some() {
        b.get(pos)?; // hop count
        pos += 1;
    }
    Some(NpduHeader {
        network_message: control & 0x80 != 0,
        priority: control & 0x03,
        dest,
        source,
        len: pos,
    })
}

/// Build the NPDU for a reply or a notification. `dest` is the remote network
/// address (DNET/DADR) when the peer sits behind a router.
pub(super) fn npdu_bytes(expecting_reply: bool, priority: u8, dest: Option<&NetAddr>) -> Vec<u8> {
    let mut control: u8 = priority & 0x03;
    if dest.is_some() {
        control |= 0x20;
    }
    if expecting_reply {
        control |= 0x04;
    }
    let mut v = vec![0x01, control];
    if let Some(d) = dest {
        v.extend_from_slice(&d.net.to_be_bytes());
        v.push(d.adr.len() as u8);
        v.extend_from_slice(&d.adr);
        v.push(0xFF); // hop count
    }
    v
}

fn bvlc_frame(function: u8, payload: &[u8]) -> Vec<u8> {
    let len = (4 + payload.len()) as u16;
    let mut f = vec![0x81, function];
    f.extend_from_slice(&len.to_be_bytes());
    f.extend_from_slice(payload);
    f
}

/// BVLC Original-Unicast-NPDU carrying `apdu` to `peer`.
pub(super) fn frame_to(peer: &Peer, apdu: &[u8], expecting_reply: bool, priority: u8) -> Vec<u8> {
    let mut payload = npdu_bytes(expecting_reply, priority, peer.net.as_ref());
    payload.extend_from_slice(apdu);
    bvlc_frame(BVLC_ORIGINAL_UNICAST, &payload)
}

/// BVLC-Result with the given result code (Annex J.2.1).
pub(super) fn bvlc_result(code: u16) -> Vec<u8> {
    bvlc_frame(BVLC_RESULT, &code.to_be_bytes())
}

// ── PDU encoders (clause 20.1) ──

pub(super) fn simple_ack_pdu(invoke_id: u8, service: u8) -> Vec<u8> {
    vec![PDU_SIMPLE_ACK << 4, invoke_id, service]
}

/// Unsegmented ComplexAck.
pub(super) fn complex_ack_pdu(invoke_id: u8, service: u8, data: &[u8]) -> Vec<u8> {
    let mut v = vec![0x30, invoke_id, service];
    v.extend_from_slice(data);
    v
}

pub(super) fn error_pdu(invoke_id: u8, service: u8, e: SimError) -> Vec<u8> {
    let mut v = vec![PDU_ERROR << 4, invoke_id, service];
    Val::Enum(e.class).encode(&mut v);
    Val::Enum(e.code).encode(&mut v);
    v
}

pub(super) fn reject_pdu(invoke_id: u8, reason: u8) -> Vec<u8> {
    vec![PDU_REJECT << 4, invoke_id, reason]
}

/// Abort sent by the server side of the transaction (SRV bit set).
pub(super) fn abort_pdu(invoke_id: u8, reason: u8) -> Vec<u8> {
    vec![(PDU_ABORT << 4) | 0x01, invoke_id, reason]
}

// ── ASN.1 encoding ──

/// Big-endian bytes of `v` with leading zero octets removed (at least one octet).
fn be_min(v: u64) -> ([u8; 8], usize) {
    let n = ((64 - v.leading_zeros() as usize + 7) / 8).max(1);
    (v.to_be_bytes(), n)
}

/// Tag header (clause 20.2.1): tag number, class and content length.
pub(super) fn put_tag(out: &mut Vec<u8>, number: u32, context: bool, len: usize) {
    let mut b = if context { 0x08 } else { 0x00 };
    b |= if number < 15 { (number as u8) << 4 } else { 0xF0 };
    b |= if len < 5 { len as u8 } else { 5 };
    out.push(b);
    if number >= 15 {
        out.push(number as u8);
    }
    if len >= 5 {
        if len < 254 {
            out.push(len as u8);
        } else if len < 65536 {
            out.push(254);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            out.push(255);
            out.extend_from_slice(&(len as u32).to_be_bytes());
        }
    }
}

pub(super) fn put_open(out: &mut Vec<u8>, number: u32) {
    if number < 15 {
        out.push(((number as u8) << 4) | 0x0E);
    } else {
        out.extend_from_slice(&[0xFE, number as u8]);
    }
}

pub(super) fn put_close(out: &mut Vec<u8>, number: u32) {
    if number < 15 {
        out.push(((number as u8) << 4) | 0x0F);
    } else {
        out.extend_from_slice(&[0xFF, number as u8]);
    }
}

/// Context-tagged Unsigned (also used for context-tagged Enumerated, same encoding).
pub(super) fn ctx_unsigned(out: &mut Vec<u8>, number: u32, v: u64) {
    let (b, n) = be_min(v);
    put_tag(out, number, true, n);
    out.extend_from_slice(&b[8 - n..]);
}

pub(super) fn oid_u32(oid: ObjectIdentifier) -> u32 {
    (u32::from(oid.object_type) << 22) | (oid.instance & 0x003F_FFFF)
}

pub(super) fn ctx_object_id(out: &mut Vec<u8>, number: u32, oid: ObjectIdentifier) {
    put_tag(out, number, true, 4);
    out.extend_from_slice(&oid_u32(oid).to_be_bytes());
}

/// An application-tagged BACnet primitive value.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Val {
    Null,
    Bool(bool),
    Unsigned(u64),
    Real(f32),
    Enum(u32),
    Str(String),
    /// Bit string, first element = bit 0 (transmitted as the most significant bit of octet 0).
    Bits(Vec<bool>),
    Oid(ObjectIdentifier),
}

impl Val {
    pub(super) fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Val::Null => out.push(0x00),
            // Application Boolean: value lives in the L/V/T field, no content octets.
            Val::Bool(b) => out.push(0x10 | u8::from(*b)),
            Val::Unsigned(v) => {
                let (b, n) = be_min(*v);
                put_tag(out, 2, false, n);
                out.extend_from_slice(&b[8 - n..]);
            }
            Val::Real(f) => {
                put_tag(out, 4, false, 4);
                out.extend_from_slice(&f.to_be_bytes());
            }
            Val::Enum(v) => {
                let (b, n) = be_min(u64::from(*v));
                put_tag(out, 9, false, n);
                out.extend_from_slice(&b[8 - n..]);
            }
            Val::Str(s) => {
                put_tag(out, 7, false, s.len() + 1);
                out.push(0x00); // character set 0 = UTF-8
                out.extend_from_slice(s.as_bytes());
            }
            Val::Bits(bits) => {
                let nbytes = (bits.len() + 7) / 8;
                put_tag(out, 8, false, nbytes + 1);
                out.push((nbytes * 8 - bits.len()) as u8); // unused bits in the last octet
                let mut bytes = vec![0u8; nbytes];
                for (i, set) in bits.iter().enumerate() {
                    if *set {
                        bytes[i / 8] |= 0x80 >> (i % 8);
                    }
                }
                out.extend_from_slice(&bytes);
            }
            Val::Oid(oid) => {
                put_tag(out, 12, false, 4);
                out.extend_from_slice(&oid_u32(*oid).to_be_bytes());
            }
        }
    }
}

// ── ASN.1 decoding ──

/// A decode failure, as the BACnetRejectReason the request deserves.
pub(super) type Decoded<T> = Result<T, u8>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TagKind {
    /// Primitive value with this many content octets.
    Primitive(usize),
    Open,
    Close,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Tag {
    pub number: u32,
    pub context: bool,
    pub kind: TagKind,
    pub header_len: usize,
}

/// Cursor over ASN.1-encoded request parameters.
pub(super) struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(super) fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.pos >= self.buf.len()
    }

    /// Look at the next tag without consuming it; `None` at the end of the data.
    pub(super) fn peek(&self) -> Decoded<Option<Tag>> {
        let b = &self.buf[self.pos.min(self.buf.len())..];
        let Some(&first) = b.first() else { return Ok(None) };
        let context = first & 0x08 != 0;
        let lvt = first & 0x07;
        let mut number = u32::from(first >> 4);
        let mut hdr = 1usize;
        if number == 0x0F {
            number = u32::from(*b.get(1).ok_or(REJ_INVALID_TAG)?);
            hdr = 2;
        }
        if context && (lvt == 6 || lvt == 7) {
            let kind = if lvt == 6 { TagKind::Open } else { TagKind::Close };
            return Ok(Some(Tag { number, context, kind, header_len: hdr }));
        }
        if !context && lvt >= 6 {
            return Err(REJ_INVALID_TAG);
        }
        let len = if !context && number == 1 {
            0 // application Boolean carries its value in the L/V/T bits
        } else if lvt < 5 {
            usize::from(lvt)
        } else {
            let l0 = *b.get(hdr).ok_or(REJ_INVALID_TAG)?;
            match l0 {
                254 => {
                    let s = b.get(hdr + 1..hdr + 3).ok_or(REJ_INVALID_TAG)?;
                    hdr += 3;
                    usize::from(u16::from_be_bytes([s[0], s[1]]))
                }
                255 => {
                    let s = b.get(hdr + 1..hdr + 5).ok_or(REJ_INVALID_TAG)?;
                    hdr += 5;
                    u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize
                }
                n => {
                    hdr += 1;
                    usize::from(n)
                }
            }
        };
        if b.len() < hdr + len {
            return Err(REJ_INVALID_TAG);
        }
        Ok(Some(Tag { number, context, kind: TagKind::Primitive(len), header_len: hdr }))
    }

    /// Consume the context-tagged primitive `number` and return its content octets.
    /// `Ok(None)` when the next tag is something else (or the data ended).
    fn take_ctx(&mut self, number: u32) -> Decoded<Option<&'a [u8]>> {
        match self.peek()? {
            Some(Tag { number: n, context: true, kind: TagKind::Primitive(len), header_len }) if n == number => {
                let start = self.pos + header_len;
                self.pos = start + len;
                Ok(Some(&self.buf[start..start + len]))
            }
            _ => Ok(None),
        }
    }

    fn require_ctx(&mut self, number: u32) -> Decoded<&'a [u8]> {
        match self.take_ctx(number)? {
            Some(c) => Ok(c),
            None if self.is_empty() => Err(REJ_MISSING_REQUIRED_PARAMETER),
            None => Err(REJ_INVALID_TAG),
        }
    }

    pub(super) fn ctx_unsigned(&mut self, number: u32) -> Decoded<u32> {
        be_unsigned(self.require_ctx(number)?)
    }

    pub(super) fn opt_ctx_unsigned(&mut self, number: u32) -> Decoded<Option<u32>> {
        self.take_ctx(number)?.map(be_unsigned).transpose()
    }

    pub(super) fn ctx_object_id(&mut self, number: u32) -> Decoded<ObjectIdentifier> {
        let c = self.require_ctx(number)?;
        let raw: [u8; 4] = c.try_into().map_err(|_| REJ_INVALID_PARAMETER_DATA_TYPE)?;
        Ok(ObjectIdentifier::from(u32::from_be_bytes(raw)))
    }

    /// Optional context-tagged Boolean: one content octet, 0 or 1 (clause 20.2.3).
    pub(super) fn opt_ctx_bool(&mut self, number: u32) -> Decoded<Option<bool>> {
        match self.take_ctx(number)? {
            None => Ok(None),
            Some([0]) => Ok(Some(false)),
            Some([1]) => Ok(Some(true)),
            Some(_) => Err(REJ_INVALID_PARAMETER_DATA_TYPE),
        }
    }

    /// Consume the opening tag `number`.
    pub(super) fn open(&mut self, number: u32) -> Decoded<()> {
        match self.peek()? {
            Some(Tag { number: n, context: true, kind: TagKind::Open, header_len }) if n == number => {
                self.pos += header_len;
                Ok(())
            }
            Some(_) => Err(REJ_INVALID_TAG),
            None => Err(REJ_MISSING_REQUIRED_PARAMETER),
        }
    }

    /// Consume the closing tag `number`.
    pub(super) fn close(&mut self, number: u32) -> Decoded<()> {
        match self.peek()? {
            Some(Tag { number: n, context: true, kind: TagKind::Close, header_len }) if n == number => {
                self.pos += header_len;
                Ok(())
            }
            Some(_) => Err(REJ_INVALID_TAG),
            None => Err(REJ_MISSING_REQUIRED_PARAMETER),
        }
    }

    /// Is the next tag the closing tag `number`? Running out of data is an error.
    pub(super) fn at_close(&self, number: u32) -> Decoded<bool> {
        match self.peek()? {
            Some(t) => Ok(t.context && t.kind == TagKind::Close && t.number == number),
            None => Err(REJ_MISSING_REQUIRED_PARAMETER),
        }
    }

    /// After an opening tag has been consumed: return the raw octets up to the
    /// matching closing tag (nesting aware) and consume that closing tag.
    pub(super) fn capture_constructed(&mut self) -> Decoded<&'a [u8]> {
        let start = self.pos;
        let mut depth = 0usize;
        loop {
            let Some(t) = self.peek()? else { return Err(REJ_MISSING_REQUIRED_PARAMETER) };
            match t.kind {
                TagKind::Open => {
                    depth += 1;
                    self.pos += t.header_len;
                }
                TagKind::Close if depth == 0 => {
                    let value = &self.buf[start..self.pos];
                    self.pos += t.header_len;
                    return Ok(value);
                }
                TagKind::Close => {
                    depth -= 1;
                    self.pos += t.header_len;
                }
                TagKind::Primitive(len) => self.pos += t.header_len + len,
            }
        }
    }

    /// The request must be fully consumed.
    pub(super) fn finish(&self) -> Decoded<()> {
        if self.is_empty() { Ok(()) } else { Err(REJ_TOO_MANY_ARGUMENTS) }
    }
}

/// Unsigned content octets (1..=4) → value.
fn be_unsigned(c: &[u8]) -> Decoded<u32> {
    if c.is_empty() || c.len() > 4 {
        return Err(REJ_INVALID_PARAMETER_DATA_TYPE);
    }
    Ok(c.iter().fold(0u32, |acc, &b| (acc << 8) | u32::from(b)))
}

/// Decode a buffer holding exactly one application-tagged primitive value:
/// `(application tag number, content octets)`. Context/constructed data,
/// trailing octets and malformed tags yield `None`. A Boolean returns its
/// value as a single content octet.
pub(super) fn single_app_value(b: &[u8]) -> Option<(u32, Vec<u8>)> {
    let r = Reader::new(b);
    let t = r.peek().ok()??;
    let TagKind::Primitive(len) = t.kind else { return None };
    if t.context || t.header_len + len != b.len() {
        return None;
    }
    if t.number == 1 {
        return Some((1, vec![b[0] & 0x07]));
    }
    Some((t.number, b[t.header_len..].to_vec()))
}
