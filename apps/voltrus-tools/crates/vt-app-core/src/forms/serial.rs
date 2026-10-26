//! Serial monitor and virtual serial device forms.

use vt_protocol::{
    FlowControl, LineEnding, LineParser, Parity, SerialDeviceSpec, SerialFormat, SerialLink,
    SerialMacro, SerialMonitorSpec, encode_send,
};

use super::{Field, FormError, error, name, number};

/// Which of a macro row's fields a problem is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroField {
    Name,
    Payload,
}

/// The line parser picked in the form; its settings are separate fields so
/// switching kinds keeps what was typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserKind {
    None,
    KeyValue,
    Csv,
    Regex,
}

impl ParserKind {
    pub const ALL: [ParserKind; 4] = [Self::None, Self::KeyValue, Self::Csv, Self::Regex];

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "Terminal only",
            Self::KeyValue => "Key=value",
            Self::Csv => "CSV",
            Self::Regex => "Regex",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacroForm {
    pub name: String,
    pub payload: String,
    pub hex: bool,
}

impl MacroForm {
    pub fn blank() -> Self {
        Self {
            name: String::new(),
            payload: String::new(),
            hex: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialMonitorForm {
    pub name: String,
    pub port: String,
    pub baud: String,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
    pub flow: FlowControl,
    pub line_ending: LineEnding,
    pub parser: ParserKind,
    /// CSV: one character; `\t` or `tab` for a tab.
    pub delimiter: String,
    /// CSV: column names, comma-separated, in order.
    pub columns: String,
    pub pattern: String,
    pub macros: Vec<MacroForm>,
}

/// Baud rates offered as presets.
pub const BAUD_PRESETS: [u32; 6] = [9_600, 19_200, 38_400, 57_600, 115_200, 230_400];

impl SerialMonitorForm {
    pub fn from_spec(name: &str, spec: &SerialMonitorSpec) -> Self {
        let (parser, delimiter, columns, pattern) = match &spec.parser {
            LineParser::None => (
                ParserKind::None,
                ",".to_owned(),
                String::new(),
                String::new(),
            ),
            LineParser::KeyValue => (
                ParserKind::KeyValue,
                ",".to_owned(),
                String::new(),
                String::new(),
            ),
            LineParser::Csv { delimiter, names } => (
                ParserKind::Csv,
                if *delimiter == '\t' {
                    "\\t".to_owned()
                } else {
                    delimiter.to_string()
                },
                names.join(", "),
                String::new(),
            ),
            LineParser::Regex { pattern } => (
                ParserKind::Regex,
                ",".to_owned(),
                String::new(),
                pattern.clone(),
            ),
        };
        Self {
            name: name.to_owned(),
            port: spec.link.port.clone(),
            baud: spec.link.baud.to_string(),
            data_bits: spec.link.data_bits,
            parity: spec.link.parity,
            stop_bits: spec.link.stop_bits,
            flow: spec.flow,
            line_ending: spec.line_ending,
            parser,
            delimiter,
            columns,
            pattern,
            macros: spec
                .macros
                .iter()
                .map(|item| MacroForm {
                    name: item.name.clone(),
                    payload: item.payload.clone(),
                    hex: item.hex,
                })
                .collect(),
        }
    }

    pub fn build(&self) -> Result<(String, SerialMonitorSpec), FormError> {
        let name = name(&self.name)?;
        let port = self.port.trim();
        if port.is_empty() {
            return Err(error(
                Field::SerialPort,
                "Pick a port from the list or type its path, e.g. /dev/cu.usbserial-0001",
            ));
        }
        let baud = number(&self.baud, Field::Baud, "Baud rate", 50, 4_000_000)?;
        if !(5..=8).contains(&self.data_bits) {
            return Err(error(Field::DataBits, "Data bits are 5, 6, 7 or 8"));
        }
        if !(1..=2).contains(&self.stop_bits) {
            return Err(error(Field::StopBits, "Stop bits are 1 or 2"));
        }
        let parser = match self.parser {
            ParserKind::None => LineParser::None,
            ParserKind::KeyValue => LineParser::KeyValue,
            ParserKind::Csv => {
                let mut names: Vec<String> = self
                    .columns
                    .split(',')
                    .map(|column| column.trim().to_owned())
                    .collect();
                while names.last().is_some_and(String::is_empty) {
                    names.pop();
                }
                LineParser::Csv {
                    delimiter: delimiter(&self.delimiter)?,
                    names,
                }
            }
            ParserKind::Regex => {
                let pattern = self.pattern.trim();
                check_pattern(pattern)?;
                LineParser::Regex {
                    pattern: pattern.to_owned(),
                }
            }
        };
        let mut macros = Vec::with_capacity(self.macros.len());
        for (index, row) in self.macros.iter().enumerate() {
            let label = row.name.trim();
            if label.is_empty() {
                return Err(error(
                    Field::Macro(index, MacroField::Name),
                    "Name the macro, e.g. Read now",
                ));
            }
            encode_send(&row.payload, row.hex, self.line_ending).map_err(|problem| {
                error(
                    Field::Macro(index, MacroField::Payload),
                    format!("{label}: {problem}"),
                )
            })?;
            macros.push(SerialMacro {
                name: label.to_owned(),
                payload: row.payload.clone(),
                hex: row.hex,
            });
        }
        Ok((
            name,
            SerialMonitorSpec {
                link: SerialLink {
                    port: port.to_owned(),
                    baud,
                    data_bits: self.data_bits,
                    parity: self.parity,
                    stop_bits: self.stop_bits,
                },
                flow: self.flow,
                line_ending: self.line_ending,
                parser,
                macros,
            },
        ))
    }
}

fn delimiter(text: &str) -> Result<char, FormError> {
    match text {
        "\\t" | "tab" | "\t" => return Ok('\t'),
        _ => {}
    }
    let mut chars = text.trim().chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_alphanumeric() && c != '.' && c != '-' => Ok(c),
        _ => Err(error(
            Field::Delimiter,
            "The delimiter is one character that is not part of a number: , ; | or \\t",
        )),
    }
}

/// A line pattern compiles and captures at least one group.
fn check_pattern(pattern: &str) -> Result<(), FormError> {
    if pattern.is_empty() {
        return Err(error(
            Field::Pattern,
            "Enter a pattern with a group around each number, e.g. T=(\\d+\\.\\d+)",
        ));
    }
    let regex = regex::Regex::new(pattern).map_err(|problem| {
        error(
            Field::Pattern,
            format!("The pattern is not valid: {problem}"),
        )
    })?;
    if regex.captures_len() < 2 {
        return Err(error(
            Field::Pattern,
            "Put a capture group around each number, e.g. T=(\\d+\\.\\d+)",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialDeviceForm {
    pub name: String,
    pub template: String,
    pub format: SerialFormat,
    pub interval_ms: String,
    pub alias: String,
}

impl SerialDeviceForm {
    pub fn from_spec(name: &str, spec: &SerialDeviceSpec) -> Self {
        Self {
            name: name.to_owned(),
            template: spec.template.clone(),
            format: spec.format,
            interval_ms: spec.interval_ms.to_string(),
            alias: spec.alias.clone(),
        }
    }

    pub fn build(&self) -> Result<(String, SerialDeviceSpec), FormError> {
        let name = name(&self.name)?;
        let interval_ms = number(
            &self.interval_ms,
            Field::IntervalMs,
            "Interval",
            SerialDeviceSpec::MIN_INTERVAL_MS,
            3_600_000,
        )?;
        let alias = self.alias.trim();
        if alias.is_empty()
            || !alias
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            || alias.starts_with('.')
        {
            return Err(error(
                Field::Alias,
                "Name the port with letters, digits, - _ and . only, e.g. voltrus-serial-1",
            ));
        }
        if self.template.trim().is_empty() {
            return Err(error(Field::Template, "Pick the instrument to play"));
        }
        Ok((
            name,
            SerialDeviceSpec {
                template: self.template.trim().to_owned(),
                format: self.format,
                interval_ms,
                alias: alias.to_owned(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_forms_round_trip_to_the_same_specs() {
        let monitor = SerialMonitorSpec {
            link: SerialLink {
                port: "/dev/cu.usbserial-1".into(),
                ..SerialMonitorSpec::default().link
            },
            ..SerialDeviceSpec::default().monitor()
        };
        assert_eq!(
            SerialMonitorForm::from_spec("Logger", &monitor).build(),
            Ok(("Logger".to_owned(), monitor))
        );
        let device = SerialDeviceSpec::default();
        assert_eq!(
            SerialDeviceForm::from_spec("Analyser", &device).build(),
            Ok(("Analyser".to_owned(), device))
        );
        // Tabs survive the trip through the text field.
        let tabs = SerialMonitorSpec {
            link: monitor_link(),
            parser: LineParser::Csv {
                delimiter: '\t',
                names: vec!["Flow".into(), "Level".into()],
            },
            ..SerialMonitorSpec::default()
        };
        assert_eq!(
            SerialMonitorForm::from_spec("Tabs", &tabs).build(),
            Ok(("Tabs".to_owned(), tabs))
        );
    }

    fn monitor_link() -> SerialLink {
        SerialLink {
            port: "/dev/cu.usbmodem1".into(),
            ..SerialMonitorSpec::default().link
        }
    }

    fn form() -> SerialMonitorForm {
        SerialMonitorForm::from_spec(
            "Logger",
            &SerialMonitorSpec {
                link: monitor_link(),
                ..SerialMonitorSpec::default()
            },
        )
    }

    #[test]
    fn monitor_errors_point_at_the_offending_field() {
        let mut bad = form();
        bad.port = "  ".into();
        assert_eq!(bad.build().unwrap_err().field, Field::SerialPort);

        let mut bad = form();
        bad.baud = "12".into();
        assert_eq!(bad.build().unwrap_err().field, Field::Baud);

        let mut bad = form();
        bad.data_bits = 9;
        assert_eq!(bad.build().unwrap_err().field, Field::DataBits);

        let mut bad = form();
        bad.stop_bits = 3;
        assert_eq!(bad.build().unwrap_err().field, Field::StopBits);

        for delimiter in ["", ",;", "a", "5", ".", "-"] {
            let mut bad = form();
            bad.parser = ParserKind::Csv;
            bad.delimiter = delimiter.into();
            assert_eq!(
                bad.build().unwrap_err().field,
                Field::Delimiter,
                "{delimiter:?}"
            );
        }

        for pattern in ["", "T=\\d+", "T=(\\d+"] {
            let mut bad = form();
            bad.parser = ParserKind::Regex;
            bad.pattern = pattern.into();
            assert_eq!(
                bad.build().unwrap_err().field,
                Field::Pattern,
                "{pattern:?}"
            );
        }
    }

    #[test]
    fn a_pattern_only_matters_when_the_parser_uses_it() {
        let mut form = form();
        form.pattern = "(".into();
        form.parser = ParserKind::KeyValue;
        assert!(form.build().is_ok());
        form.parser = ParserKind::Regex;
        form.pattern = r" T=(?P<temp>-?\d+(\.\d+)?) ".into();
        let (_, spec) = form.build().unwrap();
        assert_eq!(
            spec.parser,
            LineParser::Regex {
                pattern: r"T=(?P<temp>-?\d+(\.\d+)?)".into()
            }
        );
    }

    #[test]
    fn csv_names_drop_trailing_blanks_but_keep_gaps() {
        let mut form = form();
        form.parser = ParserKind::Csv;
        form.delimiter = ";".into();
        form.columns = "Flow, , Level, ,".into();
        let (_, spec) = form.build().unwrap();
        assert_eq!(
            spec.parser,
            LineParser::Csv {
                delimiter: ';',
                names: vec!["Flow".into(), String::new(), "Level".into()],
            }
        );
    }

    #[test]
    fn macros_need_a_name_and_a_payload_that_encodes() {
        let mut bad = form();
        bad.macros.push(MacroForm {
            name: " ".into(),
            payload: "READ".into(),
            hex: false,
        });
        assert_eq!(
            bad.build().unwrap_err().field,
            Field::Macro(0, MacroField::Name)
        );

        let mut bad = form();
        bad.macros.push(MacroForm {
            name: "Poll".into(),
            payload: "01 03 0G".into(),
            hex: true,
        });
        let problem = bad.build().unwrap_err();
        assert_eq!(problem.field, Field::Macro(0, MacroField::Payload));
        assert!(problem.message.starts_with("Poll: "), "{}", problem.message);

        let mut bad = form();
        bad.macros.push(MacroForm {
            name: "Escape".into(),
            payload: "\\q".into(),
            hex: false,
        });
        assert_eq!(
            bad.build().unwrap_err().field,
            Field::Macro(0, MacroField::Payload)
        );

        let mut good = form();
        good.macros.push(MacroForm {
            name: " Poll ".into(),
            payload: "01 03 00 00 00 0A C5 CD".into(),
            hex: true,
        });
        let (_, spec) = good.build().unwrap();
        assert_eq!(spec.macros[0].name, "Poll");
    }

    #[test]
    fn device_aliases_stay_inside_the_temporary_folder() {
        let form = || SerialDeviceForm::from_spec("Sim", &SerialDeviceSpec::default());
        for alias in ["", "../etc/x", "a/b", ".hidden", "with space"] {
            let mut bad = form();
            bad.alias = alias.into();
            assert_eq!(bad.build().unwrap_err().field, Field::Alias, "{alias:?}");
        }
        let mut bad = form();
        bad.interval_ms = "10".into();
        assert_eq!(bad.build().unwrap_err().field, Field::IntervalMs);
        let mut bad = form();
        bad.template = String::new();
        assert_eq!(bad.build().unwrap_err().field, Field::Template);
    }
}
