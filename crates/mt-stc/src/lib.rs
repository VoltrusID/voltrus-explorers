pub mod codegen;
pub mod stdlib;

use mt_plc_types::CompiledModule;
use serde::{Deserialize, Serialize};

pub struct CompileOutput {
    pub success: bool,
    pub diagnostics: Vec<Diagnostic>,
    pub pou_count: usize,
    pub variable_count: usize,
    pub module: Option<CompiledModule>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Diagnostic {
    pub severity: String,
    pub message: String,
    pub line: usize,
    pub col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub source: String,
}

fn byte_offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 0;
    for (i, ch) in source.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Strip TYPE...END_TYPE blocks from source for POU parsing.
fn strip_type_blocks(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut depth = 0;
    let mut in_type = false;
    let upper = source.to_uppercase();

    let mut i = 0;
    let chars: Vec<char> = source.chars().collect();
    let upper_chars: Vec<char> = upper.chars().collect();

    while i < chars.len() {
        if !in_type && i + 4 <= upper_chars.len() && &upper_chars[i..i + 4] == ['T', 'Y', 'P', 'E']
        {
            // Check it's "TYPE" not part of a longer word
            let before_ok = i == 0 || !chars[i - 1].is_alphanumeric();
            let after_ok = i + 4 >= upper_chars.len() || !chars[i + 4].is_alphanumeric();
            if before_ok && after_ok {
                in_type = true;
                depth = 0;
                i += 4;
                continue;
            }
        }
        if in_type
            && i + 8 <= upper_chars.len()
            && &upper_chars[i..i + 8] == ['E', 'N', 'D', '_', 'T', 'Y', 'P', 'E']
        {
            let before_ok = i == 0 || !chars[i - 1].is_alphanumeric();
            let after_ok = i + 8 >= upper_chars.len() || !chars[i + 8].is_alphanumeric();
            if before_ok && after_ok {
                in_type = false;
                i += 8;
                continue;
            }
        }
        if !in_type {
            result.push(chars[i]);
        }
        i += 1;
    }

    result.trim().to_string()
}

pub fn compile(source: &str) -> CompileOutput {
    let mut diagnostics = Vec::new();
    let pou_count: usize;
    let mut variable_count = 0;

    // Try parsing TYPE blocks from the original source
    let type_decls = iecst::parse_type_block(source).unwrap_or_default();

    // Strip TYPE blocks for POU parsing
    let pou_source = strip_type_blocks(source);

    match iecst::parse_pou(&pou_source) {
        Ok(pou) => {
            pou_count = 1;

            for block in &pou.var_blocks {
                variable_count += block.vars.len();
            }

            let analysis_diagnostics = iecst::analyze_pou(&pou);
            for d in &analysis_diagnostics {
                let msg = d.to_string();
                if msg.contains("unused variable") || msg.contains("type mismatch") {
                    continue;
                }
                let (start_line, start_col) = byte_offset_to_line_col(source, d.span.start);
                let (end_line, end_col) = byte_offset_to_line_col(source, d.span.end);
                diagnostics.push(Diagnostic {
                    severity: format!("{:?}", d.severity).to_lowercase(),
                    message: msg,
                    line: start_line,
                    col: start_col,
                    end_line,
                    end_col,
                    source: "analysis".to_string(),
                });
            }

            let has_errors = diagnostics.iter().any(|d| d.severity == "error");

            let module = if !has_errors {
                let mut codegen = codegen::CodeGen::new();
                codegen.register_types(&type_decls);
                match codegen.compile_pou(&pou, source) {
                    Ok(compiled) => Some(compiled),
                    Err(codegen_errors) => {
                        for e in codegen_errors {
                            diagnostics.push(e);
                        }
                        None
                    }
                }
            } else {
                None
            };

            let success = diagnostics.iter().all(|d| d.severity != "error");

            CompileOutput {
                success,
                diagnostics,
                pou_count,
                variable_count,
                module,
            }
        }
        Err(e) => {
            diagnostics.push(Diagnostic {
                severity: "error".to_string(),
                message: format!("{}", e),
                line: 1,
                col: 1,
                end_line: 1,
                end_col: 5,
                source: "parser".to_string(),
            });
            CompileOutput {
                success: false,
                diagnostics,
                pou_count: 0,
                variable_count: 0,
                module: None,
            }
        }
    }
}

/// Compile multiple POUs into a single module with cross-POU call support.
/// Sources: Vec<(file_name, source_code)>. First POU must be a PROGRAM.
pub fn compile_project(sources: &[(String, String)]) -> CompileOutput {
    let mut diagnostics = Vec::new();
    let mut poutines: Vec<iecst::Pou> = Vec::new();
    let mut pou_names: Vec<String> = Vec::new();

    // Collect all TYPE definitions from all source files
    let mut all_type_decls: Vec<iecst::TypeDecl> = Vec::new();
    for (_name, source) in sources {
        if let Ok(types) = iecst::parse_type_block(source) {
            all_type_decls.extend(types);
        }
    }

    for (name, source) in sources {
        match iecst::parse_pou(source) {
            Ok(pou) => {
                let analysis = iecst::analyze_pou(&pou);
                for d in &analysis {
                    let msg = d.to_string();
                    if msg.contains("unused variable") || msg.contains("type mismatch") {
                        continue;
                    }
                    let (start_line, start_col) = byte_offset_to_line_col(source, d.span.start);
                    let (end_line, end_col) = byte_offset_to_line_col(source, d.span.end);
                    diagnostics.push(Diagnostic {
                        severity: format!("{:?}", d.severity).to_lowercase(),
                        message: msg,
                        line: start_line,
                        col: start_col,
                        end_line,
                        end_col,
                        source: format!("{}:analysis", name),
                    });
                }
                pou_names.push(pou.name.clone());
                poutines.push(pou);
            }
            Err(e) => {
                diagnostics.push(Diagnostic {
                    severity: "error".to_string(),
                    message: format!("{}: {}", name, e),
                    line: 1,
                    col: 1,
                    end_line: 1,
                    end_col: 5,
                    source: "parser".to_string(),
                });
            }
        }
    }

    let has_errors = diagnostics.iter().any(|d| d.severity == "error");

    if has_errors || poutines.is_empty() {
        return CompileOutput {
            success: false,
            diagnostics,
            pou_count: poutines.len(),
            variable_count: 0,
            module: None,
        };
    }

    let pou_count = poutines.len();
    let mut pou_table: Vec<(String, usize)> = Vec::new();
    let mut all_instructions: Vec<mt_plc_types::Instruction> = Vec::new();
    let mut all_variables: Vec<(String, mt_plc_types::PlcValue)> = Vec::new();
    let mut all_name_constants: Vec<String> = Vec::new();
    let mut all_line_map: Vec<(usize, usize)> = Vec::new();

    // First pass: compile each POU separately, track entry points
    for (pou_idx, pou) in poutines.iter().enumerate() {
        let entry_ip = all_instructions.len();
        pou_table.push((pou.name.clone(), entry_ip));

        let source = &sources[pou_idx].1;
        let mut codegen = codegen::CodeGen::new();
        codegen.register_types(&all_type_decls);
        match codegen.compile_pou(pou, source) {
            Ok(mut module) => {
                // Rewrite variable slot indices to avoid collisions
                let slot_offset = all_variables.len();
                let name_offset = all_name_constants.len();
                for instr in &mut module.instructions {
                    match instr.opcode {
                        mt_plc_types::Opcode::LoadVar | mt_plc_types::Opcode::StoreVar => {
                            if let Some(ref mut slot) = instr.operand {
                                *slot += slot_offset as i32;
                            }
                        }
                        mt_plc_types::Opcode::CallPou => {
                            if let Some(ref mut idx) = instr.operand {
                                *idx += name_offset as i32;
                            }
                        }
                        _ => {}
                    }
                }
                all_variables.extend(module.variable_slots);
                all_instructions.extend(module.instructions);
                all_name_constants.extend(module.name_constants);
                for (instr_idx, line) in &module.line_map {
                    all_line_map.push((instr_idx + entry_ip, *line));
                }
            }
            Err(errs) => {
                diagnostics.extend(errs);
            }
        }
    }

    let has_codegen_errors = diagnostics.iter().any(|d| d.severity == "error");
    let variable_count = all_variables.len();

    if has_codegen_errors || pou_count == 0 {
        CompileOutput {
            success: false,
            diagnostics,
            pou_count,
            variable_count,
            module: None,
        }
    } else {
        CompileOutput {
            success: true,
            diagnostics,
            pou_count,
            variable_count,
            module: Some(CompiledModule {
                name: poutines[0].name.clone(),
                instructions: all_instructions,
                variable_slots: all_variables,
                pou_table,
                name_constants: all_name_constants,
                line_map: all_line_map,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_simple_program() {
        let source = r#"
            PROGRAM Main
            VAR
                x : INT;
            END_VAR
                x := 1 + 2;
            END_PROGRAM
        "#;
        let result = compile(source);
        assert!(result.success);
        assert_eq!(result.pou_count, 1);
        assert_eq!(result.variable_count, 1);
        assert!(result.module.is_some());
        let module = result.module.unwrap();
        assert_eq!(module.name, "Main");
        assert!(!module.instructions.is_empty());
    }

    #[test]
    fn test_compile_syntax_error() {
        let source = "PROGRAM Main VAR x := ; END_PROGRAM";
        let result = compile(source);
        assert!(!result.success);
        assert!(!result.diagnostics.is_empty());
        assert!(result.module.is_none());
    }

    #[test]
    fn test_compile_function_block() {
        let source = r#"
            FUNCTION_BLOCK Motor
            VAR_INPUT
                enable : BOOL;
            END_VAR
            VAR_OUTPUT
                speed : INT;
            END_VAR
                IF enable THEN
                    speed := 100;
                END_IF;
            END_FUNCTION_BLOCK
        "#;
        let result = compile(source);
        assert!(result.success);
        assert_eq!(result.pou_count, 1);
        assert!(result.module.is_some());
    }

    #[test]
    fn test_compile_struct_type() {
        // Test TYPE block parsing and codegen struct support.
        // parse_type_block and parse_pou handle TYPE and POU separately.
        let type_source = r#"
            TYPE MotorConfig :
            STRUCT
                speed : INT;
                enabled : BOOL;
            END_STRUCT
            END_TYPE
        "#;

        // Verify TYPE parsing succeeds
        let type_decls = iecst::parse_type_block(type_source).expect("TYPE parsing should succeed");
        assert_eq!(type_decls.len(), 1, "Expected 1 type declaration");
        assert_eq!(type_decls[0].name, "MotorConfig");

        // Now compile a POU that uses the struct type
        let pou_source = r#"
            PROGRAM Main
            VAR
                cfg : MotorConfig;
            END_VAR
                cfg.speed := 100;
                cfg.enabled := TRUE;
            END_PROGRAM
        "#;
        // Manually register the types and compile
        let pou = iecst::parse_pou(pou_source).expect("POU parsing should succeed");
        let mut codegen = codegen::CodeGen::new();
        codegen.register_types(&type_decls);
        let module = codegen.compile_pou(&pou, pou_source);
        assert!(module.is_ok(), "Codegen failed: {:?}", module.err());
        let module = module.unwrap();
        assert_eq!(module.name, "Main");
        let cfg_var = module.variable_slots.iter().find(|(n, _)| n == "cfg");
        assert!(cfg_var.is_some(), "Expected 'cfg' variable");
        let (_, val) = cfg_var.unwrap();
        assert_eq!(val.type_name(), "STRUCT");
    }

    #[test]
    fn test_compile_enum_type() {
        let type_source = r#"
            TYPE Color :
            ( Red, Green, Blue )
            END_TYPE
        "#;

        // Verify TYPE parsing succeeds
        let type_decls = iecst::parse_type_block(type_source).expect("TYPE parsing should succeed");
        assert_eq!(type_decls.len(), 1, "Expected 1 type declaration");
        assert_eq!(type_decls[0].name, "Color");

        // Now compile a POU that uses the enum type
        let pou_source = r#"
            PROGRAM Main
            VAR
                c : Color;
            END_VAR
            END_PROGRAM
        "#;
        let pou = iecst::parse_pou(pou_source).expect("POU parsing should succeed");
        let mut codegen = codegen::CodeGen::new();
        codegen.register_types(&type_decls);
        let module = codegen.compile_pou(&pou, pou_source);
        assert!(module.is_ok(), "Codegen failed: {:?}", module.err());
        let module = module.unwrap();
        assert_eq!(module.name, "Main");
        // The enum variable should have been created
        assert!(
            module.variable_slots.iter().any(|(n, _)| n == "c"),
            "Expected 'c' variable"
        );
        let c_var = module
            .variable_slots
            .iter()
            .find(|(n, _)| n == "c")
            .unwrap();
        assert_eq!(c_var.1.type_name(), "ENUM");
    }

    #[test]
    fn test_compile_project_multi_pou() {
        let sources = vec![
            (
                "Main".to_string(),
                r#"
                PROGRAM Main
                VAR
                    x : INT;
                END_VAR
                    x := 1;
                END_PROGRAM
            "#
                .to_string(),
            ),
            (
                "Helper".to_string(),
                r#"
                FUNCTION_BLOCK Helper
                VAR_INPUT
                    val : INT;
                END_VAR
                VAR_OUTPUT
                    result : INT;
                END_VAR
                    result := val + 1;
                END_FUNCTION_BLOCK
            "#
                .to_string(),
            ),
        ];
        let result = compile_project(&sources);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        assert_eq!(
            result.pou_count, 2,
            "Expected 2 POUs, got {}",
            result.pou_count
        );
        assert!(result.module.is_some());
        let module = result.module.unwrap();
        assert_eq!(module.pou_table.len(), 2);
    }

    #[test]
    fn test_compile_while_loop() {
        let source = r#"
            PROGRAM Main
            VAR
                i : INT;
                sum : INT;
            END_VAR
                i := 0;
                sum := 0;
                WHILE i < 5 DO
                    sum := sum + i;
                    i := i + 1;
                END_WHILE;
            END_PROGRAM
        "#;
        let result = compile(source);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        assert!(result.module.is_some());
        let module = result.module.unwrap();
        assert!(!module.instructions.is_empty());
        // Should have at least a Halt instruction
        assert!(
            module
                .instructions
                .iter()
                .any(|i| i.opcode == mt_plc_types::Opcode::Halt)
        );
    }

    #[test]
    fn test_compile_repeat_loop() {
        let source = r#"
            PROGRAM Main
            VAR
                x : INT;
            END_VAR
                x := 0;
                REPEAT
                    x := x + 1;
                UNTIL x >= 10;
            END_PROGRAM
        "#;
        let result = compile(source);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        assert!(result.module.is_some());
    }

    #[test]
    fn test_compile_for_loop() {
        let source = r#"
            PROGRAM Main
            VAR
                i : INT;
                total : INT;
            END_VAR
                total := 0;
                FOR i := 1 TO 10 DO
                    total := total + i;
                END_FOR;
            END_PROGRAM
        "#;
        let result = compile(source);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        assert!(result.module.is_some());
        let module = result.module.unwrap();
        // FOR loop should generate LoadVar + Le + JumpIfFalse pattern
        assert!(module.instructions.len() > 5);
    }

    #[test]
    fn test_compile_case_statement() {
        let source = r#"
            PROGRAM Main
            VAR
                cmd : INT;
                result : INT;
            END_VAR
                CASE cmd OF
                    1: result := 10;
                    2: result := 20;
                ELSE
                    result := 0;
                END_CASE;
            END_PROGRAM
        "#;
        let result = compile(source);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        assert!(result.module.is_some());
    }

    #[test]
    fn test_compile_array_access() {
        let source = r#"
            PROGRAM Main
            VAR
                arr : ARRAY[0..4] OF INT;
                idx : INT;
                val : INT;
            END_VAR
                idx := 2;
                arr[idx] := 42;
                val := arr[idx];
            END_PROGRAM
        "#;
        let result = compile(source);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        assert!(result.module.is_some());
        let module = result.module.unwrap();
        // arr should be an Array of 5 elements
        let arr_var = module.variable_slots.iter().find(|(n, _)| n == "arr");
        assert!(arr_var.is_some(), "Expected 'arr' variable");
        if let (_, val) = arr_var.unwrap() {
            assert_eq!(val.type_name(), "ARRAY");
        }
    }

    #[test]
    fn test_compile_timer_ton() {
        let source = r#"
            PROGRAM Main
            VAR
                start : BOOL;
                motor : BOOL;
                delay : TON;
            END_VAR
                delay(IN := start, PT := T#500MS);
                motor := delay.Q;
            END_PROGRAM
        "#;
        let result = compile(source);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        assert!(result.module.is_some());
        let module = result.module.unwrap();
        // Timer should generate TonCall opcode and hidden Q/ET variable slots
        assert!(
            module
                .instructions
                .iter()
                .any(|i| i.opcode == mt_plc_types::Opcode::TonCall),
            "Expected TonCall instruction"
        );
        assert!(
            module.variable_slots.iter().any(|(n, _)| n == "delay.Q"),
            "Expected delay.Q variable slot"
        );
        assert!(
            module.variable_slots.iter().any(|(n, _)| n == "delay.ET"),
            "Expected delay.ET variable slot"
        );
    }

    #[test]
    fn test_compile_ctu_counter() {
        let source = r#"
            PROGRAM Main
            VAR
                pulse : BOOL;
                reset : BOOL;
                done : BOOL;
                cnt : CTU;
            END_VAR
                cnt(CU := pulse, R := reset, PV := 10);
                done := cnt.Q;
            END_PROGRAM
        "#;
        let result = compile(source);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        assert!(result.module.is_some());
        let module = result.module.unwrap();
        assert!(
            module
                .instructions
                .iter()
                .any(|i| i.opcode == mt_plc_types::Opcode::CtuCall),
            "Expected CtuCall instruction"
        );
    }

    #[test]
    fn test_compile_ctud_counter() {
        let source = r#"
            PROGRAM Main
            VAR
                cu : BOOL;
                cd : BOOL;
                r : BOOL;
                ld : BOOL;
                done : BOOL;
                cnt : CTUD;
            END_VAR
                cnt(CU := cu, CD := cd, R := r, LD := ld, PV := 10);
                done := cnt.Q;
            END_PROGRAM
        "#;
        let result = compile(source);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        assert!(result.module.is_some());
        let module = result.module.unwrap();
        assert!(
            module
                .instructions
                .iter()
                .any(|i| i.opcode == mt_plc_types::Opcode::CtudCall),
            "Expected CtudCall instruction"
        );
    }

    #[test]
    fn test_compile_edge_detection() {
        let source = r#"
            PROGRAM Main
            VAR
                sensor : BOOL;
                rising : BOOL;
                falling : BOOL;
                rtrig : R_TRIG;
                ftrig : F_TRIG;
            END_VAR
                rtrig(CLK := sensor);
                rising := rtrig.Q;
                ftrig(CLK := sensor);
                falling := ftrig.Q;
            END_PROGRAM
        "#;
        let result = compile(source);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        assert!(result.module.is_some());
        let module = result.module.unwrap();
        assert!(
            module
                .instructions
                .iter()
                .any(|i| i.opcode == mt_plc_types::Opcode::RTrigCall),
            "Expected RTrigCall instruction"
        );
        assert!(
            module
                .instructions
                .iter()
                .any(|i| i.opcode == mt_plc_types::Opcode::FTrigCall),
            "Expected FTrigCall instruction"
        );
    }

    #[test]
    fn test_compile_string_operations() {
        let source = r#"
            PROGRAM Main
            VAR
                msg : STRING;
            END_VAR
                msg := 'hello';
            END_PROGRAM
        "#;
        let result = compile(source);
        // String literals compile to PushDint(0) in current codegen, but should succeed
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        assert!(result.module.is_some());
        let module = result.module.unwrap();
        let msg_var = module.variable_slots.iter().find(|(n, _)| n == "msg");
        assert!(msg_var.is_some(), "Expected 'msg' variable");
        if let (_, val) = msg_var.unwrap() {
            assert_eq!(val.type_name(), "STRING");
        }
    }

    #[test]
    fn test_compile_math_expressions() {
        let source = r#"
            PROGRAM Main
            VAR
                a : INT;
                b : INT;
                c : INT;
                d : INT;
                result : INT;
            END_VAR
                a := 10;
                b := 3;
                c := 5;
                d := 2;
                result := a * b + c / d;
            END_PROGRAM
        "#;
        let result = compile(source);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        assert!(result.module.is_some());
        let module = result.module.unwrap();
        // Should contain Mul and Div opcodes
        assert!(
            module
                .instructions
                .iter()
                .any(|i| i.opcode == mt_plc_types::Opcode::Mul),
            "Expected Mul instruction"
        );
        assert!(
            module
                .instructions
                .iter()
                .any(|i| i.opcode == mt_plc_types::Opcode::Div),
            "Expected Div instruction"
        );
        assert!(
            module
                .instructions
                .iter()
                .any(|i| i.opcode == mt_plc_types::Opcode::Add),
            "Expected Add instruction"
        );
    }

    #[test]
    fn test_compile_exit_continue() {
        let source = r#"
            PROGRAM Main
            VAR
                i : INT;
                sum : INT;
            END_VAR
                sum := 0;
                FOR i := 1 TO 10 DO
                    IF i = 5 THEN
                        EXIT;
                    END_IF;
                    sum := sum + i;
                END_FOR;
            END_PROGRAM
        "#;
        let result = compile(source);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        let module = result.module.unwrap();
        // Should have Jump opcodes for EXIT
        assert!(
            module
                .instructions
                .iter()
                .any(|i| i.opcode == mt_plc_types::Opcode::Jump),
            "Expected Jump instruction for EXIT"
        );
    }

    #[test]
    fn test_compile_unknown_function_error() {
        let source = r#"
            PROGRAM Main
            VAR
                x : INT;
            END_VAR
                x := UNKNOWN_FUNC(42);
            END_PROGRAM
        "#;
        let result = compile(source);
        assert!(
            !result.success,
            "Expected compile to fail for unknown function"
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| d.message.contains("Unknown function")),
            "Expected 'Unknown function' error, got: {:?}",
            result.diagnostics
        );
    }
}
