//! SML, the text notation SECS/GEM tools use for messages:
//!
//! ```text
//! S1F13 W <L [2] <A "MDLN"> <A "SOFTREV">>.
//! ```
//!
//! Rendering writes the `[n]` size on lists and quotes printable ASCII; anything else in text
//! goes out as `0x..` bytes (`<A "line" 0x0A "next">`), so no escape syntax is needed. The parser
//! accepts what the renderer writes plus the usual variations: `[n]` sizes on any item (checked
//! against the content), `<L[2]` without a space, hex or decimal numbers, `T`/`F`/`TRUE`/`FALSE`
//! booleans, a missing trailing `.`, and any amount of white space.

use std::fmt::Write as _;

use crate::error::CodecError;
use crate::item::Item;
use crate::message::SecsMessage;

impl Item {
    /// One-line SML, e.g. `<L [2] <A "MDLN"> <U4 1>>`.
    pub fn to_sml(&self) -> String {
        let mut out = String::new();
        render(self, &mut out, None, 0);
        out
    }

    /// Multi-line SML, two spaces per nesting level.
    pub fn to_sml_pretty(&self) -> String {
        let mut out = String::new();
        render(self, &mut out, Some(2), 0);
        out
    }

    /// Parse one item, `<...>`.
    pub fn from_sml(text: &str) -> Result<Item, CodecError> {
        let mut p = Parser::new(text);
        let item = p.item()?;
        p.skip_ws();
        if let Some(c) = p.peek() {
            return Err(err(format!("unexpected `{c}` after the item")));
        }
        Ok(item)
    }
}

impl SecsMessage {
    /// One-line SML, e.g. `S1F13 W <L [0]>.`
    pub fn to_sml(&self) -> String {
        let mut out = format!("S{}F{}", self.stream, self.function);
        if self.w_bit {
            out.push_str(" W");
        }
        if let Some(item) = &self.item {
            out.push(' ');
            out.push_str(&item.to_sml());
        }
        out.push('.');
        out
    }

    /// Multi-line SML: the header, the item with indentation, and the closing dot on its own line.
    pub fn to_sml_pretty(&self) -> String {
        let mut out = format!("S{}F{}", self.stream, self.function);
        if self.w_bit {
            out.push_str(" W");
        }
        if let Some(item) = &self.item {
            out.push('\n');
            out.push_str(&item.to_sml_pretty());
            out.push('\n');
        }
        out.push('.');
        out
    }

    /// Parse `S1F13 W <...>.` (the W and the item are optional).
    pub fn from_sml(text: &str) -> Result<SecsMessage, CodecError> {
        let mut p = Parser::new(text);
        let head = p.word();
        let (stream, function) = parse_sf(&head)?;
        p.skip_ws();
        let mut w_bit = false;
        if matches!(p.peek(), Some('W' | 'w')) {
            let w = p.word();
            if !w.eq_ignore_ascii_case("w") {
                return Err(err(format!("expected `W`, found `{w}`")));
            }
            w_bit = true;
        }
        p.skip_ws();
        let item = if p.peek() == Some('<') {
            Some(p.item()?)
        } else {
            None
        };
        p.skip_ws();
        if p.peek() == Some('.') {
            p.bump();
            p.skip_ws();
        }
        if let Some(c) = p.peek() {
            return Err(err(format!("unexpected `{c}` after the message")));
        }
        Ok(SecsMessage {
            stream,
            function,
            w_bit,
            item,
        })
    }
}

fn err(msg: impl Into<String>) -> CodecError {
    CodecError::Sml(msg.into())
}

fn parse_sf(word: &str) -> Result<(u8, u8), CodecError> {
    let bad = || err(format!("`{word}` is not a stream/function like S1F13"));
    let rest = word
        .strip_prefix('S')
        .or_else(|| word.strip_prefix('s'))
        .ok_or_else(bad)?;
    let (s, f) = rest
        .split_once('F')
        .or_else(|| rest.split_once('f'))
        .ok_or_else(bad)?;
    let stream: u8 = s.parse().map_err(|_| bad())?;
    let function: u8 = f.parse().map_err(|_| bad())?;
    if stream > 127 {
        return Err(err(format!("stream {stream} is above 127")));
    }
    Ok((stream, function))
}

// ---- rendering ---------------------------------------------------------------------------

fn render(item: &Item, out: &mut String, indent: Option<usize>, depth: usize) {
    match item {
        Item::List(items) => {
            let _ = write!(out, "<L [{}]", items.len());
            for child in items {
                match indent {
                    Some(step) => {
                        out.push('\n');
                        out.push_str(&" ".repeat((depth + 1) * step));
                    }
                    None => out.push(' '),
                }
                render(child, out, indent, depth + 1);
            }
            if let (Some(step), false) = (indent, items.is_empty()) {
                out.push('\n');
                out.push_str(&" ".repeat(depth * step));
            }
            out.push('>');
        }
        Item::Ascii(s) => {
            let bytes: Vec<u8> = s
                .chars()
                .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
                .collect();
            render_text(out, "A", &bytes);
        }
        Item::Jis8(v) => render_text(out, "J", v),
        Item::Binary(v) => render_nums(out, "B", v.iter().map(|b| format!("0x{b:02X}"))),
        Item::Boolean(v) => render_nums(
            out,
            "BOOLEAN",
            v.iter()
                .map(|b| if *b { "TRUE" } else { "FALSE" }.to_string()),
        ),
        Item::I1(v) => render_nums(out, "I1", v.iter().map(ToString::to_string)),
        Item::I2(v) => render_nums(out, "I2", v.iter().map(ToString::to_string)),
        Item::I4(v) => render_nums(out, "I4", v.iter().map(ToString::to_string)),
        Item::I8(v) => render_nums(out, "I8", v.iter().map(ToString::to_string)),
        Item::U1(v) => render_nums(out, "U1", v.iter().map(ToString::to_string)),
        Item::U2(v) => render_nums(out, "U2", v.iter().map(ToString::to_string)),
        Item::U4(v) => render_nums(out, "U4", v.iter().map(ToString::to_string)),
        Item::U8(v) => render_nums(out, "U8", v.iter().map(ToString::to_string)),
        Item::F4(v) => render_nums(out, "F4", v.iter().map(|x| format!("{x:?}"))),
        Item::F8(v) => render_nums(out, "F8", v.iter().map(|x| format!("{x:?}"))),
    }
}

fn render_nums(out: &mut String, name: &str, values: impl Iterator<Item = String>) {
    out.push('<');
    out.push_str(name);
    for v in values {
        out.push(' ');
        out.push_str(&v);
    }
    out.push('>');
}

fn render_text(out: &mut String, name: &str, bytes: &[u8]) {
    out.push('<');
    out.push_str(name);
    let mut quoted = false;
    for &b in bytes {
        let printable = (0x20..0x7f).contains(&b) && b != b'"';
        if printable {
            if !quoted {
                out.push_str(" \"");
                quoted = true;
            }
            out.push(char::from(b));
        } else {
            if quoted {
                out.push('"');
                quoted = false;
            }
            let _ = write!(out, " 0x{b:02X}");
        }
    }
    if quoted {
        out.push('"');
    }
    out.push('>');
}

// ---- parsing -----------------------------------------------------------------------------

struct Parser<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Parser {
            chars: text.chars().peekable(),
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn bump(&mut self) -> Option<char> {
        self.chars.next()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    /// A run of characters that are not white space or one of `<>[]"`.
    fn word(&mut self) -> String {
        self.skip_ws();
        let mut w = String::new();
        while let Some(c) = self.peek() {
            let stop = c.is_whitespace()
                || matches!(c, '<' | '>' | '[' | ']' | '"')
                || (c == '.' && !is_num_dot(&w, c));
            if stop {
                break;
            }
            w.push(c);
            self.bump();
        }
        w
    }

    fn item(&mut self) -> Result<Item, CodecError> {
        self.skip_ws();
        if self.bump() != Some('<') {
            return Err(err("expected `<`"));
        }
        let name = self.word().to_ascii_uppercase();
        if name.is_empty() {
            return Err(err("missing item type after `<`"));
        }
        self.skip_ws();
        let size = if self.peek() == Some('[') {
            self.bump();
            let n = self.word();
            self.skip_ws();
            if self.bump() != Some(']') {
                return Err(err("expected `]` after the size"));
            }
            Some(
                n.parse::<usize>()
                    .map_err(|_| err(format!("`{n}` is not a size")))?,
            )
        } else {
            None
        };

        let item = if name == "L" {
            let mut items = Vec::new();
            loop {
                self.skip_ws();
                match self.peek() {
                    Some('<') => items.push(self.item()?),
                    Some('>') => {
                        self.bump();
                        break;
                    }
                    Some(c) => return Err(err(format!("unexpected `{c}` in a list"))),
                    None => return Err(err("list is not closed with `>`")),
                }
            }
            Item::List(items)
        } else {
            self.scalar(&name)?
        };
        if let Some(n) = size
            && item.len() != n
        {
            return Err(err(format!("<{name} [{n}]> has {} elements", item.len())));
        }
        Ok(item)
    }

    /// Everything of a non-list item up to and including its closing `>`.
    fn scalar(&mut self, name: &str) -> Result<Item, CodecError> {
        enum Tok {
            Text(Vec<u8>),
            Word(String),
        }
        let mut toks = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some('>') => {
                    self.bump();
                    break;
                }
                Some('"') => {
                    self.bump();
                    let mut bytes = Vec::new();
                    loop {
                        match self.bump() {
                            Some('"') => break,
                            Some(c) => bytes.push(u8::try_from(u32::from(c)).map_err(|_| {
                                err(format!("`{c}` is not a single-byte character"))
                            })?),
                            None => return Err(err("string is not closed with `\"`")),
                        }
                    }
                    toks.push(Tok::Text(bytes));
                }
                Some('<') => return Err(err(format!("<{name}> cannot contain an item"))),
                Some(_) => {
                    let w = self.word();
                    if w.is_empty() {
                        return Err(err("unexpected character"));
                    }
                    toks.push(Tok::Word(w));
                }
                None => return Err(err(format!("<{name}> is not closed with `>`"))),
            }
        }

        if matches!(name, "A" | "J") {
            let mut bytes = Vec::new();
            for t in toks {
                match t {
                    Tok::Text(b) => bytes.extend(b),
                    Tok::Word(w) => bytes.push(
                        u8::try_from(parse_int(&w)?)
                            .map_err(|_| err(format!("`{w}` is not a byte")))?,
                    ),
                }
            }
            return Ok(if name == "A" {
                Item::Ascii(bytes.into_iter().map(char::from).collect())
            } else {
                Item::Jis8(bytes)
            });
        }

        let words: Vec<String> = toks
            .into_iter()
            .map(|t| match t {
                Tok::Word(w) => Ok(w),
                Tok::Text(_) => Err(err(format!("<{name}> takes numbers, not a string"))),
            })
            .collect::<Result<_, _>>()?;

        macro_rules! ints {
            ($variant:ident, $ty:ty) => {
                Item::$variant(
                    words
                        .iter()
                        .map(|w| {
                            <$ty>::try_from(parse_int(w)?)
                                .map_err(|_| err(format!("`{w}` does not fit {name}")))
                        })
                        .collect::<Result<_, _>>()?,
                )
            };
        }
        macro_rules! floats {
            ($variant:ident, $ty:ty) => {
                Item::$variant(
                    words
                        .iter()
                        .map(|w| {
                            w.parse::<$ty>()
                                .map_err(|_| err(format!("`{w}` is not a number")))
                        })
                        .collect::<Result<_, _>>()?,
                )
            };
        }

        Ok(match name {
            "B" | "BINARY" => ints!(Binary, u8),
            "BOOLEAN" | "BOOL" => Item::Boolean(
                words
                    .iter()
                    .map(|w| match w.to_ascii_uppercase().as_str() {
                        "T" | "TRUE" | "1" => Ok(true),
                        "F" | "FALSE" | "0" => Ok(false),
                        _ => Err(err(format!("`{w}` is not a boolean"))),
                    })
                    .collect::<Result<_, _>>()?,
            ),
            "I1" => ints!(I1, i8),
            "I2" => ints!(I2, i16),
            "I4" => ints!(I4, i32),
            "I8" => ints!(I8, i64),
            "U1" => ints!(U1, u8),
            "U2" => ints!(U2, u16),
            "U4" => ints!(U4, u32),
            "U8" => ints!(U8, u64),
            "F4" => floats!(F4, f32),
            "F8" => floats!(F8, f64),
            other => return Err(err(format!("unknown item type `{other}`"))),
        })
    }
}

/// `.` ends a message, but is part of a number like `1.5` or `.5`.
fn is_num_dot(word_so_far: &str, c: char) -> bool {
    c == '.'
        && word_so_far
            .chars()
            .all(|x| x.is_ascii_digit() || x == '-' || x == '+')
}

fn parse_int(w: &str) -> Result<i128, CodecError> {
    let (neg, digits) = match w.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, w.strip_prefix('+').unwrap_or(w)),
    };
    let v = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        i128::from_str_radix(hex, 16)
    } else if let Some(bin) = digits
        .strip_prefix("0b")
        .or_else(|| digits.strip_prefix("0B"))
    {
        i128::from_str_radix(bin, 2)
    } else {
        digits.parse::<i128>()
    }
    .map_err(|_| err(format!("`{w}` is not an integer")))?;
    Ok(if neg { -v } else { v })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_documented_example() {
        let m = SecsMessage::request(
            1,
            13,
            Some(Item::list([Item::ascii("MDLN"), Item::ascii("SOFTREV")])),
        );
        assert_eq!(m.to_sml(), r#"S1F13 W <L [2] <A "MDLN"> <A "SOFTREV">>."#);
        assert_eq!(SecsMessage::from_sml(&m.to_sml()).unwrap(), m);
    }

    #[test]
    fn parses_variations() {
        let m = SecsMessage::from_sml("s1f13 w <L[2] <A 'x'>").unwrap_err();
        assert!(matches!(m, CodecError::Sml(_)));
        let m = SecsMessage::from_sml("S1F1 W.").unwrap();
        assert_eq!(m, SecsMessage::request(1, 1, None));
        let m = SecsMessage::from_sml("S1F2\n<L\n  <A \"M\">\n  <A>\n>\n.").unwrap();
        assert_eq!(
            m,
            SecsMessage::notify(1, 2, Some(Item::list([Item::ascii("M"), Item::ascii("")])))
        );
        // no trailing dot, sizes, hex/dec/negative numbers, bool spellings
        let i = Item::from_sml("<L [4] <U1 0x10 20> <I2[2] -3 +4> <BOOLEAN T false 1> <B 0x01 2>>")
            .unwrap();
        assert_eq!(
            i,
            Item::list([
                Item::U1(vec![16, 20]),
                Item::I2(vec![-3, 4]),
                Item::Boolean(vec![true, false, true]),
                Item::Binary(vec![1, 2]),
            ])
        );
        assert_eq!(
            Item::from_sml("<F4 1.5 -2 1e3>").unwrap(),
            Item::F4(vec![1.5, -2.0, 1000.0])
        );
        assert_eq!(Item::from_sml("<F8 .5>").unwrap(), Item::F8(vec![0.5]));
    }

    #[test]
    fn float_before_the_closing_dot() {
        let m = SecsMessage::from_sml("S6F11 <F4 2.5>.").unwrap();
        assert_eq!(m.item, Some(Item::f4(2.5)));
        let m = SecsMessage::from_sml("S6F11 <U1 3>.").unwrap();
        assert_eq!(m.item, Some(Item::u1(3)));
    }

    #[test]
    fn text_round_trips_with_special_bytes() {
        let item = Item::ascii("say \"hi\"\n\tbye\u{e9}");
        let sml = item.to_sml();
        assert_eq!(sml, r#"<A "say " 0x22 "hi" 0x22 0x0A 0x09 "bye" 0xE9>"#);
        assert_eq!(Item::from_sml(&sml).unwrap(), item);
        assert_eq!(
            Item::from_sml(r#"<A "a" 0x20 "b">"#).unwrap(),
            Item::ascii("a b")
        );
        assert_eq!(Item::from_sml("<A>").unwrap(), Item::ascii(""));
        assert_eq!(Item::Jis8(vec![0x41, 0xb1]).to_sml(), r#"<J "A" 0xB1>"#);
    }

    #[test]
    fn every_type_round_trips_through_sml() {
        let item = Item::list([
            Item::empty_list(),
            Item::Binary(vec![0, 255]),
            Item::Binary(vec![]),
            Item::Boolean(vec![true, false]),
            Item::ascii("A B"),
            Item::Jis8(vec![0x41, 0xb1, 0x00]),
            Item::I1(vec![-128, 127]),
            Item::I2(vec![-32768, 32767]),
            Item::I4(vec![i32::MIN, i32::MAX]),
            Item::I8(vec![i64::MIN, i64::MAX]),
            Item::U1(vec![0, 255]),
            Item::U2(vec![0, 65535]),
            Item::U4(vec![0, u32::MAX]),
            Item::U8(vec![0, u64::MAX]),
            Item::F4(vec![0.0, -1.5, 1.0e-7, 3.25e10, f32::MAX]),
            Item::F8(vec![0.0, -1.5, 1.0e-300, f64::MAX]),
            Item::list([Item::list([Item::u1(1)])]),
        ]);
        for sml in [item.to_sml(), item.to_sml_pretty()] {
            assert_eq!(Item::from_sml(&sml).unwrap(), item, "{sml}");
        }
        let m = SecsMessage::request(6, 11, Some(item));
        for sml in [m.to_sml(), m.to_sml_pretty()] {
            assert_eq!(SecsMessage::from_sml(&sml).unwrap(), m, "{sml}");
        }
    }

    #[test]
    fn pretty_layout() {
        let item = Item::list([
            Item::u1(1),
            Item::list([Item::ascii("x")]),
            Item::empty_list(),
        ]);
        assert_eq!(
            item.to_sml_pretty(),
            "<L [3]\n  <U1 1>\n  <L [1]\n    <A \"x\">\n  >\n  <L [0]>\n>"
        );
    }

    #[test]
    fn errors_are_reported() {
        for bad in [
            "<U1 256>",
            "<U1 -1>",
            "<I1 128>",
            "<L [2] <U1 1>>",
            "<U1 [2] 1>",
            "<X 1>",
            "<U1 1",
            "<A \"open>",
            "<L <U1 1>",
            "<BOOLEAN maybe>",
            "<U4 1.5>",
            "<A 300>",
            "<U1 \"x\">",
            "U1 1",
            "<U1 1> <U1 2>",
        ] {
            assert!(Item::from_sml(bad).is_err(), "{bad}");
        }
        for bad in ["X1F1", "S1F", "S200F1", "S1F1 Z", "S1F1 W <U1 1> junk"] {
            assert!(SecsMessage::from_sml(bad).is_err(), "{bad}");
        }
    }
}
