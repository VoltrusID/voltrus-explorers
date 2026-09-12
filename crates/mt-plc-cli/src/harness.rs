use std::path::Path;

use crate::check::{collect_files, rel_path};
use crate::junit::{Case, Suite};
use crate::runner::run_scenario;
use crate::scenario::{ScenarioFile, DEFAULT_SCAN_MS};

pub struct TestReport {
    pub files: usize,
    pub scenarios: usize,
    pub failures: usize,
}

/// Run every `*.test.yaml` below `dir`. Each file targets one program
/// (`program:` next to the YAML); each scenario gets a freshly initialized
/// VM, so scenarios are independent. Exit-relevant failure count is the
/// number of failed scenarios plus files that failed to load/compile.
pub fn run(dir: &Path, junit_path: Option<&Path>) -> anyhow::Result<TestReport> {
    let yaml_files = collect_files(dir, |p| {
        p.file_name()
            .is_some_and(|n| n.to_string_lossy().to_lowercase().ends_with(".test.yaml"))
    })?;

    let mut suites = Vec::new();
    let mut scenarios = 0usize;
    let mut failures = 0usize;

    for yaml_path in &yaml_files {
        let rel = rel_path(dir, yaml_path);
        println!("{rel}");
        let suite = run_file(yaml_path, &rel);
        for case in &suite.cases {
            scenarios += 1;
            let failed = case.failed();
            failures += usize::from(failed);
            println!("  {}  {}", if failed { "FAIL" } else { "ok  " }, case.name);
            for line in case.failures.iter().chain(case.error.iter()) {
                println!("       {line}");
            }
        }
        suites.push(suite);
    }

    if let Some(path) = junit_path {
        crate::junit::write_report(path, &suites)?;
        println!("junit report written to {}", path.display());
    }

    Ok(TestReport {
        files: yaml_files.len(),
        scenarios,
        failures,
    })
}

fn run_file(yaml_path: &Path, rel: &str) -> Suite {
    let error_case = |message: String| Suite {
        name: rel.to_string(),
        cases: vec![Case {
            name: "load".into(),
            failures: Vec::new(),
            error: Some(message),
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

    let compiled = mt_stc::compile(&source);
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
            let outcome = run_scenario(scenario, &module, scan_ms);
            Case {
                name: outcome.name,
                failures: outcome.failures,
                error: None,
            }
        })
        .collect();

    Suite {
        name: rel.to_string(),
        cases,
    }
}

