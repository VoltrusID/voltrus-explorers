//! Received bytes into lines, and lines into point values.

use std::collections::HashMap;

use regex::Regex;
use vt_protocol::{LineParser, PointInfo, Quality, Reading};

/// A line longer than this ends anyway, so a device that never sends a line
/// ending cannot grow the buffer without bound.
const MAX_LINE: usize = 4096;

/// Splits a byte stream into lines at CR, LF or CR LF, across reads.
#[derive(Default)]
pub(crate) struct Lines {
    current: Vec<u8>,
    after_cr: bool,
}

impl Lines {
    /// Feed received bytes; complete lines (without their endings, decoded
    /// lossily as UTF-8) go to `out`.
    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<String>) {
        for &byte in bytes {
            match byte {
                b'\r' => {
                    self.end(out);
                    self.after_cr = true;
                }
                b'\n' if self.after_cr => self.after_cr = false,
                b'\n' => self.end(out),
                byte => {
                    self.after_cr = false;
                    self.current.push(byte);
                    if self.current.len() >= MAX_LINE {
                        self.end(out);
                    }
                }
            }
        }
    }

    fn end(&mut self, out: &mut Vec<String>) {
        out.push(String::from_utf8_lossy(&self.current).into_owned());
        self.current.clear();
    }
}

/// The number at the start of `text` and what follows it: `21.4°C` is
/// 21.4 and `°C`. Accepts a sign, a decimal point and an exponent; refuses
/// `nan`, `inf` and text without digits.
pub(crate) fn number_prefix(text: &str) -> Option<(f64, &str)> {
    let bytes = text.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        end += 1;
    }
    let digits_start = end;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    let mut digits = end - digits_start;
    if bytes.get(end) == Some(&b'.') {
        let fraction_start = end + 1;
        let mut fraction_end = fraction_start;
        while bytes.get(fraction_end).is_some_and(u8::is_ascii_digit) {
            fraction_end += 1;
        }
        digits += fraction_end - fraction_start;
        // `5.` is 5; a lone `.` is no number.
        if digits > 0 {
            end = fraction_end;
        }
    }
    if digits == 0 {
        return None;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut exponent = end + 1;
        if matches!(bytes.get(exponent), Some(b'+' | b'-')) {
            exponent += 1;
        }
        let exponent_digits = exponent;
        while bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
            exponent += 1;
        }
        if exponent > exponent_digits {
            end = exponent;
        }
    }
    let value: f64 = text[..end].parse().ok()?;
    value.is_finite().then_some((value, &text[end..]))
}

/// A whole field as a number, allowing surrounding spaces.
fn whole_number(text: &str) -> Option<f64> {
    match number_prefix(text.trim()) {
        Some((value, "")) => Some(value),
        _ => None,
    }
}

enum Kind {
    None,
    KeyValue,
    Csv {
        delimiter: char,
        names: Vec<String>,
        /// Names from the last header line seen.
        header: Vec<String>,
        /// The point of each column, once it has carried a number.
        columns: Vec<Option<usize>>,
    },
    Regex(Regex),
}

/// What a line changed.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Parsed {
    /// Points seen for the first time, ids continuing from the known ones.
    pub added: Vec<PointInfo>,
    /// Whether any value arrived.
    pub values: bool,
}

/// Turns lines into point values with a monitor's [`LineParser`].
pub(crate) struct Parser {
    kind: Kind,
    points: Vec<PointInfo>,
    by_name: HashMap<String, usize>,
    values: Vec<Option<f64>>,
}

impl Parser {
    /// A parser for `spec`, or why its regular expression is no good.
    pub fn new(spec: &LineParser) -> Result<Self, String> {
        let kind = match spec {
            LineParser::None => Kind::None,
            LineParser::KeyValue => Kind::KeyValue,
            LineParser::Csv { delimiter, names } => Kind::Csv {
                delimiter: *delimiter,
                names: names.clone(),
                header: Vec::new(),
                columns: Vec::new(),
            },
            LineParser::Regex { pattern } => Kind::Regex(compile(pattern)?),
        };
        Ok(Self {
            kind,
            points: Vec::new(),
            by_name: HashMap::new(),
            values: Vec::new(),
        })
    }

    pub fn is_none(&self) -> bool {
        matches!(self.kind, Kind::None)
    }

    /// The latest value of every point; bad until a point's first value.
    pub fn readings(&self) -> Vec<Reading> {
        self.values
            .iter()
            .map(|value| match value {
                Some(value) => Reading {
                    value: *value,
                    raw: [0; 4],
                    raw_len: 0,
                    quality: Quality::Good,
                },
                None => Reading::bad(),
            })
            .collect()
    }

    pub fn line(&mut self, line: &str) -> Parsed {
        let mut parsed = Parsed::default();
        match &mut self.kind {
            Kind::None => {}
            Kind::KeyValue => {
                for (key, value, unit) in key_values(line) {
                    let index = Self::point(
                        &mut self.points,
                        &mut self.by_name,
                        &mut self.values,
                        &mut parsed,
                        key,
                        &format!("{key}="),
                        unit,
                    );
                    self.values[index] = Some(value);
                    parsed.values = true;
                }
            }
            Kind::Csv {
                delimiter,
                names,
                header,
                columns,
            } => {
                let fields: Vec<&str> = line
                    .trim()
                    .trim_start_matches('#')
                    .split(*delimiter)
                    .map(str::trim)
                    .collect();
                if fields.iter().all(|field| field.is_empty()) {
                    return parsed;
                }
                let numbers: Vec<Option<f64>> =
                    fields.iter().map(|field| whole_number(field)).collect();
                if numbers.iter().all(Option::is_none) {
                    // A header names the columns that follow.
                    *header = fields.iter().map(|field| (*field).to_owned()).collect();
                    return parsed;
                }
                if columns.len() < fields.len() {
                    columns.resize(fields.len(), None);
                }
                for (column, number) in numbers.into_iter().enumerate() {
                    let Some(value) = number else {
                        continue;
                    };
                    let index = match columns[column] {
                        Some(index) => index,
                        None => {
                            let name = names
                                .get(column)
                                .or_else(|| header.get(column))
                                .filter(|name| !name.is_empty())
                                .cloned()
                                .unwrap_or_else(|| format!("Column {}", column + 1));
                            let index = Self::point(
                                &mut self.points,
                                &mut self.by_name,
                                &mut self.values,
                                &mut parsed,
                                &name,
                                &format!("col {}", column + 1),
                                "",
                            );
                            columns[column] = Some(index);
                            index
                        }
                    };
                    self.values[index] = Some(value);
                    parsed.values = true;
                }
            }
            Kind::Regex(regex) => {
                let Some(captures) = regex.captures(line) else {
                    return parsed;
                };
                let names: Vec<Option<String>> = regex
                    .capture_names()
                    .map(|name| name.map(str::to_owned))
                    .collect();
                for group in 1..captures.len() {
                    let Some(value) = captures
                        .get(group)
                        .and_then(|matched| whole_number(matched.as_str()))
                    else {
                        continue;
                    };
                    let name = names
                        .get(group)
                        .cloned()
                        .flatten()
                        .unwrap_or_else(|| format!("Group {group}"));
                    let index = Self::point(
                        &mut self.points,
                        &mut self.by_name,
                        &mut self.values,
                        &mut parsed,
                        &name,
                        &format!("group {group}"),
                        "",
                    );
                    self.values[index] = Some(value);
                    parsed.values = true;
                }
            }
        }
        parsed
    }

    /// The point named `name`, made (and reported in `parsed`) if new.
    fn point(
        points: &mut Vec<PointInfo>,
        by_name: &mut HashMap<String, usize>,
        values: &mut Vec<Option<f64>>,
        parsed: &mut Parsed,
        name: &str,
        address: &str,
        unit: &str,
    ) -> usize {
        if let Some(index) = by_name.get(name) {
            return *index;
        }
        let index = points.len();
        let info = PointInfo {
            id: u32::try_from(index).unwrap_or(u32::MAX),
            name: name.to_owned(),
            address: address.to_owned(),
            data_type: "Number".into(),
            unit: unit.to_owned(),
            writable: false,
            boolean: false,
            states: Vec::new(),
        };
        points.push(info.clone());
        parsed.added.push(info);
        values.push(None);
        by_name.insert(name.to_owned(), index);
        index
    }
}

/// A regular expression a line parser can use: it compiles and captures.
pub(crate) fn compile(pattern: &str) -> Result<Regex, String> {
    let regex =
        Regex::new(pattern).map_err(|error| format!("The pattern is not valid: {error}"))?;
    if regex.captures_len() < 2 {
        return Err(
            "The pattern needs a capture group around each number, e.g. T=(\\d+\\.\\d+)".into(),
        );
    }
    Ok(regex)
}

/// The `name=value` and `name:value` pairs of a line, with any unit written
/// after the number. Keys need a letter, so a time like `12:30` is no pair.
fn key_values(line: &str) -> Vec<(&str, f64, &str)> {
    let tokens: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .filter(|token| !token.is_empty())
        .collect();
    let mut pairs = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        index += 1;
        let Some(split) = token.find(['=', ':']) else {
            continue;
        };
        let key = &token[..split];
        let mut rest = &token[split + 1..];
        // `T: 21.5` and `T= 21.5`: the value is the next token.
        if rest.is_empty()
            && let Some(next) = tokens.get(index)
        {
            rest = next;
            index += 1;
        }
        if !key.chars().any(char::is_alphabetic) {
            continue;
        }
        let Some((value, unit)) = number_prefix(rest) else {
            continue;
        };
        let unit = unit.trim();
        // A unit is a short word, not more data (`1:2:3`) or the key again.
        let unit = if unit.chars().count() > 12
            || unit.contains(['=', ':'])
            || unit.eq_ignore_ascii_case(key)
        {
            ""
        } else {
            unit
        };
        pairs.push((key, value, unit));
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(chunks: &[&[u8]]) -> Vec<String> {
        let mut splitter = Lines::default();
        let mut out = Vec::new();
        for chunk in chunks {
            splitter.push(chunk, &mut out);
        }
        out
    }

    #[test]
    fn lines_end_at_cr_lf_or_both_even_across_reads() {
        assert_eq!(lines(&[b"a\r\nb\nc\rd"]), ["a", "b", "c"]);
        // CR at the end of one read, LF at the start of the next: one ending.
        assert_eq!(lines(&[b"one\r", b"\ntwo\r\n"]), ["one", "two"]);
        // Blank lines are lines.
        assert_eq!(lines(&[b"\n\n"]), ["", ""]);
        assert_eq!(lines(&[b"\r\r"]), ["", ""]);
        // Bytes that are not UTF-8 survive as replacement characters.
        assert_eq!(lines(&[&[0x41, 0xFF, 0x0A]]), ["A\u{FFFD}"]);
        // A line never ends: it is cut at the limit.
        let long = vec![b'x'; MAX_LINE + 10];
        let out = lines(&[&long]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), MAX_LINE);
    }

    #[test]
    fn numbers_take_signs_points_exponents_and_leave_units() {
        assert_eq!(number_prefix("21.4°C"), Some((21.4, "°C")));
        assert_eq!(number_prefix("-0.5"), Some((-0.5, "")));
        assert_eq!(number_prefix("+3V"), Some((3.0, "V")));
        assert_eq!(number_prefix(".5"), Some((0.5, "")));
        assert_eq!(number_prefix("5."), Some((5.0, "")));
        assert_eq!(number_prefix("1e3W"), Some((1000.0, "W")));
        assert_eq!(number_prefix("2E-2"), Some((0.02, "")));
        // An `e` without digits is a unit, not an exponent.
        assert_eq!(number_prefix("4em"), Some((4.0, "em")));
        assert_eq!(number_prefix("12inH2O"), Some((12.0, "inH2O")));
        for not_a_number in ["", "-", ".", "+.", "nan", "inf", "abc", "e5"] {
            assert_eq!(number_prefix(not_a_number), None, "{not_a_number}");
        }
        // Too big for a float.
        assert_eq!(number_prefix("1e999"), None);
    }

    #[test]
    fn key_value_lines_make_points_with_units() {
        let mut parser = Parser::new(&LineParser::KeyValue).unwrap();
        let first = parser.line("pH=7.21pH ORP=245.3mV, Temperature: 15.8°C; time=12:30:01");
        let names: Vec<&str> = first
            .added
            .iter()
            .map(|point| point.name.as_str())
            .collect();
        assert_eq!(names, ["pH", "ORP", "Temperature", "time"]);
        let units: Vec<&str> = first
            .added
            .iter()
            .map(|point| point.unit.as_str())
            .collect();
        assert_eq!(
            units,
            ["", "mV", "°C", ""],
            "pH's unit is its name; 12:30:01 is no unit"
        );
        assert!(first.values);
        // The same keys again add nothing and update the values.
        let again = parser.line("ORP=250mV");
        assert!(again.added.is_empty() && again.values);
        let readings = parser.readings();
        assert_eq!(readings[1].value, 250.0);
        assert_eq!(readings[0].value, 7.21, "values not in the line stay");
        // Lines without pairs change nothing.
        assert_eq!(parser.line("OK"), Parsed::default());
        assert_eq!(parser.line("12:30 = 5"), Parsed::default());
        assert_eq!(parser.line("speed=fast"), Parsed::default());
    }

    #[test]
    fn csv_headers_name_the_columns_that_follow() {
        let mut parser = Parser::new(&LineParser::Csv {
            delimiter: ',',
            names: Vec::new(),
        })
        .unwrap();
        assert_eq!(
            parser.line("# Temp, Humidity , Pressure"),
            Parsed::default()
        );
        let parsed = parser.line("21.5,45, 1013.2");
        let names: Vec<&str> = parsed
            .added
            .iter()
            .map(|point| point.name.as_str())
            .collect();
        assert_eq!(names, ["Temp", "Humidity", "Pressure"]);
        // Missing and text fields are skipped; extra columns get numbered.
        let parsed = parser.line("22.0,,ERR,7");
        assert_eq!(parsed.added.len(), 1);
        assert_eq!(parsed.added[0].name, "Column 4");
        assert_eq!(parsed.added[0].id, 3);
        let readings = parser.readings();
        assert_eq!(readings[0].value, 22.0);
        assert_eq!(readings[1].value, 45.0, "an empty field keeps the value");
        assert_eq!(parser.line(" , ,"), Parsed::default());
    }

    #[test]
    fn csv_names_from_the_form_win_over_the_header() {
        let mut parser = Parser::new(&LineParser::Csv {
            delimiter: ';',
            names: vec!["Flow".into()],
        })
        .unwrap();
        parser.line("a;b");
        let parsed = parser.line("1.5;2");
        let names: Vec<&str> = parsed
            .added
            .iter()
            .map(|point| point.name.as_str())
            .collect();
        assert_eq!(names, ["Flow", "b"]);
        // With ; as the delimiter, 1,5 is no number: the column keeps its value.
        let parsed = parser.line("1,5;3");
        assert!(parsed.added.is_empty() && parsed.values);
        assert_eq!(parser.readings()[0].value, 1.5);
        assert_eq!(parser.readings()[1].value, 3.0);
    }

    #[test]
    fn regex_groups_become_points_by_name_or_number() {
        let mut parser = Parser::new(&LineParser::Regex {
            pattern: r"T=(?P<temp>-?[\d.]+) H=([\d.]+)( ALARM)?".into(),
        })
        .unwrap();
        let parsed = parser.line("T=-4.5 H=80");
        let names: Vec<&str> = parsed
            .added
            .iter()
            .map(|point| point.name.as_str())
            .collect();
        assert_eq!(names, ["temp", "Group 2"]);
        assert_eq!(parser.readings()[0].value, -4.5);
        assert_eq!(parser.line("no match"), Parsed::default());
        // A group that matched text, not a number, is skipped.
        let parsed = parser.line("T=1.0 H=2 ALARM");
        assert!(parsed.added.is_empty());
        assert_eq!(parser.readings().len(), 2);
    }

    #[test]
    fn patterns_must_compile_and_capture() {
        assert!(compile(r"T=(\d+)").is_ok());
        assert!(compile(r"T=\d+").unwrap_err().contains("capture group"));
        assert!(compile(r"T=(\d+").unwrap_err().contains("not valid"));
        assert!(
            Parser::new(&LineParser::Regex {
                pattern: "(".into()
            })
            .is_err()
        );
    }

    #[test]
    fn the_none_parser_makes_no_points() {
        let mut parser = Parser::new(&LineParser::None).unwrap();
        assert!(parser.is_none());
        assert_eq!(parser.line("T=21"), Parsed::default());
    }
}
