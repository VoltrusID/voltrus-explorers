//! A serial session's terminal: the bytes that crossed the port, kept as
//! ready-to-draw rows in both views (text lines and hex dump) as they
//! arrive, so switching views or scrolling never re-decodes the history.

use std::collections::VecDeque;

use vt_protocol::{Direction, SerialChunk};

/// Rows kept per view; the oldest go first.
const MAX_ROWS: usize = 20_000;
/// A text line longer than this wraps onto the next row.
const MAX_LINE: usize = 1024;
/// Bytes per hex row.
pub const HEX_WIDTH: usize = 16;
/// A pause this long between chunks starts a new hex row, so frames stay
/// apart.
const HEX_GAP_MS: u64 = 100;

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub at_ms: u64,
    pub direction: Direction,
    /// The line (text view) or the bytes as hex pairs (hex view).
    pub text: String,
    /// Hex view: the bytes as printable ASCII.
    pub ascii: String,
}

/// A row still taking bytes.
#[derive(Default)]
struct Open {
    direction: Option<Direction>,
    bytes: Vec<u8>,
    last_ms: u64,
}

#[derive(Default)]
pub struct Terminal {
    pub text_rows: VecDeque<Row>,
    pub hex_rows: VecDeque<Row>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    /// Show the hex dump instead of text lines.
    pub hex: bool,
    /// Show each row's time.
    pub timestamps: bool,
    /// Keep receiving (and logging) but stop showing new bytes.
    pub paused: bool,
    /// Outcome of the last send.
    pub sent: Option<Result<u32, String>>,
    /// The file the session logs to.
    pub logging: Option<String>,
    /// Why logging stopped or could not start.
    pub logging_error: Option<String>,
    text_open: Open,
    /// Direction of the last chunk: a CR at the end of one received chunk
    /// and an LF at the start of the next are one line ending.
    last_direction: Option<Direction>,
    after_cr: bool,
    hex_open: Open,
}

impl Terminal {
    pub fn new() -> Self {
        Self {
            timestamps: true,
            ..Self::default()
        }
    }

    pub fn rows(&self) -> &VecDeque<Row> {
        if self.hex {
            &self.hex_rows
        } else {
            &self.text_rows
        }
    }

    pub fn clear(&mut self) {
        self.text_rows.clear();
        self.hex_rows.clear();
        self.text_open = Open::default();
        self.hex_open = Open::default();
        self.after_cr = false;
        self.last_direction = None;
        self.rx_bytes = 0;
        self.tx_bytes = 0;
    }

    pub fn set_logging(&mut self, result: Result<Option<String>, String>) {
        match result {
            Ok(path) => {
                self.logging = path;
                self.logging_error = None;
            }
            Err(error) => {
                self.logging = None;
                self.logging_error = Some(error);
            }
        }
    }

    pub fn push(&mut self, chunks: &[SerialChunk]) {
        for chunk in chunks {
            match chunk.direction {
                Direction::Rx => self.rx_bytes += chunk.bytes.len() as u64,
                Direction::Tx => self.tx_bytes += chunk.bytes.len() as u64,
            }
            if self.paused {
                continue;
            }
            self.push_text(chunk);
            self.push_hex(chunk);
        }
        trim(&mut self.text_rows);
        trim(&mut self.hex_rows);
    }

    /// Text: received bytes join the open line until CR, LF or CR LF; each
    /// send is its own lines.
    fn push_text(&mut self, chunk: &SerialChunk) {
        if self.last_direction != Some(chunk.direction) {
            self.end_text_line();
            self.after_cr = false;
        }
        self.last_direction = Some(chunk.direction);
        for &byte in &chunk.bytes {
            match byte {
                b'\r' => {
                    self.text_line(chunk);
                    self.end_text_line();
                    self.after_cr = true;
                }
                b'\n' if self.after_cr => self.after_cr = false,
                b'\n' => {
                    self.text_line(chunk);
                    self.end_text_line();
                }
                byte => {
                    self.after_cr = false;
                    self.text_line(chunk).bytes.push(byte);
                    if self.text_open.bytes.len() >= MAX_LINE {
                        self.end_text_line();
                    }
                }
            }
        }
        // A send without a line ending is still a line of its own.
        if chunk.direction == Direction::Tx {
            self.end_text_line();
        }
        self.redraw_open_text();
    }

    /// The open line for `chunk`, made (as a row) if there is none.
    fn text_line(&mut self, chunk: &SerialChunk) -> &mut Open {
        if self.text_open.direction.is_none() {
            self.text_open = Open {
                direction: Some(chunk.direction),
                bytes: Vec::new(),
                last_ms: chunk.at_ms,
            };
            self.text_rows.push_back(Row {
                at_ms: chunk.at_ms,
                direction: chunk.direction,
                text: String::new(),
                ascii: String::new(),
            });
        }
        &mut self.text_open
    }

    fn redraw_open_text(&mut self) {
        if self.text_open.direction.is_some()
            && let Some(row) = self.text_rows.back_mut()
        {
            row.text = printable_text(&self.text_open.bytes);
        }
    }

    fn end_text_line(&mut self) {
        self.redraw_open_text();
        self.text_open = Open::default();
    }

    fn push_hex(&mut self, chunk: &SerialChunk) {
        let continues = self.hex_open.direction == Some(chunk.direction)
            && chunk.at_ms.saturating_sub(self.hex_open.last_ms) < HEX_GAP_MS;
        if !continues {
            self.hex_open = Open::default();
        }
        for &byte in &chunk.bytes {
            if self.hex_open.direction.is_none() || self.hex_open.bytes.len() >= HEX_WIDTH {
                self.draw_hex();
                self.hex_open = Open {
                    direction: Some(chunk.direction),
                    bytes: Vec::with_capacity(HEX_WIDTH),
                    last_ms: chunk.at_ms,
                };
                self.hex_rows.push_back(Row {
                    at_ms: chunk.at_ms,
                    direction: chunk.direction,
                    text: String::new(),
                    ascii: String::new(),
                });
            }
            self.hex_open.bytes.push(byte);
        }
        self.hex_open.last_ms = chunk.at_ms;
        self.draw_hex();
    }

    /// Put the open hex row's bytes on its row.
    fn draw_hex(&mut self) {
        if self.hex_open.direction.is_some()
            && let Some(row) = self.hex_rows.back_mut()
        {
            row.text = hex_pairs(&self.hex_open.bytes);
            row.ascii = ascii(&self.hex_open.bytes);
        }
    }
}

fn trim(rows: &mut VecDeque<Row>) {
    while rows.len() > MAX_ROWS {
        rows.pop_front();
    }
}

/// A line's bytes as text: UTF-8 where it is, control characters as their
/// Unicode pictures (␀ ␛ …), tabs as spaces.
pub fn printable_text(bytes: &[u8]) -> String {
    let decoded = String::from_utf8_lossy(bytes);
    let mut text = String::with_capacity(decoded.len());
    for c in decoded.chars() {
        match c {
            '\t' => text.push_str("    "),
            '\u{7f}' => text.push('\u{2421}'),
            c if u32::from(c) < 0x20 => {
                text.push(char::from_u32(0x2400 + u32::from(c)).unwrap_or('\u{fffd}'));
            }
            c => text.push(c),
        }
    }
    text
}

/// `48 65 6C 6C 6F`.
pub fn hex_pairs(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 3);
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            text.push(' ');
        }
        text.push_str(&format!("{byte:02X}"));
    }
    text
}

/// Printable ASCII as is, everything else as a dot.
pub fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                '.'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rx(at_ms: u64, bytes: &[u8]) -> SerialChunk {
        SerialChunk {
            at_ms,
            direction: Direction::Rx,
            bytes: bytes.to_vec(),
        }
    }

    fn tx(at_ms: u64, bytes: &[u8]) -> SerialChunk {
        SerialChunk {
            at_ms,
            direction: Direction::Tx,
            bytes: bytes.to_vec(),
        }
    }

    fn texts(terminal: &Terminal) -> Vec<(Direction, String)> {
        terminal
            .text_rows
            .iter()
            .map(|row| (row.direction, row.text.clone()))
            .collect()
    }

    #[test]
    fn received_lines_build_up_across_chunks_and_end_at_any_ending() {
        let mut terminal = Terminal::new();
        terminal.push(&[rx(1, b"pH=7."), rx(2, b"2\r")]);
        terminal.push(&[rx(3, b"\nORP=245\nT=1"), rx(4, b"5\rX")]);
        assert_eq!(
            texts(&terminal),
            [
                (Direction::Rx, "pH=7.2".to_owned()),
                (Direction::Rx, "ORP=245".to_owned()),
                (Direction::Rx, "T=15".to_owned()),
                (Direction::Rx, "X".to_owned()),
            ]
        );
        // A row keeps the time its first byte arrived.
        assert_eq!(terminal.text_rows[0].at_ms, 1);
        assert_eq!(terminal.rx_bytes, 22);
    }

    #[test]
    fn a_send_is_its_own_line_and_ends_a_partial_one() {
        let mut terminal = Terminal::new();
        terminal.push(&[
            rx(1, b"> "),
            tx(2, b"READ\r\n"),
            rx(3, b"OK\r\n"),
            tx(4, b"AT"),
        ]);
        assert_eq!(
            texts(&terminal),
            [
                (Direction::Rx, "> ".to_owned()),
                (Direction::Tx, "READ".to_owned()),
                (Direction::Rx, "OK".to_owned()),
                (Direction::Tx, "AT".to_owned()),
            ]
        );
        assert_eq!(terminal.tx_bytes, 8);
    }

    #[test]
    fn controls_show_as_pictures_and_bad_utf8_as_replacements() {
        assert_eq!(printable_text(b"\x02A\tB\x03\x7f"), "␂A    B␃␡");
        assert_eq!(printable_text(&[0x41, 0xFF]), "A\u{fffd}");
        assert_eq!(printable_text("µS/cm".as_bytes()), "µS/cm");
    }

    #[test]
    fn hex_rows_hold_sixteen_bytes_and_split_on_direction_and_pauses() {
        let mut terminal = Terminal::new();
        let bytes: Vec<u8> = (0..20).collect();
        terminal.push(&[rx(0, &bytes[..10]), rx(50, &bytes[10..])]);
        terminal.push(&[tx(60, &[0x01, 0x03])]);
        terminal.push(&[tx(500, b"Hi")]);
        let rows: Vec<(&str, &str)> = terminal
            .hex_rows
            .iter()
            .map(|row| (row.text.as_str(), row.ascii.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F",
                    "................"
                ),
                ("10 11 12 13", "...."),
                ("01 03", ".."),
                ("48 69", "Hi"),
            ]
        );
        assert_eq!(terminal.hex_rows[1].direction, Direction::Rx);
        assert_eq!(terminal.hex_rows[2].direction, Direction::Tx);
    }

    #[test]
    fn paused_terminals_count_but_do_not_show_and_clear_starts_over() {
        let mut terminal = Terminal::new();
        terminal.paused = true;
        terminal.push(&[rx(1, b"lost\n")]);
        assert!(terminal.text_rows.is_empty() && terminal.hex_rows.is_empty());
        assert_eq!(terminal.rx_bytes, 5);
        terminal.paused = false;
        terminal.push(&[rx(2, b"kept")]);
        terminal.clear();
        assert!(terminal.rows().is_empty());
        assert_eq!(terminal.rx_bytes, 0);
        // After a clear, the open line is gone too.
        terminal.push(&[rx(3, b"new\n")]);
        assert_eq!(texts(&terminal), [(Direction::Rx, "new".to_owned())]);
    }

    #[test]
    fn long_lines_wrap_and_history_is_bounded() {
        let mut terminal = Terminal::new();
        terminal.push(&[rx(1, &vec![b'x'; MAX_LINE + 5])]);
        assert_eq!(terminal.text_rows.len(), 2);
        assert_eq!(terminal.text_rows[1].text, "xxxxx");
        let many: Vec<SerialChunk> = (0..MAX_ROWS as u64 + 10).map(|at| rx(at, b"a\n")).collect();
        terminal.push(&many);
        assert_eq!(terminal.text_rows.len(), MAX_ROWS);
    }

    #[test]
    fn logging_follows_the_engine() {
        let mut terminal = Terminal::new();
        terminal.set_logging(Ok(Some("/x.csv".into())));
        assert_eq!(terminal.logging.as_deref(), Some("/x.csv"));
        terminal.set_logging(Err("disk full".into()));
        assert_eq!(terminal.logging, None);
        assert_eq!(terminal.logging_error.as_deref(), Some("disk full"));
        terminal.set_logging(Ok(None));
        assert_eq!(terminal.logging_error, None);
    }
}
