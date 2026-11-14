//! One-line descriptions of the frames on the wire, for the comm log.

use mt_fins::frame::tcp;
use mt_fins::{Command as FinsCommand, FinsHeader, FinsResponse};
use mt_melsec::{Command as McCommand, Encoding, FrameKind, Request, Response};

/// The bytes as the log shows them: ASCII frames as text, binary frames as spaced hex.
pub fn render(bytes: &[u8]) -> String {
    let printable = !bytes.is_empty() && bytes.iter().all(|b| (0x20..0x7F).contains(b));
    if printable && mt_melsec::frame::detect(bytes).is_some_and(|(_, e)| e == Encoding::Ascii) {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

fn mc_command(c: &McCommand) -> String {
    match c {
        McCommand::ReadWords { head, count } => format!("batch read {head} ×{count} words"),
        McCommand::ReadBits { head, count } => format!("batch read {head} ×{count} bits"),
        McCommand::WriteWords { head, values } => format!("batch write {head} ×{} words", values.len()),
        McCommand::WriteBits { head, values } => format!("batch write {head} ×{} bits", values.len()),
        McCommand::ReadCpuModel => "read CPU model".into(),
        McCommand::RemoteRun { .. } => "remote RUN".into(),
        McCommand::RemoteStop => "remote STOP".into(),
    }
}

fn kind(k: FrameKind, e: Encoding) -> String {
    format!(
        "{} {}",
        if k == FrameKind::E3 { "3E" } else { "4E" },
        if e == Encoding::Binary { "binary" } else { "ASCII" }
    )
}

pub fn describe_mc(bytes: &[u8]) -> String {
    if let Ok(r) = Request::decode(bytes) {
        let what = McCommand::decode(r.command, r.subcommand, &r.body, r.encoding)
            .map(|c| mc_command(&c))
            .unwrap_or_else(|_| format!("command {:04X} sub {:04X}", r.command, r.subcommand));
        let serial = if r.kind == FrameKind::E4 { format!(", serial {}", r.serial) } else { String::new() };
        return format!("{}{serial}: {what}", kind(r.kind, r.encoding));
    }
    if let Ok(r) = Response::decode(bytes) {
        let serial = if r.kind == FrameKind::E4 { format!(", serial {}", r.serial) } else { String::new() };
        return if r.end_code == 0 {
            format!("{}{serial}: OK, {} data bytes", kind(r.kind, r.encoding), r.data.len())
        } else {
            let text = mt_melsec::end_code_text(r.end_code).unwrap_or("unknown end code");
            format!("{}{serial}: end code 0x{:04X}, {text}", kind(r.kind, r.encoding), r.end_code)
        };
    }
    "not an MC protocol frame".into()
}

fn fins_command(c: &FinsCommand) -> String {
    match c {
        FinsCommand::ReadWords { at, count } => format!("0101 memory area read {at} ×{count} words"),
        FinsCommand::ReadBits { at, count } => format!("0101 memory area read {at} ×{count} bits"),
        FinsCommand::WriteWords { at, values } => format!("0102 memory area write {at} ×{} words", values.len()),
        FinsCommand::WriteBits { at, values } => format!("0102 memory area write {at} ×{} bits", values.len()),
        FinsCommand::Run(m) => format!("0401 RUN ({m:?} mode)"),
        FinsCommand::Stop => "0402 STOP (PROGRAM mode)".into(),
        FinsCommand::ControllerData => "0501 controller data read".into(),
        FinsCommand::ControllerStatus => "0601 controller status read".into(),
    }
}

fn describe_fins_frame(b: &[u8]) -> String {
    let Some(h) = FinsHeader::decode(b) else { return "short FINS frame".into() };
    if h.is_response() {
        return match FinsResponse::decode(b) {
            Ok(r) if r.end_code() == 0 => format!("SID {} from node {}: OK, {} data bytes", h.sid, h.sa1, r.data.len()),
            Ok(r) => format!(
                "SID {} from node {}: end code 0x{:04X}, {}",
                h.sid,
                h.sa1,
                r.end_code(),
                mt_fins::end_code_text(r.end_code()).unwrap_or("unknown end code")
            ),
            Err(e) => e,
        };
    }
    let what = FinsCommand::decode(&b[mt_fins::frame::HEADER_LEN..]).map(|c| fins_command(&c)).unwrap_or_else(|_| "unknown command".into());
    format!("SID {} node {} → {}: {what}", h.sid, h.sa1, h.da1)
}

pub fn describe_fins(bytes: &[u8]) -> String {
    if bytes.starts_with(tcp::MAGIC) && bytes.len() >= 16 {
        let (command, error, data) = tcp::parse(bytes);
        let node = |d: &[u8], i: usize| d.get(i..i + 4).map(|n| u32::from_be_bytes([n[0], n[1], n[2], n[3]])).unwrap_or(0);
        return match command {
            tcp::NODE_REQUEST => format!("FINS/TCP node address request (client node {})", node(data, 0)),
            tcp::NODE_RESPONSE if error != 0 => format!("FINS/TCP handshake refused: {}", tcp::error_text(error)),
            tcp::NODE_RESPONSE => format!("FINS/TCP node addresses: client {}, PLC {}", node(data, 0), node(data, 4)),
            tcp::FRAME => format!("FINS/TCP {}", describe_fins_frame(data)),
            _ => format!("FINS/TCP error 0x{error:08X}: {}", tcp::error_text(error)),
        };
    }
    format!("FINS/UDP {}", describe_fins_frame(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_melsec::{McAddress, McDevice, Route};

    #[test]
    fn describes_mc_frames() {
        let req = Request::new(FrameKind::E3, Encoding::Binary, 0, Route::default(), 16, &McCommand::ReadWords {
            head: McAddress::new(McDevice::D, 100),
            count: 3,
        });
        assert_eq!(describe_mc(&req.encode()), "3E binary: batch read D100 ×3 words");
        let err = Response::to(&req, 0xC056, vec![]);
        assert!(describe_mc(&err.encode()).contains("end code 0xC056"));
        let ascii = Request::new(FrameKind::E4, Encoding::Ascii, 9, Route::default(), 16, &McCommand::ReadCpuModel);
        assert_eq!(render(&ascii.encode()), String::from_utf8(ascii.encode()).unwrap());
        assert_eq!(describe_mc(&ascii.encode()), "4E ASCII, serial 9: read CPU model");
        assert_eq!(render(&[0x50, 0x00, 0xFF]), "50 00 FF");
    }

    #[test]
    fn describes_fins_frames() {
        let h = FinsHeader::command(0, 10, 0, 0, 1, 0, 7);
        let f = mt_fins::frame::encode_command(&h, &FinsCommand::ReadWords {
            at: mt_fins::FinsAddress::new(mt_fins::FinsArea::Dm, 100),
            count: 2,
        })
        .unwrap();
        assert_eq!(describe_fins(&f), "FINS/UDP SID 7 node 1 → 10: 0101 memory area read D100 ×2 words");
        let env = tcp::envelope(tcp::FRAME, 0, &f);
        assert!(describe_fins(&env).starts_with("FINS/TCP SID 7"));
        let r = mt_fins::frame::encode_response(&h.reply(), 0x0101, 0x1104, &[]);
        assert!(describe_fins(&r).contains("0x1104"));
    }
}
