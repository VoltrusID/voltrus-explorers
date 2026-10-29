//! Incremental parser for the `multipart/x-mixed-replace` (or `multipart/mixed`) bodies an
//! MTConnect agent sends for `?interval=` requests.

/// Extract the multipart boundary from a `Content-Type` header value, if it is a multipart type.
pub fn boundary_from_content_type(content_type: &str) -> Option<String> {
    let mut parts = content_type.split(';');
    let mime = parts.next()?.trim().to_ascii_lowercase();
    if !mime.starts_with("multipart/") {
        return None;
    }
    for p in parts {
        let p = p.trim();
        if p.len() > 9 && p[..9].eq_ignore_ascii_case("boundary=") {
            let v = p[9..].trim().trim_matches('"');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() || from > hay.len() - needle.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

/// Splits a stream of bytes into multipart part bodies.
pub struct MultipartParser {
    delimiter: Vec<u8>,
    buf: Vec<u8>,
    finished: bool,
}

impl MultipartParser {
    pub fn new(boundary: &str) -> Self {
        Self { delimiter: format!("--{boundary}").into_bytes(), buf: Vec::new(), finished: false }
    }

    /// Append received bytes.
    pub fn push(&mut self, data: &[u8]) {
        if !self.finished {
            self.buf.extend_from_slice(data);
        }
    }

    /// True once the closing `--boundary--` delimiter was seen.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Next complete part body, or `None` if more data is needed. Parts declaring
    /// `Content-Length` complete as soon as their body has arrived; parts without it complete
    /// at the next delimiter.
    pub fn next_part(&mut self) -> Option<Vec<u8>> {
        if self.finished {
            return None;
        }
        let start = find(&self.buf, &self.delimiter, 0)?;
        let after = start + self.delimiter.len();
        // `--boundary--` closes the stream; otherwise the delimiter line ends at LF.
        if self.buf.len() < after + 2 {
            return None;
        }
        if &self.buf[after..after + 2] == b"--" {
            self.finished = true;
            self.buf.clear();
            return None;
        }
        let line_end = find(&self.buf, b"\n", after)?;
        let headers_start = line_end + 1;

        let rest = &self.buf[headers_start..];
        // A blank line right after the delimiter means the part has no headers.
        let (header_len, body_offset) = if rest.starts_with(b"\r\n") {
            (0, 2)
        } else if rest.starts_with(b"\n") {
            (0, 1)
        } else if let Some(i) = find(rest, b"\r\n\r\n", 0) {
            (i, i + 4)
        } else {
            let i = find(rest, b"\n\n", 0)?;
            (i, i + 2)
        };
        let header_text = String::from_utf8_lossy(&rest[..header_len]).into_owned();
        let body_start = headers_start + body_offset;
        let content_length = header_text.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case("content-length").then(|| v.trim().parse::<usize>().ok())?
        });

        let (body_end, consumed) = match content_length {
            Some(n) => {
                if self.buf.len() < body_start + n {
                    return None;
                }
                (body_start + n, body_start + n)
            }
            None => {
                let next = find(&self.buf, &self.delimiter, body_start)?;
                // Strip the CRLF (or LF) that precedes the next delimiter.
                let mut end = next;
                if end > body_start && self.buf[end - 1] == b'\n' {
                    end -= 1;
                    if end > body_start && self.buf[end - 1] == b'\r' {
                        end -= 1;
                    }
                }
                (end, next)
            }
        };
        let body = self.buf[body_start..body_end].to_vec();
        self.buf.drain(..consumed);
        Some(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_extraction() {
        assert_eq!(boundary_from_content_type("multipart/mixed;boundary=abc-1").as_deref(), Some("abc-1"));
        assert_eq!(
            boundary_from_content_type("multipart/x-mixed-replace; Boundary=\"xyz\"").as_deref(),
            Some("xyz")
        );
        assert_eq!(boundary_from_content_type("text/xml"), None);
        assert_eq!(boundary_from_content_type("multipart/mixed"), None);
    }

    fn part(b: &str, body: &str) -> String {
        format!("--{b}\r\nContent-type: text/xml\r\nContent-length: {}\r\n\r\n{body}\r\n\r\n", body.len())
    }

    #[test]
    fn parts_split_across_arbitrary_chunks() {
        let stream = format!("{}{}", part("B", "<a>1</a>"), part("B", "<b>two\r\n\r\nlines</b>"));
        for chunk in [1usize, 3, 7, 64, 10_000] {
            let mut p = MultipartParser::new("B");
            let mut got = Vec::new();
            for c in stream.as_bytes().chunks(chunk) {
                p.push(c);
                while let Some(b) = p.next_part() {
                    got.push(String::from_utf8(b).unwrap());
                }
            }
            assert_eq!(got, vec!["<a>1</a>", "<b>two\r\n\r\nlines</b>"], "chunk size {chunk}");
        }
    }

    #[test]
    fn parts_without_content_length_end_at_next_delimiter() {
        let mut p = MultipartParser::new("B");
        p.push(b"--B\r\nContent-type: text/xml\r\n\r\n<a/>\r\n--B\r\nContent-type: text/xml\r\n\r\n<b/>\r\n--B--\r\n");
        assert_eq!(p.next_part().unwrap(), b"<a/>");
        assert_eq!(p.next_part().unwrap(), b"<b/>");
        assert_eq!(p.next_part(), None);
        assert!(p.is_finished());
    }
}
