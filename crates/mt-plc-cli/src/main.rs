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
    }
}

fn exit_code(code: usize) -> std::process::ExitCode {
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(u8::MAX))
}
