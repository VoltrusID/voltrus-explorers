use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Instant;

use crate::check::{collect_files, rel_path};
use crate::junit::write_report;
use crate::runner::{run_scenario, Assertion};
use crate::scenario::{ScenarioFile, DEFAULT_SCAN_MS};

/// How one scenario ended, serialized lowercase (`passed`/`failed`/`error`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScenarioOutcome {
    Passed,
    Failed,
    Error,
}

/// Top-level result of a `test` run: totals plus one suite per YAML file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TestReport {
    pub files: usize,
    pub scenarios: usize,
    pub failures: usize,
    /// Total wall-clock time of the run (the simulation itself is virtual
    /// time).
    pub duration_ms: u64,
    /// `true` when no scenario failed and no fatal error occurred.
    pub passed: bool,
    /// One suite per `*.test.yaml` file, sorted by path.
    pub suites: Vec<SuiteReport>,
    /// Fatal setup problem (unreadable directory, failed JUnit write).
    #[serde(default)]
    pub error: Option<String>,
}

/// One `*.test.yaml` file's results.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SuiteReport {
    pub name: String,
    pub cases: Vec<ScenarioReport>,
}

/// One scenario (or a file-level load/compile failure, reported as a
/// synthetic `load` case so JUnit XML keeps a `<testcase>` per error).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ScenarioReport {
    pub name: String,
    pub outcome: ScenarioOutcome,
    /// Wall-clock duration of the scenario run.
    pub duration_ms: u64,
    /// Assertion failure messages, rendered as `<failure>` elements.
    pub failures: Vec<String>,
    /// Setup/compile abort, rendered as an `<error>` element.
    pub error: Option<String>,
    /// Per-assertion details (one entry per checked matcher).
    #[serde(default)]
    pub assertions: Vec<Assertion>,
}

/// Run every `*.test.yaml` below `dir`. Each file targets one program
/// (`program:` next to the YAML); each scenario gets a freshly initialized
/// VM, so scenarios are independent. I/O and JUnit-write problems are
/// folded into `TestReport::error` instead of aborting the run.
pub fn run(dir: &Path, junit_path: Option<&Path>) -> TestReport {
    let started = Instant::now();
    let yaml_files = match collect_files(dir, |p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().to_lowercase().ends_with(".test.yaml"))
    }) {
        Ok(files) => files,
        Err(err) => {
            return TestReport {
                files: 0,
                scenarios: 0,
                failures: 0,
                duration_ms: 0,
                passed: false,
                suites: Vec::new(),
                error: Some(format!("{err:#}")),
            }
        }
    };

    let mut suites = Vec::new();
    let mut scenarios = 0usize;
    let mut failures = 0usize;

    for yaml_path in &yaml_files {
        let suite = run_file(yaml_path, &rel_path(dir, yaml_path));
        for case in &suite.cases {
            scenarios += 1;
            failures += usize::from(case.outcome != ScenarioOutcome::Passed);
        }
        suites.push(suite);
    }

    let mut error = None;
    if let Some(path) = junit_path {
        if let Err(err) = write_report(path, &suites) {
            error = Some(format!("{err:#}"));
        }
    }

    let passed = failures == 0 && error.is_none();
    TestReport {
        files: yaml_files.len(),
        scenarios,
        failures,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        passed,
        suites,
        error,
    }
}

fn run_file(yaml_path: &Path, rel: &str) -> SuiteReport {
    let error_case = |message: String| SuiteReport {
        name: rel.to_string(),
        cases: vec![ScenarioReport {
            name: "load".into(),
            outcome: ScenarioOutcome::Error,
            duration_ms: 0,
            failures: Vec::new(),
            error: Some(message),
            assertions: Vec::new(),
        }],
    };

    let raw = match std::fs::read_to_string(yaml_path) {
        Ok(raw) => raw,
        Err(err) => {
            return error_case(format!("failed to read {}: {err}", yaml_path.display()))
        }
    };
    let file: ScenarioFile = match serde_yaml::from_str(&raw) {
        Ok(file) => file,
        Err(err) => return error_case(format!("invalid scenario YAML: {err}")),
    };
    let scan_ms = match file.scan_ms.unwrap_or(DEFAULT_SCAN_MS) {
        0 => return error_case("scan_ms must be greater than zero".into()),
        scan_ms => scan_ms,
    };

    let program_path = yaml_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(&file.program);
    let source = match std::fs::read_to_string(&program_path) {
        Ok(source) => source,
        Err(err) => {
            return error_case(format!(
                "failed to read program {}: {err}",
                program_path.display()
            ))
        }
    };
    let st_source = match crate::diagram::DiagramKind::from_extension(
        &program_path
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default(),
    ) {
        Some(kind) => match crate::diagram::transpile_source(kind, &source) {
            Ok(result)
                if !result.source.is_empty()
                    && !result.diagnostics.iter().any(|d| d.severity == "error") =>
            {
                result.source
            }
            Ok(result) => {
                let detail = result
                    .diagnostics
                    .iter()
                    .map(|d| format!("  {}: 1:1: {}: {}", file.program, d.message, d.severity))
                    .collect::<Vec<_>>()
                    .join("\n");
                return error_case(format!(
                    "program {} failed to transpile{}",
                    file.program,
                    if detail.is_empty() { String::new() } else { format!("\n{detail}") }
                ));
            }
            Err(err) => {
                return error_case(format!("program {} failed to parse: {err}", file.program))
            }
        },
        None => source,
    };

    let compiled = mt_stc::compile(&st_source);
    let module = match compiled.module {
        Some(module) if compiled.success => module,
        _ => {
            let detail = compiled
                .diagnostics
                .iter()
                .filter(|d| d.severity == "error")
                .map(|d| format!("  {}:{}:{}: {}", file.program, d.line, d.col, d.message))
                .collect::<Vec<_>>()
                .join("\n");
            return error_case(format!(
                "program {} failed to compile{}",
                file.program,
                if detail.is_empty() { String::new() } else { format!("\n{detail}") }
            ));
        }
    };
    let cases = file
        .scenarios
        .iter()
        .map(|scenario| {
            let result = run_scenario(scenario, &module, scan_ms);
            let outcome = if result.passed() {
                ScenarioOutcome::Passed
            } else {
                ScenarioOutcome::Failed
            };
            ScenarioReport {
                name: result.name,
                outcome,
                duration_ms: result.duration_ms,
                failures: result.failures,
                error: None,
                assertions: result.assertions,
            }
        })
        .collect();

    SuiteReport {
        name: rel.to_string(),
        cases,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTU_PROGRAM: &str = r#"
PROGRAM PulseTrain
VAR
    Pulse : BOOL := FALSE;
    Tally : CTU;
END_VAR
    Tally(CU := Pulse, R := FALSE, PV := 10);
END_PROGRAM
"#;

    const THREE_PULSE_YAML: &str = r#"
program: PulseTrain.st
scenarios:
  - name: three set pulses reach CV 3
    steps:
      - set: {Pulse: true}
        advance: 1scans
      - set: {Pulse: false}
        advance: 1scans
      - set: {Pulse: true}
        advance: 1scans
      - set: {Pulse: false}
        advance: 1scans
      - set: {Pulse: true}
        advance: 1scans
      - set: {Pulse: false}
        advance: 1scans
    expect:
      Tally.CV: 3
"#;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mt-plc-cli-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn run_collects_structured_report_for_real_program() {
        let dir = temp_dir("harness");
        std::fs::write(dir.join("PulseTrain.st"), CTU_PROGRAM).unwrap();
        std::fs::write(dir.join("PulseTrain.test.yaml"), THREE_PULSE_YAML).unwrap();

        let report = run(&dir, None);
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.files, 1);
        assert_eq!(report.scenarios, 1);
        assert_eq!(report.failures, 0);
        assert!(report.passed);

        let case = &report.suites[0].cases[0];
        assert_eq!(case.name, "three set pulses reach CV 3");
        assert_eq!(case.outcome, ScenarioOutcome::Passed);
        assert!(case.error.is_none());
        // The final `expect` records per-assertion detail.
        let cv = case
            .assertions
            .iter()
            .find(|a| a.name == "Tally.CV")
            .expect("CV assertion recorded");
        assert!(cv.passed);
        assert_eq!(cv.expected, "3");
        assert_eq!(cv.actual, "3 (DINT)");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn report_json_round_trips() {
        let report = TestReport {
            files: 1,
            scenarios: 1,
            failures: 1,
            duration_ms: 12,
            passed: false,
            suites: vec![SuiteReport {
                name: "Counter.test.yaml".into(),
                cases: vec![ScenarioReport {
                    name: "premature expectation".into(),
                    outcome: ScenarioOutcome::Failed,
                    duration_ms: 12,
                    failures: vec!["Tally.CV: expected 3, got 1 (DINT)".into()],
                    error: None,
                    assertions: vec![Assertion {
                        name: "Tally.CV".into(),
                        passed: false,
                        expected: "3".into(),
                        actual: "1 (DINT)".into(),
                    }],
                }],
            }],
            error: None,
        };

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"outcome\":\"failed\""));
        assert!(json.contains("\"duration_ms\":12"));
        assert!(json.contains("\"expected\":\"3\""));
        assert!(json.contains("\"actual\":\"1 (DINT)\""));

        let round: TestReport = serde_json::from_str(&json).unwrap();
        assert_eq!(round, report);
    }

    #[test]
    fn junit_write_failure_is_folded_into_error() {
        let dir = temp_dir("junitfail");
        std::fs::write(dir.join("PulseTrain.st"), CTU_PROGRAM).unwrap();
        std::fs::write(dir.join("PulseTrain.test.yaml"), THREE_PULSE_YAML).unwrap();
        // A directory path cannot be written as a file.
        let junit_target = dir.join("blocked");
        std::fs::create_dir(&junit_target).unwrap();

        let report = run(&dir, Some(&junit_target));
        assert!(report.error.is_some(), "expected junit write failure");
        assert!(!report.passed);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
