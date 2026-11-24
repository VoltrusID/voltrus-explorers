//! Parser tests for the L5X reader against committed fixtures.

use mt_import::{ImportError, Skipped, import_l5x};

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).expect("fixture readable")
}

#[test]
fn clean_l5x_imports_tags_and_routine() {
    let report = import_l5x(&fixture("l5x_clean.l5x"));
    assert!(report.errors.is_empty(), "{:?}", report.errors);

    // Controller tags file + the one ST routine file.
    assert_eq!(report.imported.len(), 2, "{:?}", report.imported);
    let names: Vec<&str> = report.imported.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"Line1_Tags.st"), "{names:?}");
    assert!(names.contains(&"Conveyor_Main.st"), "{names:?}");

    // Controller tags file: 7 mappable tags, CONTROL skipped with a reason.
    let controller = report
        .imported
        .iter()
        .find(|f| f.name == "Line1_Tags.st")
        .unwrap();
    assert!(controller.st_source.contains("SystemOk : BOOL;"));
    assert!(controller.st_source.contains("PartCount : DINT;"));
    assert!(controller.st_source.contains("SpeedSetpoint : REAL;"));
    assert!(controller.st_source.contains("RecipeId : INT;"));
    assert!(controller.st_source.contains("BatchLabel : STRING;"));
    assert!(controller.st_source.contains("CycleTimer : TON;"));
    assert!(controller.st_source.contains("RejectCounter : CTUD;"));
    assert!(!controller.st_source.contains("CONTROL"));

    // Program routine file carries the program tags as VAR block + body.
    let routine = report
        .imported
        .iter()
        .find(|f| f.name == "Conveyor_Main.st")
        .unwrap();
    assert!(routine.st_source.contains("PROGRAM Conveyor_Main"));
    assert!(routine.st_source.contains("MotorStart : BOOL;"));
    assert!(routine.st_source.contains("RunTimer : TON;"));
    assert!(routine.st_source.contains("ProductCount : CTUD;"));
    assert!(routine.st_source.contains("MotorStart := TRUE;"));
    assert!(
        routine
            .st_source
            .contains("RunTimer(IN := MotorStart, PT := T#500MS);")
    );

    // UDT-typed tag skipped with the type named; never silent.
    let udt = report
        .skipped
        .iter()
        .find(|s| s.name == "Conveyor/Tags/RecipeData")
        .expect("UDT tag skip is reported");
    assert!(udt.reason.contains("RecipeUDT"), "{}", udt.reason);
    let control = report
        .skipped
        .iter()
        .find(|s| s.name == "Controller/Tags/SequencerStep")
        .expect("CONTROL tag skip is reported");
    assert!(control.reason.contains("sequencer"), "{}", control.reason);
}

#[test]
fn clean_l5x_report_serializes_deterministically() {
    let a = import_l5x(&fixture("l5x_clean.l5x"));
    let b = import_l5x(&fixture("l5x_clean.l5x"));
    assert_eq!(a, b, "same input must produce an identical report");
    let json = serde_json::to_string(&a).expect("report is JSON-serializable");
    assert!(json.contains("Line1_Tags.st"));
}

#[test]
fn ld_routine_is_skipped_with_reason() {
    let report = import_l5x(&fixture("l5x_ld_routine.l5x"));
    // Controller tags file (Packaging_Tags.st) plus the ST routine.
    assert_eq!(report.imported.len(), 2, "{:?}", report.imported);
    assert!(report.imported.iter().any(|f| f.name == "Packer_Timing.st"));

    let ladder = report
        .skipped
        .iter()
        .find(|s| s.name == "Packer/Routines/Main")
        .expect("RLL routine skip is reported");
    assert!(ladder.reason.contains("ladder"), "{}", ladder.reason);
    assert!(
        ladder.reason.contains("graphics sheet"),
        "{}",
        ladder.reason
    );
    assert!(report.errors.is_empty(), "{:?}", report.errors);
}

#[test]
fn malformed_l5x_produces_parse_error_and_no_files() {
    let report = import_l5x(&fixture("l5x_malformed.l5x"));
    assert!(report.imported.is_empty());
    assert_eq!(report.skipped.len(), 0, "malformed input has no skips");
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].context, "parse");
    assert!(!report.is_success());
}

#[test]
fn non_l5x_root_is_a_parse_error() {
    let report = import_l5x("<project xmlns='http://www.plcopen.org/xml/tc6/0201'></project>");
    assert!(report.imported.is_empty());
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].context, "parse");
    assert!(report.errors[0].message.contains("RSLogix5000Content"));
}

#[test]
fn alias_and_array_tags_are_skipped_with_reasons() {
    let xml = r#"
<RSLogix5000Content TargetType="Controller" TargetName="C1">
  <Controller Use="Target" Name="C1">
    <Tags>
      <Tag Name="Aliased" TagType="Alias" DataType="BOOL" AliasFor="Other" Constant="false"/>
      <Tag Name="Matrix" TagType="Base" DataType="DINT" Dimensions="[0,3]" Constant="false"/>
      <Tag Name="Fine" TagType="Base" DataType="BOOL" Constant="false"/>
    </Tags>
  </Controller>
</RSLogix5000Content>"#;
    let report = import_l5x(xml);
    assert_eq!(report.imported.len(), 1, "{:?}", report.imported);
    let skips: Vec<&Skipped> = report
        .skipped
        .iter()
        .filter(|s| s.name.starts_with("Controller/Tags/"))
        .collect();
    assert_eq!(skips.len(), 2, "{skips:?}");
    assert!(
        skips
            .iter()
            .any(|s| s.name.ends_with("Aliased") && s.reason.contains("alias"))
    );
    assert!(
        skips
            .iter()
            .any(|s| s.name.ends_with("Matrix") && s.reason.contains("[0,3]"))
    );
    // The mappable tag survived in the tags-only POU.
    assert!(report.imported[0].st_source.contains("Fine : BOOL;"));
}

#[test]
fn produced_and_consumed_tags_import_as_plain_variables() {
    let xml = r#"
<RSLogix5000Content TargetType="Controller" TargetName="C1">
  <Controller Use="Target" Name="C1">
    <Tags>
      <Tag Name="Out" TagType="Produced" DataType="DINT" Constant="false"/>
      <Tag Name="In" TagType="Consumed" DataType="DINT" Constant="false"/>
    </Tags>
  </Controller>
</RSLogix5000Content>"#;
    let report = import_l5x(xml);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.imported[0].st_source.contains("Out : DINT;"));
    assert!(report.imported[0].st_source.contains("In : DINT;"));
}

#[test]
fn context_program_is_skipped_as_a_unit() {
    let xml = r#"
<RSLogix5000Content TargetType="Controller" TargetName="C1">
  <Controller Use="Target" Name="C1">
    <Programs>
      <Program Use="Context" Name="RefOnly" Type="Normal">
        <Tags>
          <Tag Name="X" TagType="Base" DataType="BOOL" Constant="false"/>
        </Tags>
      </Program>
      <Program Use="Target" Name="Real" Type="Normal" MainRoutineName="Main">
        <Tags>
          <Tag Name="X" TagType="Base" DataType="BOOL" Constant="false"/>
        </Tags>
        <Routines>
          <Routine Use="Target" Name="Main" Type="ST">
            <STContent><Line Number="0">X := TRUE;</Line></STContent>
          </Routine>
        </Routines>
      </Program>
    </Programs>
  </Controller>
</RSLogix5000Content>"#;
    let report = import_l5x(xml);
    let ctx = report
        .skipped
        .iter()
        .find(|s| s.name == "RefOnly")
        .expect("context program skip is reported");
    assert!(ctx.reason.contains("context"), "{}", ctx.reason);
    assert_eq!(report.imported.len(), 1);
    assert_eq!(report.imported[0].name, "Real_Main.st");
}

#[test]
fn st_routine_without_content_is_an_error() {
    let xml = r#"
<RSLogix5000Content TargetType="Controller" TargetName="C1">
  <Controller Use="Target" Name="C1">
    <Programs>
      <Program Use="Target" Name="P" Type="Normal">
        <Routines>
          <Routine Use="Target" Name="Empty" Type="ST"/>
        </Routines>
      </Program>
    </Programs>
  </Controller>
</RSLogix5000Content>"#;
    let report = import_l5x(xml);
    assert!(report.imported.is_empty());
    assert_eq!(report.errors.len(), 1);
    assert_eq!(
        report.errors[0],
        ImportError {
            context: "P/Routines/Empty".into(),
            message: "ST routine has no STContent lines".into(),
        }
    );
}
