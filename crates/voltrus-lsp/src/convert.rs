//! Maps `mt-stc` diagnostics to LSP diagnostics.
//!
//! `mt-stc` reports positions as (1-based line, 0-based char-count column,
//! counted per line). LSP wants (0-based line, 0-based column in UTF-16 code
//! units). Columns are converted by taking the first `col` chars of the line
//! and summing their UTF-16 lengths — identical for plain ASCII ST sources,
//! correct for any non-ASCII content.

use tower_lsp::lsp_types;

/// A line-split view of a source text, used for all position conversion.
pub struct LineIndex<'a> {
    lines: Vec<&'a str>,
    /// Byte offset of each line start (parallel to `lines`).
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(text: &'a str) -> Self {
        let lines: Vec<&'a str> = text.split('\n').collect();
        let mut starts = Vec::with_capacity(lines.len());
        let mut off = 0usize;
        for line in &lines {
            starts.push(off);
            off += line.len() + 1; // +1 for the '\n'
        }
        Self { lines, starts }
    }

    /// Convert an `mt-stc` (1-based line, char-count col) pair to an LSP
    /// position. Out-of-range inputs clamp to the last line / line end.
    pub fn position(&self, line1: usize, col_chars: usize) -> lsp_types::Position {
        let idx = line1.saturating_sub(1).min(self.lines.len() - 1);
        let utf16: usize = self.lines[idx]
            .chars()
            .take(col_chars)
            .map(char::len_utf16)
            .sum();
        lsp_types::Position {
            line: idx as u32,
            character: utf16 as u32,
        }
    }

    /// Convert an `mt-stc` (line, col, end_line, end_col) quadruple to an LSP range.
    pub fn range(
        &self,
        line1: usize,
        col_chars: usize,
        end_line1: usize,
        end_col_chars: usize,
    ) -> lsp_types::Range {
        lsp_types::Range {
            start: self.position(line1, col_chars),
            end: self.position(end_line1, end_col_chars),
        }
    }

    /// LSP position for a byte offset in the original text.
    pub fn position_from_offset(&self, offset: usize) -> lsp_types::Position {
        let idx = self
            .starts
            .partition_point(|&s| s <= offset)
            .saturating_sub(1);
        let line = self.lines[idx];
        let chars_before = line[..offset.saturating_sub(self.starts[idx]).min(line.len())]
            .chars()
            .count();
        let utf16: usize = line
            .chars()
            .take(chars_before)
            .map(char::len_utf16)
            .sum();
        lsp_types::Position {
            line: idx as u32,
            character: utf16 as u32,
        }
    }

    /// 1-based line containing a byte offset (mt-stc's line convention).
    pub fn line_of(&self, offset: usize) -> u32 {
        self.position_from_offset(offset).line + 1
    }

    /// LSP range for an `iecst` span (byte offsets into the same text).
    pub fn range_from_span(&self, span: iecst::Span) -> lsp_types::Range {
        lsp_types::Range {
            start: self.position_from_offset(span.start),
            end: self.position_from_offset(span.end),
        }
    }
}

/// Convert one `mt-stc` diagnostic to an LSP diagnostic over `text`.
pub fn to_lsp(d: &mt_stc::Diagnostic, text: &str) -> lsp_types::Diagnostic {
    let li = LineIndex::new(text);
    let severity = match d.severity.as_str() {
        "error" => lsp_types::DiagnosticSeverity::ERROR,
        "warning" => lsp_types::DiagnosticSeverity::WARNING,
        _ => lsp_types::DiagnosticSeverity::INFORMATION,
    };
    lsp_types::Diagnostic {
        range: li.range(d.line, d.col, d.end_line, d.end_col),
        severity: Some(severity),
        message: d.message.clone(),
        source: Some(format!("mt-stc:{}", d.source)),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_positions_pass_through() {
        let li = LineIndex::new("abc\n    boom := 1;");
        // mt-stc reports positions as 1-based line + char-count col; here
        // line 2 (1-based), col 4 (chars) points at `boom`.
        let p = li.position(2, 4);
        assert_eq!((p.line, p.character), (1, 4));
        let r = li.range(2, 4, 2, 8);
        assert_eq!(r.start.character, 4);
        assert_eq!(r.end.character, 8);
    }

    #[test]
    fn utf16_columns_account_for_astral_chars() {
        // 🚀 is one char but two UTF-16 code units.
        let li = LineIndex::new("🚀x");
        let p = li.position(1, 2); // char col 2 = past 🚀 and x
        assert_eq!(p.character, 3);
    }

    #[test]
    fn out_of_range_clamps() {
        let li = LineIndex::new("ab\ncd");
        let p = li.position(99, 999);
        assert_eq!((p.line, p.character), (1, 2));
    }

    #[test]
    fn parse_error_default_position_maps_to_zero() {
        // mt-stc parser errors hardcode line 1, col 1.
        let li = LineIndex::new("garbage");
        let p = li.position(1, 1);
        assert_eq!((p.line, p.character), (0, 1));
    }

    #[test]
    fn offsets_convert_to_positions_and_lines() {
        let text = "abc\n  boom := 1;\nEND_PROGRAM";
        let li = LineIndex::new(text);
        let boom_off = text.find("boom").unwrap();
        let p = li.position_from_offset(boom_off);
        assert_eq!((p.line, p.character), (1, 2));
        assert_eq!(li.line_of(boom_off), 2);
        assert_eq!(li.line_of(0), 1);
        let span = iecst::Span::new(boom_off, boom_off + 4);
        let r = li.range_from_span(span);
        assert_eq!(r.end.character, 6);
    }
}
