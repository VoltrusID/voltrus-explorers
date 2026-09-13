//! Reusable core of the `voltrus-plc` CLI: headless ST source checks and
//! virtual-time scenario test runs. The binary in `main.rs` is a thin clap
//! wrapper over [`check_dir`] and [`test_dir`]; the voltrus-plc IDE links
//! the same entry points.

pub mod check;
pub mod junit;
pub mod diagram;
pub mod runner;
pub mod scenario;

mod harness;

pub use check::{CheckReport, Diagnostic};
pub use harness::{ScenarioOutcome, ScenarioReport, SuiteReport, TestReport};
pub use runner::Assertion;

use std::path::Path;

/// Compile every `.st` file below `dir` and collect per-file diagnostics.
pub fn check_dir(dir: &str) -> CheckReport {
    check::run(Path::new(dir))
}

/// Run every `*.test.yaml` below `dir` against the runtime VM in virtual
/// time; when `junit_path` is given, also write a JUnit XML report there.
pub fn test_dir(dir: &str, junit_path: Option<&str>) -> TestReport {
    harness::run(Path::new(dir), junit_path.map(Path::new))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAM: &str = r#"
PROGRAM Tally
VAR
    x : INT := 1;
END_VAR
    x := x + 1;
END_PROGRAM
"#;

    const SCENARIO: &str = r#"
program: Tally.st
scenarios:
  - name: set overrides seed then scans
    given:
      x: 0
    steps:
      - set: {x: 40}
        advance: 2scans
        expect:
          x: 42
"#;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mt-plc-cli-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn check_dir_reports_per_file_diagnostics() {
        let dir = temp_dir("lib-check");
        std::fs::write(dir.join("ok.st"), PROGRAM).unwrap();
        std::fs::write(dir.join("bad.st"), "PROGRAM B\nVAR\n    x : NOTATYPE;\nEND_VAR\n").unwrap();

        let report = check_dir(dir.to_str().unwrap());
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.files, 2);
        assert!(report.errors >= 1, "{report:?}");
        assert!(!report.passed);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.file == "bad.st" && d.severity == "error" && d.line > 0));

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"file\":\"bad.st\""));
        assert!(json.contains("\"severity\""));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_dir_runs_set_scenarios_and_writes_junit() {
        let dir = temp_dir("lib-test");
        std::fs::write(dir.join("Tally.st"), PROGRAM).unwrap();
        std::fs::write(dir.join("Tally.test.yaml"), SCENARIO).unwrap();

        let report = test_dir(dir.to_str().unwrap(), None);
        assert!(report.error.is_none(), "{:?}", report.error);
        assert_eq!(report.files, 1);
        assert_eq!(report.scenarios, 1);
        assert_eq!(report.failures, 0);
        assert!(report.passed);
        assert!(report.suites[0].cases[0].outcome == ScenarioOutcome::Passed);

        let junit = dir.join("reports/junit.xml");
        let report = test_dir(dir.to_str().unwrap(), junit.to_str());
        assert!(report.error.is_none(), "{:?}", report.error);
        assert!(junit.is_file(), "junit report written");

        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"passed\":true"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_dir_folds_missing_directory_into_error() {
        let report = test_dir("/nonexistent/mt-plc-cli-definitely-absent", None);
        assert!(report.error.is_some());
        assert!(!report.passed);
        assert_eq!(report.files, 0);
    }
}
