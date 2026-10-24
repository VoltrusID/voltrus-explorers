use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// One rendered compiler diagnostic, tied to its source file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub message: String,
    pub severity: String,
}

/// Result of a `check` run; rendering (stdout, JSON) is the caller's job.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CheckReport {
    pub files: usize,
    pub errors: usize,
    pub warnings: usize,
    /// `true` when no error diagnostics and no fatal I/O problem.
    pub passed: bool,
    /// Per-file diagnostics in deterministic (path-sorted) order.
    pub diagnostics: Vec<Diagnostic>,
    /// Fatal problem that aborted the walk (unreadable directory/file).
    #[serde(default)]
    pub error: Option<String>,
}

/// Compile every `.st` file below `dir` headlessly and collect
/// `file:line:col: severity: message` diagnostics in deterministic order.
pub fn run(dir: &Path) -> CheckReport {
    let sources = match collect_files(dir, |p| {
        p.extension().is_some_and(|e| {
            let e = e.to_string_lossy();
            e.eq_ignore_ascii_case("st")
                || crate::diagram::DiagramKind::from_extension(&e).is_some()
        })
    }) {
        Ok(sources) => sources,
        Err(err) => {
            return CheckReport {
                files: 0,
                errors: 0,
                warnings: 0,
                passed: false,
                diagnostics: Vec::new(),
                error: Some(format!("{err:#}")),
            };
        }
    };

    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut diagnostics = Vec::new();

    for path in &sources {
        let rel = rel_path(dir, path);
        let source = match fs::read_to_string(path) {
            Ok(source) => source,
            Err(err) => {
                return CheckReport {
                    files: sources.len(),
                    errors,
                    warnings,
                    passed: false,
                    diagnostics,
                    error: Some(format!("failed to read {rel}: {err}")),
                };
            }
        };
        for (line, col, message, severity) in compile_source(path, &source) {
            if severity == "error" {
                errors += 1;
            } else {
                warnings += 1;
            }
            diagnostics.push(Diagnostic {
                file: rel.clone(),
                line,
                col,
                message,
                severity,
            });
        }
    }

    let passed = errors == 0;
    CheckReport {
        files: sources.len(),
        errors,
        warnings,
        passed,
        diagnostics,
        error: None,
    }
}

/// Compile one source file to `(line, col, message, severity)` diagnostics.
/// `.fbd` / `.ld` / `.sfc` files are transpiled to ST first; transpile
/// diagnostics are attributed to the diagram file itself (line 1, col 1) and
/// the generated ST is then compiled through the normal `mt-stc` path.
/// `.st` files compile directly.
pub(crate) fn compile_source(path: &Path, source: &str) -> Vec<(usize, usize, String, String)> {
    let kind = path
        .extension()
        .and_then(|e| crate::diagram::DiagramKind::from_extension(&e.to_string_lossy()));
    let st_source = match kind {
        None => source.to_string(),
        Some(kind) => match crate::diagram::transpile_source(kind, source) {
            Ok(result) => {
                if result.source.is_empty()
                    || result.diagnostics.iter().any(|d| d.severity == "error")
                {
                    return result
                        .diagnostics
                        .into_iter()
                        .map(|d| (1, 1, d.message, d.severity))
                        .collect();
                }
                result.source
            }
            Err(err) => return vec![(1, 1, err, "error".into())],
        },
    };
    mt_stc::compile(&st_source)
        .diagnostics
        .into_iter()
        .map(|d| (d.line, d.col, d.message, d.severity))
        .collect()
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

fn visit(
    dir: &Path,
    pred: &impl Fn(&Path) -> bool,
    found: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
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
