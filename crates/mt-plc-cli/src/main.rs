mod check;
mod harness;
mod junit;
mod runner;
mod scenario;

use clap::{Parser, Subcommand};
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
    },
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Check { dir } => check::run(&dir).map(|report| {
            println!(
                "checked {} files, {} errors, {} warnings",
                report.files, report.errors, report.warnings
            );
            report.errors
        }),
        Command::Test { dir, junit } => harness::run(&dir, junit.as_deref()).map(|report| {
            println!(
                "tested {} files, {} scenarios, {} failures",
                report.files, report.scenarios, report.failures
            );
            report.failures
        }),
    };
    match result {
        Ok(0) => std::process::ExitCode::SUCCESS,
        Ok(code) => std::process::ExitCode::from(u8::try_from(code).unwrap_or(u8::MAX)),
        Err(err) => {
            eprintln!("error: {err:#}");
            std::process::ExitCode::from(2)
        }
    }
}
