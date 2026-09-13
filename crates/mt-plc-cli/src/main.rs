use clap::{Parser, Subcommand};
use mt_plc_cli::{check_dir, test_dir};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "voltrus-plc",
    about = "Headless Voltrus PLC toolchain: source checks and virtual-time scenario tests"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compile every .st file under a directory and report diagnostics
    Check { dir: PathBuf },
    /// Run *.test.yaml scenarios against the runtime VM in virtual time
    Test {
        dir: PathBuf,
        /// Write a JUnit XML report to this path
        #[arg(long)]
        junit: Option<PathBuf>,
        /// Print the full structured report as JSON instead of human progress
        #[arg(long)]
        json: bool,
    },
    /// Import a Studio 5000 L5X or PLCopen TC6 XML export into .st files
    Import {
        /// L5X (Rockwell Studio 5000) export file
        #[arg(long, required_unless_present = "plcopen", conflicts_with = "plcopen")]
        l5x: Option<PathBuf>,
        /// PLCopen TC6 XML (IEC 61131-10) project file
        #[arg(long, required_unless_present = "l5x")]
        plcopen: Option<PathBuf>,
        /// Output directory for the generated .st files
        #[arg(long)]
        out: PathBuf,
    },
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Check { dir } => {
            let report = check_dir(&dir.to_string_lossy());
            if let Some(err) = &report.error {
                eprintln!("error: {err}");
                return exit_code(2);
            }
            for d in &report.diagnostics {
                println!("{}:{}:{}: {}: {}", d.file, d.line, d.col, d.severity, d.message);
            }
            println!(
                "checked {} files, {} errors, {} warnings",
                report.files, report.errors, report.warnings
            );
            exit_code(report.errors)
        }
        Command::Test { dir, junit, json } => {
            let junit_path = junit.as_deref().map(|p| p.to_string_lossy().into_owned());
            let report = test_dir(&dir.to_string_lossy(), junit_path.as_deref());
            if let Some(err) = &report.error {
                eprintln!("error: {err}");
                return exit_code(2);
            }
            if json {
                let body =
                    serde_json::to_string_pretty(&report).expect("report is JSON-serializable");
                println!("{body}");
                return exit_code(report.failures);
            }
            for suite in &report.suites {
                println!("{}", suite.name);
                for case in &suite.cases {
                    println!(
                        "  {}  {}",
                        if case.outcome == mt_plc_cli::ScenarioOutcome::Passed {
                            "ok  "
                        } else {
                            "FAIL"
                        },
                        case.name
                    );
                    for line in case.failures.iter().chain(case.error.iter()) {
                        println!("       {line}");
                    }
                }
            }
            if let Some(path) = junit_path {
                println!("junit report written to {path}");
            }
            println!(
                "tested {} files, {} scenarios, {} failures",
                report.files, report.scenarios, report.failures
            );
            exit_code(report.failures)
        }
        Command::Import { l5x, plcopen, out } => {
            let format = if l5x.is_some() {
                ImportFormat::L5x
            } else {
                ImportFormat::PlcOpen
            };
            let source = l5x
                .as_deref()
                .or(plcopen.as_deref())
                .expect("clap enforces exactly one source");
            exit_code(run_import(source, format, &out))
        }
    }
}

enum ImportFormat {
    L5x,
    PlcOpen,
}

/// Run one import: parse, write generated files, print the report. Exit
/// code 2 on unreadable input or malformed XML, 1 when nothing usable was
/// imported (or any other error was reported), 0 otherwise.
fn run_import(source: &std::path::Path, format: ImportFormat, out: &std::path::Path) -> usize {
    let xml = match std::fs::read_to_string(source) {
        Ok(xml) => xml,
        Err(err) => {
            eprintln!("error: cannot read {}: {err}", source.display());
            return 2;
        }
    };
    let report = match format {
        ImportFormat::L5x => mt_import::import_l5x(&xml),
        ImportFormat::PlcOpen => mt_import::import_plcopen(&xml),
    };
    if let Err(err) = std::fs::create_dir_all(out) {
        eprintln!(
            "error: cannot create output directory {}: {err}",
            out.display()
        );
        return 2;
    }
    for file in &report.imported {
        let path = out.join(&file.name);
        if let Err(err) = std::fs::write(&path, &file.st_source) {
            eprintln!("error: cannot write {}: {err}", path.display());
            return 2;
        }
    }
    println!(
        "imported {} file(s) into {}",
        report.imported.len(),
        out.display()
    );
    for file in &report.imported {
        println!("  {}", file.name);
    }
    if !report.skipped.is_empty() {
        println!("skipped {} item(s):", report.skipped.len());
        for s in &report.skipped {
            println!("  {} — {}", s.name, s.reason);
        }
    }
    if !report.errors.is_empty() {
        println!("errors {}:", report.errors.len());
        for e in &report.errors {
            println!("  {}: {}", e.context, e.message);
        }
    }
    let malformed = report.errors.iter().any(|e| e.context == "parse");
    if malformed {
        2
    } else if !report.is_success() {
        1
    } else {
        0
    }
}

fn exit_code(code: usize) -> std::process::ExitCode {
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(u8::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../mt-import/tests/fixtures")
            .join(name)
    }

    fn temp_out(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "voltrus-plc-import-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn import_l5x_fixture_writes_files_and_exits_zero() {
        let out = temp_out("l5x-clean");
        let code = run_import(&fixture("l5x_clean.l5x"), ImportFormat::L5x, &out);
        assert_eq!(code, 0);
        let written: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(written.contains(&"Line1_Tags.st".to_string()), "{written:?}");
        assert!(written.contains(&"Conveyor_Main.st".to_string()), "{written:?}");
        std::fs::remove_dir_all(&out).ok();
    }

    #[test]
    fn import_ld_fixture_exits_zero_with_skip_reported() {
        let out = temp_out("l5x-ld");
        let code = run_import(&fixture("l5x_ld_routine.l5x"), ImportFormat::L5x, &out);
        assert_eq!(code, 0);
        std::fs::remove_dir_all(&out).ok();
    }

    #[test]
    fn import_plcopen_fixture_writes_files_and_exits_zero() {
        let out = temp_out("plcopen-clean");
        let code = run_import(&fixture("plcopen_clean.xml"), ImportFormat::PlcOpen, &out);
        assert_eq!(code, 0);
        assert!(out.join("Main.st").exists());
        std::fs::remove_dir_all(&out).ok();
    }

    #[test]
    fn import_malformed_fixture_exits_two() {
        let out = temp_out("malformed");
        let code = run_import(&fixture("l5x_malformed.l5x"), ImportFormat::L5x, &out);
        assert_eq!(code, 2);
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
        std::fs::remove_dir_all(&out).ok();
    }

    #[test]
    fn import_missing_file_exits_two() {
        let out = temp_out("missing");
        let code = run_import(&PathBuf::from("/nonexistent/export.l5x"), ImportFormat::L5x, &out);
        assert_eq!(code, 2);
        std::fs::remove_dir_all(&out).ok();
    }
}
