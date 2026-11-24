//! PLC-10 golden parity tests: transpile the committed diagram fixtures with
//! the Rust ports and compare byte-for-byte against goldens generated from
//! the real TypeScript transpilers (`apps/voltrus-plc/scripts/gen-goldens.test.ts`).
//!
//! Regenerate goldens with:
//!   cd apps/voltrus-plc && pnpm vitest run --config scripts/gen-goldens.vitest.config.ts
//!
//! Exact-byte compare is the contract. If a divergence is ever unavoidable,
//! normalize BOTH sides with `normalize` (collapses whitespace runs) and
//! document the divergence in `src/diagram/mod.rs`.

use std::fs;
use std::path::{Path, PathBuf};

use mt_plc_cli::diagram::{DiagramKind, transpile_source};

fn fixtures_dir(lang: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(lang)
}

/// Whitespace-collapsing fallback comparator (kept in lockstep with the doc
/// contract above — unused while byte parity holds).
#[allow(dead_code)]
fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn golden_case(lang: &str, kind: DiagramKind) {
    let dir = fixtures_dir(lang);
    let mut fixture_paths: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("fixture directory exists")
        .map(|e| e.expect("readable").path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(kind.extension()))
        })
        .collect();
    fixture_paths.sort();

    assert!(
        fixture_paths.len() >= 3,
        "{lang}: expected at least 3 fixtures, found {}",
        fixture_paths.len()
    );

    for fixture in fixture_paths {
        let name = fixture.file_stem().unwrap().to_string_lossy().into_owned();
        let json = fs::read_to_string(&fixture).expect("fixture readable");
        let result = transpile_source(kind, &json)
            .unwrap_or_else(|e| panic!("{lang}/{name}: fixture must parse: {e}"));

        let golden_st = dir.join(format!("{name}.golden.st"));
        let golden_err = dir.join(format!("{name}.expected.txt"));
        let has_error = result.diagnostics.iter().any(|d| d.severity == "error");

        if golden_err.exists() {
            let expected = fs::read_to_string(&golden_err).expect("error golden readable");
            assert!(
                has_error,
                "{lang}/{name}: expected an error diagnostic, got source:\n{}",
                result.source
            );
            let rendered: String = result
                .diagnostics
                .iter()
                .map(|d| format!("{}: {}\n", d.severity, d.message))
                .collect();
            assert_eq!(rendered, expected, "{lang}/{name}: diagnostics differ");
        } else {
            let expected = fs::read_to_string(&golden_st).expect("golden .st readable");
            assert!(
                !has_error,
                "{lang}/{name}: unexpected error diagnostics: {:?}",
                result.diagnostics
            );
            assert_eq!(result.source, expected, "{lang}/{name}: ST parity broken");
        }
    }
}

#[test]
fn fbd_matches_ts_goldens() {
    golden_case("fbd", DiagramKind::Fbd);
}

#[test]
fn ld_matches_ts_goldens() {
    golden_case("ld", DiagramKind::Ld);
}

#[test]
fn sfc_matches_ts_goldens() {
    golden_case("sfc", DiagramKind::Sfc);
}

#[test]
fn cycle_fixture_reports_the_ts_cycle_message() {
    let json = fs::read_to_string(fixtures_dir("fbd").join("cycle-error.fbd")).unwrap();
    let result = transpile_source(DiagramKind::Fbd, &json).unwrap();
    assert_eq!(result.source, "");
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message == "Diagram contains a cycle — FBD execution order must be acyclic")
    );
}
