//! Reads `original_application_version` (receipt field 19) from the App Store receipt that ships with
//! every store install. The receipt is a PKCS#7 SignedData container (BER, often with indefinite lengths)
//! whose content is a SET of `ReceiptAttribute ::= SEQUENCE { type INTEGER, version INTEGER, value OCTET STRING }`.
//!
//! The signature is not verified: the result only decides whether a buyer of the formerly paid app keeps
//! Pro, and the code is open source anyway.

use std::path::PathBuf;

const ORIGINAL_APPLICATION_VERSION: u64 = 19;

const TAG_INTEGER: u8 = 0x02;
const TAG_OCTET_STRING: u8 = 0x04;
const TAG_OCTET_STRING_CONSTRUCTED: u8 = 0x24;
const TAG_UTF8_STRING: u8 = 0x0c;
const TAG_IA5_STRING: u8 = 0x16;
const TAG_SEQUENCE: u8 = 0x30;
const TAG_SET: u8 = 0x31;
const TAG_CONTEXT_0: u8 = 0xa0;

#[derive(Debug, Clone, Copy)]
struct Tlv {
    tag: u8,
    start: usize,
    end: usize,
    next: usize,
}

fn read_tlv(buf: &[u8], pos: usize) -> Option<Tlv> {
    let tag = *buf.get(pos)?;
    if tag & 0x1f == 0x1f {
        return None; // multi-byte tags never appear in receipts
    }
    let first = *buf.get(pos + 1)? as usize;
    let mut start = pos + 2;
    if first == 0x80 {
        // Indefinite length: children until the end-of-contents marker 00 00.
        let mut p = start;
        loop {
            if buf.get(p)? == &0 && buf.get(p + 1)? == &0 {
                return Some(Tlv { tag, start, end: p, next: p + 2 });
            }
            p = read_tlv(buf, p)?.next;
        }
    }
    let len = if first < 0x80 {
        first
    } else {
        let n = first & 0x7f;
        if n > 4 {
            return None;
        }
        let mut len = 0usize;
        for i in 0..n {
            len = (len << 8) | *buf.get(start + i)? as usize;
        }
        start += n;
        len
    };
    let end = start.checked_add(len)?;
    (end <= buf.len()).then_some(Tlv { tag, start, end, next: end })
}

fn children(buf: &[u8], parent: Tlv) -> Option<Vec<Tlv>> {
    let mut out = Vec::new();
    let mut p = parent.start;
    while p < parent.end {
        let t = read_tlv(buf, p)?;
        p = t.next;
        out.push(t);
    }
    Some(out)
}

fn octets(buf: &[u8], t: Tlv) -> Option<Vec<u8>> {
    match t.tag {
        TAG_OCTET_STRING => Some(buf[t.start..t.end].to_vec()),
        TAG_OCTET_STRING_CONSTRUCTED => {
            let mut out = Vec::new();
            for c in children(buf, t)? {
                out.extend(octets(buf, c)?);
            }
            Some(out)
        }
        _ => None,
    }
}

fn uint(buf: &[u8], t: Tlv) -> Option<u64> {
    if t.tag != TAG_INTEGER || t.end - t.start > 8 {
        return None;
    }
    Some(buf[t.start..t.end].iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b)))
}

/// Receipt payload (the SET of attributes) inside the PKCS#7 container.
fn payload(receipt: &[u8]) -> Option<Vec<u8>> {
    let content_info = read_tlv(receipt, 0).filter(|t| t.tag == TAG_SEQUENCE)?;
    let explicit = *children(receipt, content_info)?.get(1).filter(|t| t.tag == TAG_CONTEXT_0)?;
    let signed_data = *children(receipt, explicit)?.first().filter(|t| t.tag == TAG_SEQUENCE)?;
    let encap = *children(receipt, signed_data)?.get(2).filter(|t| t.tag == TAG_SEQUENCE)?;
    let e_content = *children(receipt, encap)?.get(1).filter(|t| t.tag == TAG_CONTEXT_0)?;
    octets(receipt, *children(receipt, e_content)?.first()?)
}

/// `original_application_version` from a receipt: on macOS the `CFBundleShortVersionString`, on iOS the
/// `CFBundleVersion` of the build the user first downloaded.
pub fn original_application_version(receipt: &[u8]) -> Option<String> {
    let payload = payload(receipt)?;
    let set = read_tlv(&payload, 0).filter(|t| t.tag == TAG_SET)?;
    for attr in children(&payload, set)? {
        let fields = children(&payload, attr)?;
        let [kind, _version, value] = fields.as_slice() else { continue };
        if uint(&payload, *kind) != Some(ORIGINAL_APPLICATION_VERSION) || value.tag != TAG_OCTET_STRING {
            continue;
        }
        let inner = read_tlv(&payload, value.start)?;
        if inner.tag == TAG_UTF8_STRING || inner.tag == TAG_IA5_STRING {
            return String::from_utf8(payload[inner.start..inner.end].to_vec()).ok();
        }
    }
    None
}

/// Where the App Store puts the receipt for the running app.
pub fn receipt_paths() -> Vec<PathBuf> {
    let Ok(exe) = std::env::current_exe() else { return Vec::new() };
    let mut paths = Vec::new();
    if cfg!(target_os = "ios") {
        // [INFERENCE] Bundle.main.appStoreReceiptURL resolves inside the .app (StoreKit/receipt, or
        // sandboxReceipt for TestFlight); older OS versions used the data container.
        if let Some(app) = exe.parent() {
            paths.push(app.join("StoreKit/receipt"));
            paths.push(app.join("StoreKit/sandboxReceipt"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(PathBuf::from(home).join("StoreKit/receipt"));
        }
    } else if let Some(contents) = exe.parent().and_then(|p| p.parent()) {
        paths.push(contents.join("_MASReceipt/receipt"));
    }
    paths
}

/// `original_application_version` of the installed app, if it came from the App Store.
pub fn installed_original_version() -> Option<String> {
    receipt_paths()
        .into_iter()
        .find_map(|p| std::fs::read(p).ok())
        .and_then(|bytes| original_application_version(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn der(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        if content.len() < 0x80 {
            out.push(content.len() as u8);
        } else {
            out.push(0x82);
            out.extend((content.len() as u16).to_be_bytes());
        }
        out.extend(content);
        out
    }

    fn indefinite(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag, 0x80];
        out.extend(content);
        out.extend([0, 0]);
        out
    }

    fn attr(kind: u8, value: &[u8]) -> Vec<u8> {
        let mut seq = der(TAG_INTEGER, &[kind]);
        seq.extend(der(TAG_INTEGER, &[1]));
        seq.extend(der(TAG_OCTET_STRING, value));
        der(TAG_SEQUENCE, &seq)
    }

    /// Minimal PKCS#7 receipt shaped like Apple's: indefinite-length outer layers, a constructed
    /// (chunked) eContent OCTET STRING, and unrelated attributes around field 19.
    fn receipt(original_version: &str) -> Vec<u8> {
        let mut attrs = attr(2, &der(TAG_UTF8_STRING, b"com.example.app"));
        attrs.extend(attr(19, &der(TAG_UTF8_STRING, original_version.as_bytes())));
        attrs.extend(attr(3, &der(TAG_UTF8_STRING, b"9.9.9")));
        let set = der(TAG_SET, &attrs);
        let (a, b) = set.split_at(set.len() / 2);
        let mut chunks = der(TAG_OCTET_STRING, a);
        chunks.extend(der(TAG_OCTET_STRING, b));
        let oid_data = der(0x06, &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x01]);
        let mut encap = oid_data.clone();
        encap.extend(indefinite(TAG_CONTEXT_0, &indefinite(TAG_OCTET_STRING_CONSTRUCTED, &chunks)));
        let mut signed = der(TAG_INTEGER, &[1]);
        signed.extend(der(TAG_SET, &[]));
        signed.extend(indefinite(TAG_SEQUENCE, &encap));
        signed.extend(der(TAG_SET, &[])); // signerInfos
        let mut root = der(0x06, &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x02]);
        root.extend(indefinite(TAG_CONTEXT_0, &indefinite(TAG_SEQUENCE, &signed)));
        indefinite(TAG_SEQUENCE, &root)
    }

    #[test]
    fn reads_original_version_through_indefinite_and_chunked_encoding() {
        assert_eq!(original_application_version(&receipt("1.3.4")).as_deref(), Some("1.3.4"));
    }

    #[test]
    fn rejects_truncated_and_garbage_input() {
        let full = receipt("1.0");
        assert_eq!(original_application_version(&full[..full.len() / 2]), None);
        assert_eq!(original_application_version(b"not a receipt"), None);
        assert_eq!(original_application_version(&[]), None);
    }
}
