use anyhow::Context;
use std::fmt::Write as _;
use std::path::Path;

/// One `*.test.yaml` file's results.
pub struct Suite {
    pub name: String,
    pub cases: Vec<Case>,
}

/// One scenario (or a file-level load/compile failure).
pub struct Case {
    pub name: String,
    /// Assertion failures, rendered as `<failure>` elements.
    pub failures: Vec<String>,
    /// Setup/compile abort, rendered as an `<error>` element.
    pub error: Option<String>,
}

impl Case {
    pub fn failed(&self) -> bool {
        self.error.is_some() || !self.failures.is_empty()
    }
}

pub fn write_report(path: &Path, suites: &[Suite]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    std::fs::write(path, to_xml(suites))
        .with_context(|| format!("failed to write {}", path.display()))
}

pub fn to_xml(suites: &[Suite]) -> String {
    let tests: usize = suites.iter().map(|s| s.cases.len()).sum();
    let failures: usize = suites.iter().map(|s| s.cases.iter().filter(|c| !c.failures.is_empty()).count()).sum();
    let errors: usize = suites.iter().map(|s| s.cases.iter().filter(|c| c.error.is_some()).count()).sum();

    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    writeln!(
        xml,
        "<testsuites tests=\"{tests}\" failures=\"{failures}\" errors=\"{errors}\">"
    )
    .unwrap();
    for suite in suites {
        writeln!(
            xml,
            "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\">",
            escape(&suite.name),
            suite.cases.len(),
            suite.cases.iter().filter(|c| !c.failures.is_empty()).count(),
            suite.cases.iter().filter(|c| c.error.is_some()).count(),
        )
        .unwrap();
        for case in &suite.cases {
            writeln!(
                xml,
                "    <testcase name=\"{}\" classname=\"{}\" time=\"0\">",
                escape(&case.name),
                escape(&suite.name)
            )
            .unwrap();
            if let Some(err) = &case.error {
                writeln!(
                    xml,
                    "      <error message=\"{}\" type=\"setup\">{}</error>",
                    escape(err),
                    escape(err)
                )
                .unwrap();
            }
            for failure in &case.failures {
                writeln!(
                    xml,
                    "      <failure message=\"{}\" type=\"assertion\">{}</failure>",
                    escape(first_line(failure)),
                    escape(failure)
                )
                .unwrap();
            }
            xml.push_str("    </testcase>\n");
        }
        xml.push_str("  </testsuite>\n");
    }
    xml.push_str("</testsuites>\n");
    xml
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Suite> {
        vec![Suite {
            name: "TankLevel.test.yaml".into(),
            cases: vec![
                Case {
                    name: "low level opens inlet".into(),
                    failures: vec![],
                    error: None,
                },
                Case {
                    name: "fails <hard> & 'loud'".into(),
                    failures: vec!["level: expected 91 ±0.001, got 90.00 (REAL)\nsecond line".into()],
                    error: None,
                },
            ],
        }]
    }

    #[test]
    fn xml_shape_counts_and_escaping() {
        let xml = to_xml(&sample());
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
        assert!(xml.contains("<testsuites tests=\"2\" failures=\"1\" errors=\"0\">"));
        assert!(xml.contains(
            "<testsuite name=\"TankLevel.test.yaml\" tests=\"2\" failures=\"1\" errors=\"0\">"
        ));
        assert!(xml.contains("<testcase name=\"low level opens inlet\" classname=\"TankLevel.test.yaml\" time=\"0\">"));
        assert!(
            xml.contains("<failure message=\"level: expected 91 ±0.001, got 90.00 (REAL)\" type=\"assertion\">"),
            "failure message uses the first line only"
        );
        assert!(xml.contains("&lt;hard&gt; &amp; &apos;loud&apos;"));
        assert!(!xml.contains("<hard>"));
        assert!(xml.contains("second line"), "full failure body is kept");
        assert!(xml.ends_with("</testsuites>\n"));
    }

    #[test]
    fn error_case_renders_error_element() {
        let suites = vec![Suite {
            name: "broken.test.yaml".into(),
            cases: vec![Case {
                name: "load".into(),
                failures: vec![],
                error: Some("invalid scenario YAML: bad token".into()),
            }],
        }];
        let xml = to_xml(&suites);
        assert!(xml.contains("<testsuites tests=\"1\" failures=\"0\" errors=\"1\">"));
        assert!(xml.contains(
            "<error message=\"invalid scenario YAML: bad token\" type=\"setup\">invalid scenario YAML: bad token</error>"
        ));
    }

    #[test]
    fn write_report_creates_parent_directories() {
        let dir = std::env::temp_dir().join(format!("mt-plc-cli-junit-{}", std::process::id()));
        let path = dir.join("nested/report.xml");
        write_report(&path, &sample()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("</testsuites>"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
