//! End-to-end: import each fixture, write the generated files to a temp
//! project folder, and compile every file through the same call the headless
//! `voltrus-plc check` path uses (`mt_stc::compile` per source file).

use mt_import::{import_l5x, import_plcopen};

fn temp_project_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mt-import-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temp project dir created");
    dir
}

/// Re-compile every written file like `check::compile_source` does for `.st`.
fn check_dir(dir: &std::path::Path) -> Vec<String> {
    let mut errors = Vec::new();
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .expect("temp dir readable")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "st"))
        .collect();
    files.sort();
    for path in files {
        let source = std::fs::read_to_string(&path).expect("generated file readable");
        let output = mt_stc::compile(&source);
        for d in output.diagnostics.iter().filter(|d| d.severity == "error") {
            errors.push(format!(
                "{}:{}:{}: {}",
                path.file_name().unwrap().to_string_lossy(),
                d.line,
                d.col,
                d.message
            ));
        }
    }
    errors
}

fn write_imported(tag: &str, report: &mt_import::ImportReport) -> std::path::PathBuf {
    let dir = temp_project_dir(tag);
    for file in &report.imported {
        std::fs::write(dir.join(&file.name), &file.st_source).expect("generated file written");
    }
    dir
}

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).expect("fixture readable")
}

#[test]
fn l5x_clean_fixture_end_to_end_compiles_with_zero_errors() {
    let report = import_l5x(&fixture("l5x_clean.l5x"));
    assert!(report.is_success(), "{:?}", report.errors);
    let dir = write_imported("l5x-clean", &report);
    let errors = check_dir(&dir);
    assert!(errors.is_empty(), "{errors:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn l5x_ld_fixture_end_to_end_compiles_with_zero_errors() {
    let report = import_l5x(&fixture("l5x_ld_routine.l5x"));
    // The controller tag file (Packaging_Tags.st) plus the ST routine.
    assert_eq!(report.imported.len(), 2, "{:?}", report.imported);
    assert!(report.is_success(), "{:?}", report.errors);
    assert!(report.imported.iter().any(|f| f.name == "Packer_Timing.st"));
    let dir = write_imported("l5x-ld", &report);
    let errors = check_dir(&dir);
    assert!(errors.is_empty(), "{errors:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn plcopen_clean_fixture_end_to_end_compiles_with_zero_errors() {
    let report = import_plcopen(&fixture("plcopen_clean.xml"));
    assert!(report.is_success(), "{:?}", report.errors);
    let dir = write_imported("plcopen-clean", &report);
    let errors = check_dir(&dir);
    assert!(errors.is_empty(), "{errors:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn plcopen_struct_chain_end_to_end_compiles() {
    // Nested struct: Machine contains Recipe; the POU uses both.
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://www.plcopen.org/xml/tc6/0201">
  <types>
    <dataTypes>
      <dataType name="Recipe">
        <baseType>
          <struct>
            <variable name="target"><type><REAL/></type></variable>
          </struct>
        </baseType>
      </dataType>
      <dataType name="Machine">
        <baseType>
          <struct>
            <variable name="recipe"><type><derived name="Recipe"/></type></variable>
            <variable name="label"><type><string length="8"/></type></variable>
          </struct>
        </baseType>
      </dataType>
    </dataTypes>
    <pous>
      <pou name="Cell" pouType="program">
        <interface>
          <localVars>
            <variable name="machine"><type><derived name="Machine"/></type></variable>
          </localVars>
        </interface>
        <body>
          <ST><xhtml xmlns="http://www.w3.org/1999/xhtml">machine.recipe.target := 1.5;</xhtml></ST>
        </body>
      </pou>
    </pous>
  </types>
</project>"#;
    let report = import_plcopen(xml);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let dir = write_imported("plcopen-nested", &report);
    let errors = check_dir(&dir);
    assert!(errors.is_empty(), "{errors:?}");
    // Both structs must be embedded for the file to be self-contained.
    let src = std::fs::read_to_string(dir.join("Cell.st")).unwrap();
    assert!(src.contains("TYPE Recipe :"), "{src}");
    assert!(src.contains("TYPE Machine :"), "{src}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn function_block_pou_end_to_end_compiles() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://www.plcopen.org/xml/tc6/0201">
  <types>
    <pous>
      <pou name="Motor" pouType="functionBlock">
        <interface>
          <inputVars><variable name="start"><type><BOOL/></type></variable></inputVars>
          <outputVars><variable name="run"><type><BOOL/></type></variable></outputVars>
        </interface>
        <body><ST><xhtml xmlns="http://www.w3.org/1999/xhtml">run := start;</xhtml></ST></body>
      </pou>
    </pous>
  </types>
</project>"#;
    let report = import_plcopen(xml);
    assert!(report.is_success(), "{:?}", report.errors);
    let dir = write_imported("plcopen-fb", &report);
    let errors = check_dir(&dir);
    assert!(errors.is_empty(), "{errors:?}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn l5x_routine_that_uses_controller_tags_fails_into_errors() {
    // The flat model cannot see controller-scope tags from a program POU;
    // such a routine must fail into report.errors, not produce output.
    let xml = r#"
<RSLogix5000Content TargetType="Controller" TargetName="C1">
  <Controller Use="Target" Name="C1">
    <Tags>
      <Tag Name="Global" TagType="Base" DataType="BOOL" Constant="false"/>
    </Tags>
    <Programs>
      <Program Use="Target" Name="P" Type="Normal">
        <Routines>
          <Routine Use="Target" Name="Main" Type="ST">
            <STContent><Line Number="0">Global := TRUE;</Line></STContent>
          </Routine>
        </Routines>
      </Program>
    </Programs>
  </Controller>
</RSLogix5000Content>"#;
    let report = import_l5x(xml);
    // The routine that references a controller-scope tag fails to compile
    // and is reported as an error; the controller tags file still imports.
    assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
    assert_eq!(report.errors[0].context, "P_Main.st");
    assert!(report.errors[0].message.contains("failed to compile"));
    assert_eq!(report.imported.len(), 1, "{:?}", report.imported);
    assert_eq!(report.imported[0].name, "C1_Tags.st");
    assert!(!report.is_success());
}
