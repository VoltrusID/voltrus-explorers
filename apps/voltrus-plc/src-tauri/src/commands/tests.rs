use mt_plc_cli::{ScenarioOutcome, SuiteReport, TestReport};

// ── DTOs (serde wire format consumed by the Tests panel) ──

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct TestReportDto {
    pub passed: bool,
    pub duration_ms: u64,
    pub file_count: usize,
    pub scenario_count: usize,
    pub failure_count: usize,
    pub error: Option<String>,
    pub suites: Vec<TestSuiteDto>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct TestSuiteDto {
    pub path: String,
    pub scenarios: Vec<ScenarioDto>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct ScenarioDto {
    pub name: String,
    pub outcome: String,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub assertions: Vec<AssertionDto>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct AssertionDto {
    pub name: String,
    pub passed: bool,
    pub expected: String,
    pub actual: String,
}

fn map_report(report: TestReport) -> TestReportDto {
    TestReportDto {
        passed: report.passed,
        duration_ms: report.duration_ms,
        file_count: report.files,
        scenario_count: report.scenarios,
        failure_count: report.failures,
        error: report.error,
        suites: report
            .suites
            .into_iter()
            .map(|s| TestSuiteDto {
                path: s.name,
                scenarios: s.cases.into_iter().map(map_scenario).collect(),
            })
            .collect(),
    }
}

fn map_scenario(scenario: mt_plc_cli::ScenarioReport) -> ScenarioDto {
    ScenarioDto {
        name: scenario.name,
        outcome: match scenario.outcome {
            ScenarioOutcome::Passed => "passed".to_string(),
            ScenarioOutcome::Failed => "failed".to_string(),
            ScenarioOutcome::Error => "error".to_string(),
        },
        duration_ms: scenario.duration_ms,
        error: scenario.error,
        assertions: scenario
            .assertions
            .into_iter()
            .map(|a| AssertionDto {
                name: a.name,
                passed: a.passed,
                expected: a.expected,
                actual: a.actual,
            })
            .collect(),
    }
}

#[tauri::command]
pub async fn run_project_tests(
    project_dir: String,
    junit: Option<String>,
) -> Result<TestReportDto, String> {
    if !std::path::Path::new(&project_dir).is_dir() {
        return Err(format!("project directory not found: {}", project_dir));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let report = mt_plc_cli::test_dir(&project_dir, junit.as_deref());
        Ok(map_report(report))
    })
    .await
    .map_err(|e| e.to_string())?
}

// Mirrors the schema documented in crates/mt-plc-cli/README.md (including the
// mid-step `set:` override), embedded verbatim into the AI prompt so the model
// emits YAML the CLI actually parses.
const TEST_YAML_SCHEMA: &str = r#"```yaml
# TankLevel.test.yaml
program: TankLevel.st   # ST source, resolved relative to this file
scan_ms: 10             # optional virtual scan-cycle period (default 10)

scenarios:
  - name: low level opens inlet          # required, shown in reports
    given:                               # optional seeds, coerced to the
      level: 40.0                        # declared variable types
    steps:                               # ordered list of advances
      - advance: 1scans                  # run 1 scan cycle
        set:                             # optional writes applied before
          reset: true                    # this step's scans
        expect:                          # optional, checked after the scans
          inlet_valve: true              # exact bool
          level: {approx: 41.0, tol: 0.001}  # numeric window
    expect:                              # optional final check, after the
      high_level_alarm: false            # last step
```"#;

/// Builds the single prompt handed to the streaming chat commands: rules and
/// schema first (system-style), then the program and its declared variables.
#[tauri::command]
pub fn generate_test_prompt(program_source: String, variables: Vec<String>) -> String {
    let vars = if variables.is_empty() {
        "(none declared)".to_string()
    } else {
        variables
            .iter()
            .map(|v| format!("- {}", v))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"You generate virtual-time scenario tests for IEC 61131-3 Structured Text programs.

Rules:
- Reply with ONE fenced ```yaml block and nothing else.
- The file tests one program: set `program:` to the source file name shown below.
- Cover normal paths, boundary/edge cases, and timer/counter behavior.
- `given:` seeds variables before the first scan; values are coerced to the declared types.
- Each step needs `advance:` (`25scans`, `500ms` or `2s` — virtual time, nothing sleeps).
- `set:` optionally writes variables before the step's scans, coerced like `given:`.
- `expect:` matchers: `true`/`false` for BOOL, `42` for any numeric, `41.5` for a numeric within 1e-4, `{{approx: 90.0, tol: 0.5}}` for a numeric window, `"RUN"` for STRING/enum equality.
- Timers run virtually: a TON reaches Q = TRUE after ~(PT / scan_ms) + 1 scans; keep expectations away from the PT boundary.
- Only reference declared variables. Every scenario must be deterministic.

Scenario YAML schema:
{schema}

Program variables:
{vars}

Program source:
```st
{source}
```"#,
        schema = TEST_YAML_SCHEMA,
        vars = vars,
        source = program_source,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mt_plc_cli::{Assertion, ScenarioReport};

    fn sample_report() -> TestReport {
        TestReport {
            files: 1,
            scenarios: 2,
            failures: 1,
            passed: false,
            duration_ms: 100,
            error: None,
            suites: vec![SuiteReport {
                name: "TankLevel.test.yaml".to_string(),
                cases: vec![
                    ScenarioReport {
                        name: "low level opens inlet".to_string(),
                        outcome: ScenarioOutcome::Passed,
                        duration_ms: 12,
                        failures: vec![],
                        error: None,
                        assertions: vec![Assertion {
                            name: "inlet_valve".to_string(),
                            passed: true,
                            expected: "TRUE".to_string(),
                            actual: "TRUE".to_string(),
                        }],
                    },
                    ScenarioReport {
                        name: "high level alarms".to_string(),
                        outcome: ScenarioOutcome::Failed,
                        duration_ms: 34,
                        failures: vec!["high_level_alarm: expected TRUE, got FALSE (BOOL)".to_string()],
                        error: None,
                        assertions: vec![
                            Assertion {
                                name: "high_level_alarm".to_string(),
                                passed: false,
                                expected: "TRUE".to_string(),
                                actual: "FALSE (BOOL)".to_string(),
                            },
                            Assertion {
                                name: "inlet_valve".to_string(),
                                passed: true,
                                expected: "FALSE".to_string(),
                                actual: "FALSE".to_string(),
                            },
                        ],
                    },
                ],
            }],
        }
    }

    #[test]
    fn maps_report_to_dto() {
        let dto = map_report(sample_report());
        assert!(!dto.passed);
        assert_eq!(dto.duration_ms, 100);
        assert_eq!(dto.file_count, 1);
        assert_eq!(dto.scenario_count, 2);
        assert_eq!(dto.failure_count, 1);
        assert_eq!(dto.error, None);
        assert_eq!(dto.suites.len(), 1);
        assert_eq!(dto.suites[0].path, "TankLevel.test.yaml");
        assert_eq!(dto.suites[0].scenarios.len(), 2);
        assert_eq!(dto.suites[0].scenarios[0].outcome, "passed");
        assert_eq!(dto.suites[0].scenarios[0].duration_ms, 12);
        assert_eq!(dto.suites[0].scenarios[1].outcome, "failed");
        assert_eq!(dto.suites[0].scenarios[1].assertions.len(), 2);
        assert!(!dto.suites[0].scenarios[1].assertions[0].passed);
        assert_eq!(dto.suites[0].scenarios[1].assertions[0].expected, "TRUE");
        assert_eq!(dto.suites[0].scenarios[1].assertions[0].actual, "FALSE (BOOL)");
    }

    #[test]
    fn dto_serializes_snake_case_for_frontend() {
        let json = serde_json::to_value(map_report(sample_report())).unwrap();
        assert_eq!(json["duration_ms"], 100);
        assert_eq!(json["failure_count"], 1);
        assert_eq!(json["suites"][0]["scenarios"][0]["assertions"][0]["expected"], "TRUE");
        assert!(json.get("durationMs").is_none());
    }

    #[test]
    fn maps_load_error_and_fatal_error() {
        let mut report = sample_report();
        report.error = Some("dir unreadable".to_string());
        report.suites[0].cases[0].outcome = ScenarioOutcome::Error;
        report.suites[0].cases[0].error = Some("TankLevel.st: compile error".to_string());
        let dto = map_report(report);
        assert_eq!(dto.error.as_deref(), Some("dir unreadable"));
        assert_eq!(dto.suites[0].scenarios[0].outcome, "error");
        assert_eq!(dto.suites[0].scenarios[0].error.as_deref(), Some("TankLevel.st: compile error"));
    }

    #[test]
    fn prompt_embeds_schema_program_and_variables() {
        let prompt = generate_test_prompt(
            "PROGRAM TankLevel\nVAR\n    level : REAL;\nEND_VAR\nEND_PROGRAM".to_string(),
            vec!["level".to_string(), "inlet_valve".to_string()],
        );
        assert!(prompt.contains("scenarios:"));
        assert!(prompt.contains("advance:"));
        assert!(prompt.contains("set:"));
        assert!(prompt.contains("{approx:"));
        assert!(prompt.contains("PROGRAM TankLevel"));
        assert!(prompt.contains("- level"));
        assert!(prompt.contains("- inlet_valve"));
    }

    #[test]
    fn prompt_handles_empty_variables() {
        let prompt = generate_test_prompt("PROGRAM Main END_PROGRAM".to_string(), vec![]);
        assert!(prompt.contains("(none declared)"));
    }
}
