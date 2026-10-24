//! Parser tests for the PLCopen TC6 reader against committed fixtures.

use mt_import::{Skipped, import_plcopen};

fn fixture(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).expect("fixture readable")
}

#[test]
fn clean_plcopen_imports_pou_with_interface_struct_and_body() {
    let report = import_plcopen(&fixture("plcopen_clean.xml"));
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.imported.len(), 1, "{:?}", report.imported);
    let file = &report.imported[0];
    assert_eq!(file.name, "Main.st");

    // Struct datatype emitted as TYPE block (reachable from recipe var).
    assert!(
        file.st_source.contains("TYPE Recipe :"),
        "{}",
        file.st_source
    );
    assert!(file.st_source.contains("STRUCT"), "{}", file.st_source);
    assert!(
        file.st_source.contains("target : REAL;"),
        "{}",
        file.st_source
    );
    assert!(
        file.st_source.contains("enabled : BOOL;"),
        "{}",
        file.st_source
    );
    assert!(
        file.st_source.contains("label : STRING;"),
        "{}",
        file.st_source
    );
    assert!(file.st_source.contains("END_STRUCT;"), "{}", file.st_source);

    // Interface mapped to IEC var blocks, including initial values.
    assert!(
        file.st_source.contains("PROGRAM Main"),
        "{}",
        file.st_source
    );
    assert!(
        file.st_source
            .contains("VAR_INPUT\n    start_cmd : BOOL := FALSE;"),
        "{}",
        file.st_source
    );
    assert!(file.st_source.contains("VAR_OUTPUT"), "{}", file.st_source);
    assert!(
        file.st_source.contains("running : BOOL;"),
        "{}",
        file.st_source
    );
    assert!(file.st_source.contains("VAR_IN_OUT"), "{}", file.st_source);
    assert!(
        file.st_source.contains("recipe : Recipe;"),
        "{}",
        file.st_source
    );
    assert!(
        file.st_source.contains("cycle_count : DINT := 0;"),
        "{}",
        file.st_source
    );
    // derived name="TON" becomes a function block instance.
    assert!(
        file.st_source.contains("delay : TON;"),
        "{}",
        file.st_source
    );

    // ST body preserved.
    assert!(
        file.st_source.contains("running := start_cmd;"),
        "{}",
        file.st_source
    );
    assert!(
        file.st_source
            .contains("recipe.target := recipe.target * 2.0;"),
        "{}",
        file.st_source
    );
    assert!(file.st_source.trim_end().ends_with("END_PROGRAM"));

    // instances + subrange datatype skipped with reasons.
    let instances = report
        .skipped
        .iter()
        .find(|s| s.name == "instances")
        .expect("instances skip is reported");
    assert!(instances.reason.contains("task"), "{}", instances.reason);
    let subrange = report
        .skipped
        .iter()
        .find(|s| s.name == "dataTypes/SpeedLimit")
        .expect("subrange datatype skip is reported");
    assert!(subrange.reason.contains("subrange"), "{}", subrange.reason);
}

#[test]
fn clean_plcopen_is_deterministic() {
    let a = import_plcopen(&fixture("plcopen_clean.xml"));
    let b = import_plcopen(&fixture("plcopen_clean.xml"));
    assert_eq!(a, b);
}

#[test]
fn malformed_plcopen_produces_parse_error_and_no_files() {
    let report = import_plcopen(&fixture("plcopen_malformed.xml"));
    assert!(report.imported.is_empty());
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].context, "parse");
    assert!(!report.is_success());
}

#[test]
fn non_plcopen_root_is_a_parse_error() {
    let report = import_plcopen("<RSLogix5000Content/>");
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].context, "parse");
    assert!(report.errors[0].message.contains("project"));
}

#[test]
fn function_pou_is_skipped_with_reason() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://www.plcopen.org/xml/tc6/0201">
  <types>
    <pous>
      <pou name="Scale" pouType="function">
        <interface>
          <returnType><REAL/></returnType>
          <inputVars><variable name="x"><type><REAL/></type></variable></inputVars>
        </interface>
        <body><ST><xhtml xmlns="http://www.w3.org/1999/xhtml">Scale := x;</xhtml></ST></body>
      </pou>
    </pous>
  </types>
</project>"#;
    let report = import_plcopen(xml);
    assert!(report.imported.is_empty());
    let skip = report
        .skipped
        .iter()
        .find(|s| s.name == "Scale")
        .expect("function POU skip is reported");
    assert!(skip.reason.contains("FUNCTION"), "{}", skip.reason);
    assert!(skip.reason.contains("code generator"), "{}", skip.reason);
}

#[test]
fn il_and_ld_bodies_are_skipped_but_st_body_imports() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://www.plcopen.org/xml/tc6/0201">
  <types>
    <pous>
      <pou name="Mixed" pouType="program">
        <interface>
          <localVars><variable name="x"><type><BOOL/></type></variable></localVars>
        </interface>
        <body>
          <LD/>
        </body>
        <body>
          <ST><xhtml xmlns="http://www.w3.org/1999/xhtml">x := 1;</xhtml></ST>
        </body>
      </pou>
      <pou name="LadderOnly" pouType="program">
        <interface/>
        <body>
          <LD/>
        </body>
      </pou>
    </pous>
  </types>
</project>"#;
    let report = import_plcopen(xml);
    // Mixed: LD body skipped, ST body imported.
    assert_eq!(report.imported.len(), 1, "{:?}", report.imported);
    assert!(report.imported[0].st_source.contains("x := 1;"));
    let ld = report
        .skipped
        .iter()
        .find(|s| s.name == "Mixed/body#LD")
        .expect("LD body skip is reported");
    assert!(ld.reason.contains("graphics sheet"), "{}", ld.reason);
    // LadderOnly has no ST body at all -> nothing imported from it.
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.name == "LadderOnly/body#LD")
    );
}

#[test]
fn enum_and_array_datatypes_are_skipped_individually() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://www.plcopen.org/xml/tc6/0201">
  <types>
    <dataTypes>
      <dataType name="Mode">
        <baseType>
          <enum><values><value name="OFF"/><value name="ON"/></values></enum>
        </baseType>
      </dataType>
      <dataType name="Buf">
        <baseType>
          <array><dimension lower="0" upper="9"/><baseType><INT/></baseType></array>
        </baseType>
      </dataType>
    </dataTypes>
    <pous/>
  </types>
</project>"#;
    let report = import_plcopen(xml);
    assert!(report.imported.is_empty());
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.name == "dataTypes/Mode" && s.reason.contains("enum"))
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.name == "dataTypes/Buf" && s.reason.contains("array"))
    );
}

#[test]
fn unsupported_variable_types_are_skipped_with_reasons() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://www.plcopen.org/xml/tc6/0201">
  <types>
    <pous>
      <pou name="P" pouType="program">
        <interface>
          <localVars>
            <variable name="when"><type><DATE/></type></variable>
            <variable name="mystery"><type><derived name="Nope"/></type></variable>
            <variable name="ok"><type><BOOL/></type></variable>
            <variable name="broke"/>
          </localVars>
        </interface>
        <body><ST><xhtml xmlns="http://www.w3.org/1999/xhtml">ok := TRUE;</xhtml></ST></body>
      </pou>
    </pous>
  </types>
</project>"#;
    let report = import_plcopen(xml);
    assert_eq!(report.imported.len(), 1, "{:?}", report.imported);
    let src = &report.imported[0].st_source;
    assert!(src.contains("ok : BOOL;"), "{src}");
    assert!(!src.contains("when"), "{src}");
    assert!(!src.contains("mystery"), "{src}");
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.name == "P/when" && s.reason.contains("DATE"))
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.name == "P/mystery" && s.reason.contains("Nope"))
    );
    // A variable without any <type> element is malformed input for that
    // variable and is reported as an error.
    assert!(
        report
            .errors
            .iter()
            .any(|e| e.context == "P/broke" && e.message.contains("no <type>"))
    );
    assert!(!src.contains("broke"), "{src}");
}

#[test]
fn external_vars_and_struct_literals_are_skipped() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://www.plcopen.org/xml/tc6/0201">
  <types>
    <pous>
      <pou name="P" pouType="program">
        <interface>
          <externalVars>
            <variable name="g"><type><BOOL/></type></variable>
          </externalVars>
          <localVars>
            <variable name="r"><type><derived name="Recipe"/></type>
              <initialValue><structValue/></initialValue>
            </variable>
            <variable name="plain"><type><BOOL/></type></variable>
          </localVars>
        </interface>
        <body><ST><xhtml xmlns="http://www.w3.org/1999/xhtml">plain := TRUE;</xhtml></ST></body>
      </pou>
    </pous>
  </types>
</project>"#;
    let report = import_plcopen(xml);
    assert_eq!(report.imported.len(), 1, "{:?}", report.imported);
    let src = &report.imported[0].st_source;
    assert!(src.contains("plain : BOOL;"), "{src}");
    assert!(!src.contains("g :"), "{src}");
    assert!(!src.contains("r :"), "{src}");
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.name == "P/g" && s.reason.contains("external"))
    );
    assert!(report.skipped.iter().any(|s| s.name == "P/r"));
}

#[test]
fn function_block_pou_maps_to_function_block() {
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
    assert_eq!(report.imported.len(), 1);
    let src = &report.imported[0].st_source;
    assert!(src.contains("FUNCTION_BLOCK Motor"), "{src}");
    assert!(src.trim_end().ends_with("END_FUNCTION_BLOCK"));
}

#[test]
fn entities_in_st_body_are_resolved() {
    // The iecst runtime has no `&` operator, so the resolved entity must be
    // a `<` comparison for the generated source to compile.
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://www.plcopen.org/xml/tc6/0201">
  <types>
    <pous>
      <pou name="P" pouType="program">
        <interface>
          <localVars><variable name="x"><type><DINT/></type></variable></localVars>
        </interface>
        <body><ST><xhtml xmlns="http://www.w3.org/1999/xhtml">x := 1 &lt; 2;</xhtml></ST></body>
      </pou>
    </pous>
  </types>
</project>"#;
    let report = import_plcopen(xml);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.imported[0].st_source.contains("x := 1 < 2;"));
}

#[test]
fn skipped_report_entries_are_typed() {
    let report = import_plcopen(&fixture("plcopen_clean.xml"));
    let skip: &Skipped = &report.skipped[0];
    assert!(!skip.name.is_empty());
    assert!(!skip.reason.is_empty());
}
