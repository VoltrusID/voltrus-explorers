use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_yaml::Mapping;
use std::time::{Duration, Instant};

use mt_plc_runtime::Vm;
use mt_plc_types::{CompiledModule, Instruction, PlcValue};

use crate::scenario::{Matcher, Scenario, coerce_given, parse_advance, parse_expect};

/// Deterministic scan-cycle driver around the runtime VM.
///
/// Every scan advances virtual time by `scan_ms`. The runtime's
/// `TimerInstance` reads the wall clock (`Instant::now`) on each execution,
/// so virtual time is implemented by rewinding every timer's start instant
/// by `scan_ms` just before each scan: a timer sees the accumulated virtual
/// elapsed time while scan-order effects (edges, counters) stay exact.
pub struct TestRunner {
    vm: Vm,
    instructions: Vec<Instruction>,
    scan_ms: u64,
}

impl TestRunner {
    pub fn new(module: CompiledModule, scan_ms: u64) -> anyhow::Result<Self> {
        if scan_ms == 0 {
            anyhow::bail!("scan_ms must be greater than zero");
        }
        let instructions = module.instructions.clone();
        Ok(Self {
            vm: Vm::from_instructions(module),
            instructions,
            scan_ms,
        })
    }

    pub fn scan_ms(&self) -> u64 {
        self.scan_ms
    }
    pub fn variable_names(&self) -> Vec<String> {
        self.vm.memory().variable_names()
    }

    pub fn value_of(&self, name: &str) -> Option<PlcValue> {
        self.vm.memory().get_variable(name).cloned()
    }

    /// Seed one `given:` variable, coercing the YAML scalar to the slot's
    /// declared type.
    pub fn give(&mut self, name: &str, raw: &serde_yaml::Value) -> anyhow::Result<()> {
        let slot = self
            .vm
            .memory()
            .get_variable(name)
            .cloned()
            .with_context(|| {
                format!(
                    "unknown variable `{}` (declared: {})",
                    name,
                    self.variable_names().join(", ")
                )
            })?;
        let value = coerce_given(name, &slot, raw)?;
        self.vm
            .memory_mut()
            .set_variable(name, value)
            .map_err(anyhow::Error::msg)
    }

    /// Run `scans` scan cycles, advancing virtual time by `scan_ms` each.
    pub fn advance(&mut self, scans: usize) -> Result<(), String> {
        for _ in 0..scans {
            self.advance_one()?;
        }
        Ok(())
    }

    fn advance_one(&mut self) -> Result<(), String> {
        let mut snapshot = self.vm.snapshot_fb_state();
        for timer in &mut snapshot.timers {
            if let Some(started) = timer.start_time {
                // Guarded: rewinding must never panic on clock underflow.
                timer.start_time = started
                    .checked_sub(Duration::from_millis(self.scan_ms))
                    .or(Some(started));
            }
        }
        self.vm.restore_fb_state(&snapshot);
        self.vm.execute(&self.instructions)
    }
}

/// Per-assertion detail recorded for report consumers (JSON panel, CI).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Assertion {
    pub name: String,
    pub passed: bool,
    /// Rendered expectation, e.g. `3` or `90 ±0.001`.
    pub expected: String,
    /// Rendered actual value, e.g. `2 (DINT)`.
    pub actual: String,
}

/// In-memory result of one scenario run; the harness converts it into a
/// serializable `ScenarioReport`.
#[derive(Clone, Debug)]
pub struct ScenarioResult {
    pub name: String,
    pub failures: Vec<String>,
    pub assertions: Vec<Assertion>,
    pub duration_ms: u64,
}

impl ScenarioResult {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Execute one scenario: seed `given`, apply each step's `set:` writes
/// (before its advance), run each step's scans, check the step's and
/// finally the scenario's expectations. All failures are collected (never
/// early-panic); a runtime abort stops the scenario.
pub fn run_scenario(scenario: &Scenario, module: &CompiledModule, scan_ms: u64) -> ScenarioResult {
    let started = Instant::now();
    let mut failures = Vec::new();
    let mut assertions = Vec::new();
    let mut runner = match TestRunner::new(module.clone(), scan_ms) {
        Ok(runner) => runner,
        Err(err) => {
            return ScenarioResult {
                name: scenario.name.clone(),
                failures: vec![format!("setup: {err:#}")],
                assertions: Vec::new(),
                duration_ms: 0,
            };
        }
    };

    for (key, raw) in &scenario.given {
        let name = match key.as_str() {
            Some(name) => name,
            None => {
                failures.push("setup: `given` keys must be variable names".into());
                break;
            }
        };
        if let Err(err) = runner.give(name, raw) {
            failures.push(format!("setup: {err:#}"));
        }
    }

    if failures.is_empty() {
        for (index, step) in scenario.steps.iter().enumerate() {
            if step.advance.is_none() && step.set.as_ref().is_none_or(Mapping::is_empty) {
                failures.push(format!("step {index}: needs `set` or `advance`"));
                break;
            }
            if let Some(set) = &step.set {
                for (key, raw) in set {
                    let name = match key.as_str() {
                        Some(name) => name,
                        None => {
                            failures
                                .push(format!("step {index}: `set` keys must be variable names"));
                            break;
                        }
                    };
                    if let Err(err) = runner.give(name, raw) {
                        failures.push(format!("step {index}: {err:#}"));
                    }
                }
            }
            if let Some(advance) = &step.advance {
                let scans = match parse_advance(advance, scan_ms) {
                    Ok(scans) => scans,
                    Err(err) => {
                        failures.push(format!("invalid advance `{advance}`: {err:#}"));
                        break;
                    }
                };
                if let Err(err) = runner.advance(scans) {
                    failures.push(format!("runtime error during `advance {advance}`: {err}"));
                    break;
                }
            }
            if let Some(expect) = &step.expect {
                match parse_expect(expect) {
                    Ok(matchers) => {
                        check_matchers(&matchers, &runner, &mut failures, &mut assertions)
                    }
                    Err(err) => {
                        failures.push(format!("invalid expect: {err:#}"));
                        break;
                    }
                }
            }
        }

        if failures.is_empty() {
            if let Some(expect) = &scenario.expect {
                match parse_expect(expect) {
                    Ok(matchers) => {
                        check_matchers(&matchers, &runner, &mut failures, &mut assertions)
                    }
                    Err(err) => failures.push(format!("invalid expect: {err:#}")),
                }
            }
        }
    }

    ScenarioResult {
        name: scenario.name.clone(),
        failures,
        assertions,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }
}

fn check_matchers(
    expect: &[(String, Matcher)],
    runner: &TestRunner,
    failures: &mut Vec<String>,
    assertions: &mut Vec<Assertion>,
) {
    for (name, matcher) in expect {
        match runner.value_of(name) {
            Some(actual) => {
                let passed = matcher.matches(&actual);
                if !passed {
                    failures.push(format!(
                        "{}: expected {}, got {} ({})",
                        name,
                        matcher.describe(),
                        actual,
                        actual.type_name()
                    ));
                }
                assertions.push(Assertion {
                    name: name.clone(),
                    passed,
                    expected: matcher.describe(),
                    actual: format!("{actual} ({})", actual.type_name()),
                });
            }
            None => {
                failures.push(format!(
                    "{}: no such variable (declared: {})",
                    name,
                    runner.variable_names().join(", ")
                ));
                assertions.push(Assertion {
                    name: name.clone(),
                    passed: false,
                    expected: matcher.describe(),
                    actual: "no such variable".into(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::ScenarioFile;

    const TON_PROGRAM: &str = r#"
PROGRAM TimedLamp
VAR
    Start : BOOL := FALSE;
    Done : BOOL := FALSE;
    LampDelay : TON;
END_VAR
    LampDelay(IN := Start, PT := T#2s);
    Done := LampDelay.Q;
END_PROGRAM
"#;

    const CTU_PROGRAM: &str = r#"
PROGRAM CounterTest
VAR
    Pulse : BOOL := FALSE;
    Count : BOOL := FALSE;
    Tally : CTU;
END_VAR
    Tally(CU := Pulse, R := FALSE, PV := 1);
    Count := Tally.Q;
END_PROGRAM
"#;

    fn compile(source: &str) -> CompiledModule {
        let out = mt_stc::compile(source);
        assert!(out.success, "compile failed: {:?}", out.diagnostics);
        out.module.expect("module present on success")
    }

    fn give(runner: &mut TestRunner, name: &str, raw: &str) {
        let value: serde_yaml::Value = serde_yaml::from_str(raw).unwrap();
        runner.give(name, &value).unwrap();
    }

    #[test]
    fn ton_reaches_done_in_virtual_time() {
        let module = compile(TON_PROGRAM);
        let mut runner = TestRunner::new(module, 10).unwrap();
        assert_eq!(runner.scan_ms(), 10);

        give(&mut runner, "Start", "true");
        runner.advance(190).unwrap(); // 1.9s of virtual time, PT is T#2s

        assert_eq!(runner.value_of("Done").unwrap(), PlcValue::Bool(false));
        // Scan 1 starts the timer at ~0ms; scan k sees ~ (k-1) * scan_ms.
        let et = runner.value_of("LampDelay.ET").unwrap().to_int();
        assert!(
            (1800..=1990).contains(&et),
            "ET {et} outside expected window"
        );

        runner.advance(30).unwrap(); // crosses T#2s
        assert_eq!(runner.value_of("Done").unwrap(), PlcValue::Bool(true));
    }

    #[test]
    fn ton_resets_when_input_drops() {
        let module = compile(TON_PROGRAM);
        let mut runner = TestRunner::new(module, 10).unwrap();

        give(&mut runner, "Start", "true");
        runner.advance(250).unwrap(); // 2.5s, well past PT
        assert_eq!(runner.value_of("Done").unwrap(), PlcValue::Bool(true));

        give(&mut runner, "Start", "false");
        runner.advance(1).unwrap();
        assert_eq!(runner.value_of("Done").unwrap(), PlcValue::Bool(false));
        assert_eq!(runner.value_of("LampDelay.ET").unwrap(), PlcValue::Time(0));
    }

    #[test]
    fn counter_counts_one_edge_per_scan() {
        let module = compile(CTU_PROGRAM);
        let mut runner = TestRunner::new(module, 10).unwrap();

        give(&mut runner, "Pulse", "true");
        runner.advance(1).unwrap();
        assert_eq!(runner.value_of("Count").unwrap(), PlcValue::Bool(true));
        assert_eq!(runner.value_of("Tally.CV").unwrap(), PlcValue::Dint(1));

        // Holding CU high adds no further edges.
        runner.advance(3).unwrap();
        assert_eq!(runner.value_of("Tally.CV").unwrap(), PlcValue::Dint(1));
    }

    #[test]
    fn zero_scan_period_is_rejected() {
        let module = compile(TON_PROGRAM);
        assert!(TestRunner::new(module, 0).is_err());
    }

    #[test]
    fn unknown_given_variable_lists_declared_names() {
        let module = compile(TON_PROGRAM);
        let mut runner = TestRunner::new(module, 10).unwrap();
        let err = runner
            .give("Stat", &serde_yaml::Value::Bool(true))
            .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("unknown variable `Stat`"), "{message}");
        assert!(message.contains("Start"), "{message}");
    }

    #[test]
    fn scenario_end_to_end_with_approx_and_final_expect() {
        let yaml = r#"
program: inline
scenarios:
  - name: lamp lights after 2s
    given:
      Start: true
    steps:
      - advance: 190scans
        expect:
          Done: false
          LampDelay.ET: {approx: 1890, tol: 90}
      - advance: 30scans
    expect:
      Done: true
"#;
        let file: crate::scenario::ScenarioFile = serde_yaml::from_str(yaml).unwrap();
        let module = compile(TON_PROGRAM);
        let outcome = run_scenario(&file.scenarios[0], &module, 10);
        assert!(
            outcome.passed(),
            "unexpected failures: {:?}",
            outcome.failures
        );
    }

    #[test]
    fn failing_expect_reports_actual_value() {
        let yaml = r#"
program: inline
scenarios:
  - name: premature expectation
    given:
      Start: true
    steps:
      - advance: 1scans
        expect:
          Done: true
"#;
        let file: crate::scenario::ScenarioFile = serde_yaml::from_str(yaml).unwrap();
        let module = compile(TON_PROGRAM);
        let outcome = run_scenario(&file.scenarios[0], &module, 10);
        assert!(!outcome.passed());
        let first = &outcome.failures[0];
        assert!(first.contains("Done"), "{first}");
        assert!(first.contains("expected TRUE"), "{first}");
        assert!(first.contains("FALSE"), "{first}");
    }

    #[test]
    fn unknown_expect_variable_is_reported() {
        let yaml = r#"
program: inline
scenarios:
  - name: typo watch
    expect:
      Dunno: true
"#;
        let file: crate::scenario::ScenarioFile = serde_yaml::from_str(yaml).unwrap();
        let module = compile(TON_PROGRAM);
        let outcome = run_scenario(&file.scenarios[0], &module, 10);
        assert!(
            outcome.failures[0].contains("no such variable"),
            "{:?}",
            outcome.failures
        );
    }

    #[test]
    fn set_only_step_writes_without_scanning() {
        let module = compile(CTU_PROGRAM);
        let yaml = r#"
program: inline
scenarios:
  - name: write without scanning
    steps:
      - set: {Pulse: true}
        expect:
          Pulse: true
          Tally.CV: 0
"#;
        let file: ScenarioFile = serde_yaml::from_str(yaml).unwrap();
        let outcome = run_scenario(&file.scenarios[0], &module, 10);
        assert!(
            outcome.passed(),
            "unexpected failures: {:?}",
            outcome.failures
        );
        // The write landed and no scan ran, so the counter saw no edge.
        assert_eq!(outcome.assertions.len(), 2);
        assert!(outcome.assertions.iter().all(|a| a.passed));
    }

    #[test]
    fn set_then_advance_pulses_the_counter() {
        let module = compile(CTU_PROGRAM);
        let yaml = r#"
program: inline
scenarios:
  - name: set then advance
    steps:
      - set: {Pulse: true}
        advance: 1scans
        expect:
          Tally.CV: 1
          Count: true
      - set: {Pulse: false}
        advance: 1scans
        expect:
          Tally.CV: 1
"#;
        let file: ScenarioFile = serde_yaml::from_str(yaml).unwrap();
        let outcome = run_scenario(&file.scenarios[0], &module, 10);
        assert!(
            outcome.passed(),
            "unexpected failures: {:?}",
            outcome.failures
        );
    }

    #[test]
    fn ctu_counts_three_set_pulses() {
        let module = compile(CTU_PROGRAM);
        let yaml = r#"
program: inline
scenarios:
  - name: three pulses reach CV 3
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
      Count: true
"#;
        let file: ScenarioFile = serde_yaml::from_str(yaml).unwrap();
        let outcome = run_scenario(&file.scenarios[0], &module, 10);
        assert!(
            outcome.passed(),
            "unexpected failures: {:?}",
            outcome.failures
        );
    }

    #[test]
    fn unknown_set_variable_lists_declared_names() {
        let module = compile(TON_PROGRAM);
        let yaml = r#"
program: inline
scenarios:
  - name: typo in set
    steps:
      - set: {Stat: true}
"#;
        let file: ScenarioFile = serde_yaml::from_str(yaml).unwrap();
        let outcome = run_scenario(&file.scenarios[0], &module, 10);
        assert!(!outcome.passed());
        let message = &outcome.failures[0];
        assert!(message.contains("step 0"), "{message}");
        assert!(message.contains("unknown variable `Stat`"), "{message}");
        assert!(message.contains("Start"), "{message}");
    }

    #[test]
    fn step_without_set_or_advance_names_the_index() {
        let module = compile(TON_PROGRAM);
        let yaml = r#"
program: inline
scenarios:
  - name: empty step
    steps:
      - advance: 1scans
      - expect: {Done: false}
"#;
        let file: ScenarioFile = serde_yaml::from_str(yaml).unwrap();
        let outcome = run_scenario(&file.scenarios[0], &module, 10);
        assert!(!outcome.passed());
        assert!(
            outcome.failures[0].contains("step 1"),
            "{:?}",
            outcome.failures
        );
    }
}
