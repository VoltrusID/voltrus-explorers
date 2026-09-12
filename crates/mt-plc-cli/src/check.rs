use anyhow::Context;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

pub struct CheckReport {
    pub files: usize,
    pub errors: usize,
    pub warnings: usize,
}

/// Compile every `.st` file below `dir` headlessly and print
/// `file:line:col: severity: message` diagnostics in deterministic order.
pub fn run(dir: &Path) -> anyhow::Result<CheckReport> {
    let sources = collect_files(dir, |p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("st")))?;
    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut stdout = std::io::stdout().lock();

    for path in &sources {
        let rel = rel_path(dir, path);
        let source = fs::read_to_string(path).with_context(|| format!("failed to read {rel}"))?;
        let result = mt_stc::compile(&source);
        for d in &result.diagnostics {
            writeln!(
                stdout,
                "{}:{}:{}: {}: {}",
                rel, d.line, d.col, d.severity, d.message
            )?;
            if d.severity == "error" {
                errors += 1;
            } else {
                warnings += 1;
            }
        }
    }

    Ok(CheckReport {
        files: sources.len(),
        errors,
        warnings,
    })
}

/// Recursively collect files matching `pred`, sorted for deterministic
/// output. Dot-directories (`.git`, `node_modules`, …) are skipped.
pub(crate) fn collect_files(
    dir: &Path,
    pred: impl Fn(&Path) -> bool,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    visit(dir, &pred, &mut found)?;
    found.sort();
    Ok(found)
}

fn visit(dir: &Path, pred: &impl Fn(&Path) -> bool, found: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    let entries =
        fs::read_dir(dir).with_context(|| format!("failed to read directory {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to list {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            if !entry.file_name().to_string_lossy().starts_with('.') {
                visit(&path, pred, found)?;
            }
        } else if pred(&path) {
            found.push(path);
        }
    }
    Ok(())
}

pub(crate) fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
