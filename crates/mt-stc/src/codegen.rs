use mt_plc_types::{CompiledModule, Instruction, Opcode, PlcValue};
use std::collections::HashMap;

use crate::Diagnostic;

pub struct CodeGen {
    instructions: Vec<Instruction>,
    variables: Vec<(String, PlcValue)>,
    var_map: HashMap<String, usize>,
    errors: Vec<Diagnostic>,
    // Timer/Counter tracking: var_name -> (instance_index, hidden Q slot, hidden ET/CV slot)
    timers: HashMap<String, (usize, usize, usize)>,
    counters: HashMap<String, (usize, usize, usize)>,
    // Edge detection: var_name -> (instance_index, hidden Q slot)
    edges: HashMap<String, (usize, usize, bool)>, // (idx, q_slot, is_rising)
    timer_kinds: HashMap<String, String>,
    counter_kinds: HashMap<String, String>,
    // User-defined types: struct_name -> [(field_name, field_type)]
    struct_defs: HashMap<String, Vec<(String, iecst::TypeSpec)>>,
    // User-defined enums: enum_name -> [value_names]
    enum_defs: HashMap<String, Vec<String>>,
    next_timer_idx: usize,
    next_counter_idx: usize,
    next_edge_idx: usize,
    name_table: Vec<String>,
    line_map: Vec<(usize, usize)>,
    line_offsets: Vec<usize>,
    current_line: usize,
    // Loop nesting: (exit_jump_slots, continue_jump_slots)
    loop_stack: Vec<(Vec<usize>, Vec<usize>)>,
}

impl CodeGen {
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            variables: Vec::new(),
            var_map: HashMap::new(),
            errors: Vec::new(),
            timers: HashMap::new(),
            counters: HashMap::new(),
            timer_kinds: HashMap::new(),
            counter_kinds: HashMap::new(),
            edges: HashMap::new(),
            struct_defs: HashMap::new(),
            enum_defs: HashMap::new(),
            next_timer_idx: 0,
            next_counter_idx: 0,
            next_edge_idx: 0,
            name_table: Vec::new(),
            line_map: Vec::new(),
            line_offsets: Vec::new(),
            current_line: 1,
            loop_stack: Vec::new(),
        }
    }

    fn intern_name(&mut self, name: &str) -> usize {
        if let Some(idx) = self.name_table.iter().position(|n| n == name) {
            idx
        } else {
            let idx = self.name_table.len();
            self.name_table.push(name.to_string());
            idx
        }
    }

    fn build_line_offsets(&mut self, source: &str) {
        self.line_offsets = vec![0];
        for (i, ch) in source.char_indices() {
            if ch == '\n' {
                self.line_offsets.push(i + 1);
            }
        }
    }

    fn byte_offset_to_line(&self, offset: usize) -> usize {
        match self.line_offsets.binary_search(&offset) {
            Ok(line) => line + 1,
            Err(line) => line,
        }
    }

    fn byte_offset_to_col(&self, offset: usize) -> usize {
        let line_start = match self.line_offsets.binary_search(&offset) {
            Ok(line) => self.line_offsets[line],
            Err(line) => self
                .line_offsets
                .get(line.saturating_sub(1))
                .copied()
                .unwrap_or(0),
        };
        offset.saturating_sub(line_start)
    }

    fn push_error(&mut self, message: String, span: &iecst::Span) {
        let line = self.byte_offset_to_line(span.start);
        let col = self.byte_offset_to_col(span.start);
        let end_line = self.byte_offset_to_line(span.end);
        let end_col = self.byte_offset_to_col(span.end);
        self.errors.push(Diagnostic {
            severity: "error".into(),
            message,
            line,
            col,
            end_line,
            end_col,
            source: "codegen".into(),
        });
    }

    pub fn compile_pou(
        mut self,
        pou: &iecst::Pou,
        source: &str,
    ) -> Result<CompiledModule, Vec<Diagnostic>> {
        self.build_line_offsets(source);
        for block in &pou.var_blocks {
            for var in &block.vars {
                let type_name = match &var.var_type.kind {
                    iecst::TypeKind::Simple(name) => name.to_uppercase(),
                    _ => String::new(),
                };

                match type_name.as_str() {
                    "TON" | "TOF" | "TP" => {
                        let idx = self.next_timer_idx;
                        self.next_timer_idx += 1;
                        let q_slot = self.variables.len();
                        self.variables
                            .push((format!("{}.Q", var.name), PlcValue::Bool(false)));
                        let et_slot = self.variables.len();
                        self.variables
                            .push((format!("{}.ET", var.name), PlcValue::Time(0)));
                        self.timers.insert(var.name.clone(), (idx, q_slot, et_slot));
                        self.timer_kinds.insert(var.name.clone(), type_name.clone());
                    }
                    "CTU" | "CTD" | "CTUD" => {
                        let idx = self.next_counter_idx;
                        self.next_counter_idx += 1;
                        let q_slot = self.variables.len();
                        self.variables
                            .push((format!("{}.Q", var.name), PlcValue::Bool(false)));
                        let cv_slot = self.variables.len();
                        self.variables
                            .push((format!("{}.CV", var.name), PlcValue::Dint(0)));
                        self.counters
                            .insert(var.name.clone(), (idx, q_slot, cv_slot));
                        self.counter_kinds
                            .insert(var.name.clone(), type_name.clone());
                    }
                    "R_TRIG" | "F_TRIG" => {
                        let idx = self.next_edge_idx;
                        self.next_edge_idx += 1;
                        let q_slot = self.variables.len();
                        self.variables
                            .push((format!("{}.Q", var.name), PlcValue::Bool(false)));
                        let is_rising = type_name == "R_TRIG";
                        self.edges
                            .insert(var.name.clone(), (idx, q_slot, is_rising));
                    }
                    _ => {
                        let default_value = match &var.initial {
                            Some(expr) => {
                                let val = eval_const_expr(expr);
                                coerce_to_type(val, &var.var_type)
                            }
                            None => type_spec_to_default_with_defs(
                                &var.var_type,
                                &self.struct_defs,
                                &self.enum_defs,
                            ),
                        };
                        let slot = self.variables.len();
                        self.var_map.insert(var.name.clone(), slot);
                        self.variables.push((var.name.clone(), default_value));
                    }
                }
            }
        }

        for stmt in &pou.body {
            self.current_line = self.byte_offset_to_line(stmt.span.start);
            self.compile_stmt(stmt);
        }

        self.emit(Opcode::Halt, None);

        if self.errors.is_empty() {
            Ok(CompiledModule {
                name: pou.name.clone(),
                instructions: self.instructions,
                variable_slots: self.variables,
                pou_table: vec![(pou.name.clone(), 0)],
                name_constants: self.name_table,
                line_map: self.line_map,
            })
        } else {
            Err(self.errors)
        }
    }

    /// Register user-defined types (STRUCT and ENUM definitions).
    pub fn register_types(&mut self, type_decls: &[iecst::TypeDecl]) {
        for td in type_decls {
            match &td.definition {
                iecst::TypeDef::Struct { fields } => {
                    let field_specs: Vec<(String, iecst::TypeSpec)> = fields
                        .iter()
                        .map(|f| (f.name.clone(), f.var_type.clone()))
                        .collect();
                    self.struct_defs.insert(td.name.to_uppercase(), field_specs);
                }
                iecst::TypeDef::Enum { values } => {
                    let names: Vec<String> = values.iter().map(|v| v.name.clone()).collect();
                    self.enum_defs.insert(td.name.to_uppercase(), names);
                }
                _ => {}
            }
        }
    }

    fn emit(&mut self, opcode: Opcode, operand: Option<i32>) -> usize {
        let pos = self.instructions.len();
        self.instructions.push(Instruction { opcode, operand });
        self.line_map.push((pos, self.current_line));
        pos
    }

    fn var_slot(&self, name: &str) -> Option<usize> {
        self.var_map.get(name).copied()
    }

    fn compile_stmt(&mut self, stmt: &iecst::Stmt) {
        match &stmt.kind {
            iecst::StmtKind::Assignment { target, value } => {
                // Handle array index assignment: arr[i] := val
                if let iecst::ExprKind::ArrayIndex { array, indices } = &target.kind {
                    self.compile_expr(array);
                    if let Some(index) = indices.first() {
                        self.compile_expr(index);
                    } else {
                        self.emit(Opcode::PushDint, Some(0));
                    }
                    self.compile_expr(value);
                    self.emit(Opcode::StoreArrayIndex, None);
                } else if let iecst::ExprKind::MemberAccess {
                    expr: object,
                    member,
                } = &target.kind
                {
                    // Handle struct field assignment: struct.field := val
                    if let iecst::ExprKind::Ident(name) = &object.kind {
                        if let Some(&var_slot) = self.var_map.get(name) {
                            if let Some(field_idx) = self.find_struct_field_index(var_slot, member)
                            {
                                self.compile_expr(value);
                                self.emit(Opcode::LoadVar, Some(var_slot as i32));
                                self.emit(Opcode::StoreStructField, Some(field_idx as i32));
                                self.emit(Opcode::StoreVar, Some(var_slot as i32));
                            }
                        }
                    }
                } else {
                    self.compile_expr(value);
                    if let Some(slot) = self.resolve_target(target) {
                        if slot == usize::MAX {
                            // Array index not handled above? fallback
                        } else {
                            self.emit(Opcode::StoreVar, Some(slot as i32));
                        }
                    } else {
                        self.push_error(
                            format!(
                                "Unknown variable '{}' in assignment",
                                get_target_name(target)
                            ),
                            &target.span,
                        );
                    }
                }
            }

            iecst::StmtKind::If {
                condition,
                then_body,
                elsif_branches,
                else_body,
            } => {
                self.compile_expr(condition);
                let jump_false_pos = self.emit(Opcode::JumpIfFalse, None);

                for s in then_body {
                    self.compile_stmt(s);
                }

                let has_elsif_or_else = !elsif_branches.is_empty() || else_body.is_some();
                let mut exit_jumps: Vec<usize> = Vec::new();

                if has_elsif_or_else {
                    exit_jumps.push(self.emit(Opcode::Jump, None));
                }

                let after_then = self.instructions.len();
                if let Some(instr) = self.instructions.get_mut(jump_false_pos) {
                    instr.operand = Some((after_then as i32) - (jump_false_pos as i32));
                }

                for (cond, body) in elsif_branches.iter() {
                    self.compile_expr(cond);
                    let jf_pos = self.emit(Opcode::JumpIfFalse, None);
                    for s in body {
                        self.compile_stmt(s);
                    }
                    exit_jumps.push(self.emit(Opcode::Jump, None));
                    let after_elsif = self.instructions.len();
                    if let Some(instr) = self.instructions.get_mut(jf_pos) {
                        instr.operand = Some((after_elsif as i32) - (jf_pos as i32));
                    }
                }

                if let Some(else_stmts) = else_body {
                    for s in else_stmts {
                        self.compile_stmt(s);
                    }
                }

                let end_pos = self.instructions.len();
                for pos in exit_jumps {
                    if let Some(instr) = self.instructions.get_mut(pos) {
                        instr.operand = Some((end_pos as i32) - (pos as i32));
                    }
                }
            }

            iecst::StmtKind::Case {
                expr,
                cases,
                else_body,
            } => {
                // Evaluate selector and store in a hidden temp variable
                let temp_slot = self.variables.len();
                self.variables.push(("_case_sel".into(), PlcValue::Dint(0)));
                self.compile_expr(expr);
                self.emit(Opcode::StoreVar, Some(temp_slot as i32));

                let mut exit_jumps: Vec<usize> = Vec::new();

                for case in cases {
                    let mut skip_to_next_jumps: Vec<usize> = Vec::new();
                    for value in &case.values {
                        match value {
                            iecst::CaseValue::Single(v) => {
                                self.emit(Opcode::LoadVar, Some(temp_slot as i32));
                                self.compile_expr(v);
                                self.emit(Opcode::Eq, None);
                                // If not equal, skip to next value/case
                                let jf = self.emit(Opcode::JumpIfFalse, None);
                                skip_to_next_jumps.push(jf);
                            }
                            iecst::CaseValue::Range { from, to } => {
                                // selector >= from AND selector <= to
                                self.emit(Opcode::LoadVar, Some(temp_slot as i32));
                                self.compile_expr(from);
                                self.emit(Opcode::Ge, None);
                                let ge_jf = self.emit(Opcode::JumpIfFalse, None);
                                skip_to_next_jumps.push(ge_jf);
                                self.emit(Opcode::LoadVar, Some(temp_slot as i32));
                                self.compile_expr(to);
                                self.emit(Opcode::Le, None);
                                let le_jf = self.emit(Opcode::JumpIfFalse, None);
                                skip_to_next_jumps.push(le_jf);
                            }
                        }
                    }

                    // Case body
                    for s in &case.body {
                        self.compile_stmt(s);
                    }
                    exit_jumps.push(self.emit(Opcode::Jump, None));

                    // All "not matched" jumps land here (start of next case)
                    let next_case = self.instructions.len();
                    for pos in skip_to_next_jumps {
                        if let Some(instr) = self.instructions.get_mut(pos) {
                            instr.operand = Some((next_case as i32) - (pos as i32));
                        }
                    }
                }

                if let Some(else_stmts) = else_body {
                    for s in else_stmts {
                        self.compile_stmt(s);
                    }
                }

                let end_pos = self.instructions.len();
                for pos in exit_jumps {
                    if let Some(instr) = self.instructions.get_mut(pos) {
                        instr.operand = Some((end_pos as i32) - (pos as i32));
                    }
                }
            }

            iecst::StmtKind::For {
                var,
                from,
                to,
                by,
                body,
            } => {
                self.compile_expr(from);
                if let Some(slot) = self.var_slot(var) {
                    self.emit(Opcode::StoreVar, Some(slot as i32));
                }

                let check_pos = self.instructions.len();
                if let Some(slot) = self.var_slot(var) {
                    self.emit(Opcode::LoadVar, Some(slot as i32));
                }
                self.compile_expr(to);
                self.emit(Opcode::Le, None);
                let jf_pos = self.emit(Opcode::JumpIfFalse, None);

                self.loop_stack.push((Vec::new(), Vec::new()));
                for s in body {
                    self.compile_stmt(s);
                }
                let (mut exit_jumps, continue_jumps) = self.loop_stack.pop().unwrap();

                // Continue target: increment step
                if let Some(slot) = self.var_slot(var) {
                    self.emit(Opcode::LoadVar, Some(slot as i32));
                    if let Some(by_expr) = by {
                        self.compile_expr(by_expr);
                    } else {
                        self.emit(Opcode::PushInt, Some(1));
                    }
                    self.emit(Opcode::Add, None);
                    self.emit(Opcode::StoreVar, Some(slot as i32));
                }

                let jump_back = self.instructions.len();
                self.emit(Opcode::Jump, Some((check_pos as i32) - (jump_back as i32)));

                let end_pos = self.instructions.len();
                if let Some(instr) = self.instructions.get_mut(jf_pos) {
                    instr.operand = Some((end_pos as i32) - (jf_pos as i32));
                }
                for pos in exit_jumps {
                    if let Some(instr) = self.instructions.get_mut(pos) {
                        instr.operand = Some((end_pos as i32) - (pos as i32));
                    }
                }
            }

            iecst::StmtKind::While { condition, body } => {
                let check_pos = self.instructions.len();
                self.compile_expr(condition);
                let jf_pos = self.emit(Opcode::JumpIfFalse, None);

                self.loop_stack.push((Vec::new(), Vec::new()));
                for s in body {
                    self.compile_stmt(s);
                }
                let (mut exit_jumps, continue_jumps) = self.loop_stack.pop().unwrap();

                let jump_back = self.instructions.len();
                self.emit(Opcode::Jump, Some((check_pos as i32) - (jump_back as i32)));

                let end_pos = self.instructions.len();
                if let Some(instr) = self.instructions.get_mut(jf_pos) {
                    instr.operand = Some((end_pos as i32) - (jf_pos as i32));
                }
                for pos in exit_jumps {
                    if let Some(instr) = self.instructions.get_mut(pos) {
                        instr.operand = Some((end_pos as i32) - (pos as i32));
                    }
                }
                // Continue jumps go to check_pos
                for pos in continue_jumps {
                    if let Some(instr) = self.instructions.get_mut(pos) {
                        instr.operand = Some((check_pos as i32) - (pos as i32));
                    }
                }
            }

            iecst::StmtKind::Repeat { body, until } => {
                let body_start = self.instructions.len();

                self.loop_stack.push((Vec::new(), Vec::new()));
                for s in body {
                    self.compile_stmt(s);
                }
                let (mut exit_jumps, continue_jumps) = self.loop_stack.pop().unwrap();

                // Continue target: UNTIL condition check
                let until_pos = self.instructions.len();
                self.compile_expr(until);
                let jf_pos = self.emit(Opcode::JumpIfFalse, None);
                let jump_back = self.instructions.len();
                self.emit(Opcode::Jump, Some((body_start as i32) - (jump_back as i32)));

                let end_pos = self.instructions.len();
                if let Some(instr) = self.instructions.get_mut(jf_pos) {
                    instr.operand = Some((end_pos as i32) - (jf_pos as i32));
                }
                for pos in exit_jumps {
                    if let Some(instr) = self.instructions.get_mut(pos) {
                        instr.operand = Some((end_pos as i32) - (pos as i32));
                    }
                }
                for pos in continue_jumps {
                    if let Some(instr) = self.instructions.get_mut(pos) {
                        instr.operand = Some((until_pos as i32) - (pos as i32));
                    }
                }
            }

            iecst::StmtKind::Call { name, args } => {
                if let Some(&(idx, q_slot, et_or_cv_slot)) = self.timers.get(name) {
                    // Timer call: push PT, push IN, emit timer opcode, store Q and ET
                    let pt_val = args
                        .iter()
                        .find(|a| a.name.as_deref() == Some("PT"))
                        .and_then(|a| a.value.as_ref());
                    let in_val = args
                        .iter()
                        .find(|a| a.name.as_deref() == Some("IN"))
                        .and_then(|a| a.value.as_ref());

                    if let (Some(pt), Some(in_)) = (pt_val, in_val) {
                        self.compile_expr(pt);
                        self.compile_expr(in_);
                    } else {
                        // Positional: args[0]=IN, args[1]=PT
                        if let Some(arg) = args.get(1) {
                            if let Some(ref v) = arg.value {
                                self.compile_expr(v);
                            } else {
                                self.emit(Opcode::PushTime, Some(0));
                            }
                        } else {
                            self.emit(Opcode::PushTime, Some(0));
                        }
                        if let Some(arg) = args.get(0) {
                            if let Some(ref v) = arg.value {
                                self.compile_expr(v);
                            } else {
                                self.emit(Opcode::PushBool, Some(0));
                            }
                        } else {
                            self.emit(Opcode::PushBool, Some(0));
                        }
                    }

                    let opcode = match self.timer_type(name) {
                        "TOF" => Opcode::TofCall,
                        "TP" => Opcode::TpCall,
                        _ => Opcode::TonCall,
                    };
                    self.emit(opcode, Some(idx as i32));
                    // Store results: stack has [ET, Q]
                    self.emit(Opcode::StoreVar, Some(q_slot as i32)); // Q
                    self.emit(Opcode::StoreVar, Some(et_or_cv_slot as i32)); // ET
                } else if let Some(&(idx, q_slot, cv_slot)) = self.counters.get(name) {
                    // Counter call
                    let ctype = self.counter_type(name);

                    if ctype == "CTUD" {
                        // CTUD: push PV, LD, CD, R, CU (5 operands)
                        let cu_val = args
                            .iter()
                            .find(|a| a.name.as_deref() == Some("CU"))
                            .and_then(|a| a.value.as_ref());
                        let r_val = args
                            .iter()
                            .find(|a| a.name.as_deref() == Some("R"))
                            .and_then(|a| a.value.as_ref());
                        let cd_val = args
                            .iter()
                            .find(|a| a.name.as_deref() == Some("CD"))
                            .and_then(|a| a.value.as_ref());
                        let ld_val = args
                            .iter()
                            .find(|a| a.name.as_deref() == Some("LD"))
                            .and_then(|a| a.value.as_ref());
                        let pv_val = args
                            .iter()
                            .find(|a| a.name.as_deref() == Some("PV"))
                            .and_then(|a| a.value.as_ref());

                        if let Some(pv) = pv_val {
                            self.compile_expr(pv);
                        } else {
                            self.emit(Opcode::PushDint, Some(0));
                        }
                        if let Some(ld) = ld_val {
                            self.compile_expr(ld);
                        } else {
                            self.emit(Opcode::PushBool, Some(0));
                        }
                        if let Some(cd) = cd_val {
                            self.compile_expr(cd);
                        } else {
                            self.emit(Opcode::PushBool, Some(0));
                        }
                        if let Some(r) = r_val {
                            self.compile_expr(r);
                        } else {
                            self.emit(Opcode::PushBool, Some(0));
                        }
                        if let Some(cu) = cu_val {
                            self.compile_expr(cu);
                        } else {
                            self.emit(Opcode::PushBool, Some(0));
                        }

                        self.emit(Opcode::CtudCall, Some(idx as i32));
                    } else if ctype == "CTD" {
                        // CTD: push PV, LD, CD
                        let cd_val = args
                            .iter()
                            .find(|a| a.name.as_deref() == Some("CD"))
                            .and_then(|a| a.value.as_ref());
                        let ld_val = args
                            .iter()
                            .find(|a| a.name.as_deref() == Some("LD"))
                            .and_then(|a| a.value.as_ref());
                        let pv_val = args
                            .iter()
                            .find(|a| a.name.as_deref() == Some("PV"))
                            .and_then(|a| a.value.as_ref());

                        if let Some(pv) = pv_val {
                            self.compile_expr(pv);
                        } else {
                            self.emit(Opcode::PushDint, Some(0));
                        }
                        if let Some(ld) = ld_val {
                            self.compile_expr(ld);
                        } else {
                            self.emit(Opcode::PushBool, Some(0));
                        }
                        if let Some(cd) = cd_val {
                            self.compile_expr(cd);
                        } else {
                            self.emit(Opcode::PushBool, Some(0));
                        }

                        self.emit(Opcode::CtdCall, Some(idx as i32));
                    } else {
                        // CTU: push PV, R, CU
                        let cu_val = args
                            .iter()
                            .find(|a| a.name.as_deref() == Some("CU"))
                            .and_then(|a| a.value.as_ref());
                        let r_val = args
                            .iter()
                            .find(|a| a.name.as_deref() == Some("R"))
                            .and_then(|a| a.value.as_ref());
                        let pv_val = args
                            .iter()
                            .find(|a| a.name.as_deref() == Some("PV"))
                            .and_then(|a| a.value.as_ref());

                        if let Some(pv) = pv_val {
                            self.compile_expr(pv);
                        } else {
                            self.emit(Opcode::PushDint, Some(0));
                        }
                        if let Some(r) = r_val {
                            self.compile_expr(r);
                        } else {
                            self.emit(Opcode::PushBool, Some(0));
                        }
                        if let Some(cu) = cu_val {
                            self.compile_expr(cu);
                        } else {
                            self.emit(Opcode::PushBool, Some(0));
                        }

                        self.emit(Opcode::CtuCall, Some(idx as i32));
                    }
                    // Store results: stack has [CV, Q]
                    self.emit(Opcode::StoreVar, Some(q_slot as i32)); // Q
                    self.emit(Opcode::StoreVar, Some(cv_slot as i32)); // CV
                } else if let Some(&(idx, q_slot, is_rising)) = self.edges.get(name) {
                    // Edge detection: push CLK, emit edge opcode, store Q
                    let clk_val = args
                        .iter()
                        .find(|a| a.name.as_deref() == Some("CLK"))
                        .and_then(|a| a.value.as_ref());
                    if let Some(clk) = clk_val {
                        self.compile_expr(clk);
                    } else if let Some(arg) = args.get(0) {
                        if let Some(ref v) = arg.value {
                            self.compile_expr(v);
                        } else {
                            self.emit(Opcode::PushBool, Some(0));
                        }
                    } else {
                        self.emit(Opcode::PushBool, Some(0));
                    }

                    let opcode = if is_rising {
                        Opcode::RTrigCall
                    } else {
                        Opcode::FTrigCall
                    };
                    self.emit(opcode, Some(idx as i32));
                    self.emit(Opcode::StoreVar, Some(q_slot as i32));
                } else {
                    // POU call (user-defined FUNCTION or FUNCTION_BLOCK)
                    for arg in args {
                        if let Some(ref value) = arg.value {
                            self.compile_expr(value);
                        }
                    }
                    let name_idx = self.intern_name(name);
                    self.emit(Opcode::CallPou, Some(name_idx as i32));
                }
            }

            iecst::StmtKind::Return { value } => {
                if let Some(expr) = value {
                    self.compile_expr(expr);
                }
                self.emit(Opcode::Return, None);
            }

            iecst::StmtKind::Exit => {
                if !self.loop_stack.is_empty() {
                    let pos = self.emit(Opcode::Jump, None);
                    self.loop_stack.last_mut().unwrap().0.push(pos);
                }
            }

            iecst::StmtKind::Continue => {
                if !self.loop_stack.is_empty() {
                    let pos = self.emit(Opcode::Jump, None);
                    self.loop_stack.last_mut().unwrap().1.push(pos);
                }
            }

            iecst::StmtKind::Empty => {}
        }
    }

    fn compile_expr(&mut self, expr: &iecst::Expr) {
        match &expr.kind {
            iecst::ExprKind::IntLiteral(val) => {
                self.emit(Opcode::PushDint, Some(*val as i32));
            }
            iecst::ExprKind::RealLiteral(val) => {
                self.emit(Opcode::PushReal, Some((*val as f32).to_bits() as i32));
            }
            iecst::ExprKind::BoolLiteral(val) => {
                self.emit(Opcode::PushBool, Some(if *val { 1 } else { 0 }));
            }
            iecst::ExprKind::StringLiteral(s) | iecst::ExprKind::WStringLiteral(s) => {
                let trimmed = s.trim_start_matches('\'').trim_end_matches('\'');
                let idx = self.intern_name(trimmed);
                self.emit(Opcode::PushString, Some(idx as i32));
            }
            iecst::ExprKind::TimeLiteral(val) => {
                let ms = parse_time_literal(val);
                self.emit(Opcode::PushTime, Some(ms));
            }

            iecst::ExprKind::Ident(name) => {
                if let Some(slot) = self.var_slot(name) {
                    self.emit(Opcode::LoadVar, Some(slot as i32));
                } else {
                    self.push_error(format!("Unknown variable '{}'", name), &expr.span);
                    self.emit(Opcode::PushDint, Some(0));
                }
            }

            iecst::ExprKind::DirectAddress(addr) => {
                let encoded = encode_io_address(addr);
                self.emit(Opcode::LoadIo, Some(encoded));
            }

            iecst::ExprKind::BinaryOp { left, op, right } => {
                self.compile_expr(left);
                self.compile_expr(right);
                self.emit(binary_op_to_opcode(op), None);
            }

            iecst::ExprKind::UnaryOp { op, expr } => {
                self.compile_expr(expr);
                match op {
                    iecst::UnaryOp::Neg => {
                        self.emit(Opcode::Neg, None);
                    }
                    iecst::UnaryOp::Not => {
                        self.emit(Opcode::Not, None);
                    }
                }
            }

            iecst::ExprKind::FunctionCall { name, args } => {
                for arg in args {
                    if let Some(ref value) = arg.value {
                        self.compile_expr(value);
                    }
                }
                if let Some(opcode) = stdlib_opcode(name) {
                    self.emit(opcode, Some(args.len() as i32));
                } else {
                    self.push_error(format!("Unknown function '{}'", name), &expr.span);
                    self.emit(Opcode::PushDint, Some(0));
                }
            }

            iecst::ExprKind::Paren(inner) => {
                self.compile_expr(inner);
            }

            iecst::ExprKind::ArrayIndex { array, indices } => {
                self.compile_expr(array);
                if let Some(index) = indices.first() {
                    self.compile_expr(index);
                } else {
                    self.emit(Opcode::PushDint, Some(0));
                }
                self.emit(Opcode::LoadArrayIndex, None);
            }

            iecst::ExprKind::MemberAccess {
                expr: object,
                member,
            } => {
                if let iecst::ExprKind::Ident(name) = &object.kind {
                    if let Some(&(_idx, q_slot, _second_slot)) = self.timers.get(name) {
                        let slot = match member.as_str() {
                            "Q" => q_slot,
                            "ET" => self.timers[name].2,
                            _ => q_slot,
                        };
                        self.emit(Opcode::LoadVar, Some(slot as i32));
                    } else if let Some(&(_idx, q_slot, _second_slot)) = self.counters.get(name) {
                        let slot = match member.as_str() {
                            "Q" => q_slot,
                            "CV" => self.counters[name].2,
                            _ => q_slot,
                        };
                        self.emit(Opcode::LoadVar, Some(slot as i32));
                    } else if let Some(&(_idx, q_slot, _is_rising)) = self.edges.get(name) {
                        if member == "Q" {
                            self.emit(Opcode::LoadVar, Some(q_slot as i32));
                        } else {
                            self.emit(Opcode::PushDint, Some(0));
                        }
                    } else if self.enum_defs.contains_key(&name.to_uppercase()) {
                        // Enum value access: TypeName.VALUE -> push as Enum string
                        let idx = self.intern_name(member);
                        self.emit(Opcode::PushEnum, Some(idx as i32));
                    } else if let Some(&var_slot) = self.var_map.get(name) {
                        // Struct field access: load struct, then load field by index
                        let field_idx = self.find_struct_field_index(var_slot, member);
                        if let Some(idx) = field_idx {
                            self.emit(Opcode::LoadVar, Some(var_slot as i32));
                            self.emit(Opcode::LoadStructField, Some(idx as i32));
                        } else {
                            self.push_error(
                                format!("Unknown field '{}' on variable '{}'", member, name),
                                &expr.span,
                            );
                            self.emit(Opcode::PushDint, Some(0));
                        }
                    } else {
                        self.push_error(format!("Unknown variable '{}'", name), &expr.span);
                        self.emit(Opcode::PushDint, Some(0));
                    }
                } else {
                    self.push_error("Invalid member access target".into(), &expr.span);
                    self.emit(Opcode::PushDint, Some(0));
                }
            }

            _ => {
                self.push_error("Unsupported expression".into(), &expr.span);
                self.emit(Opcode::PushDint, Some(0));
            }
        }
    }

    fn resolve_target(&self, expr: &iecst::Expr) -> Option<usize> {
        match &expr.kind {
            iecst::ExprKind::Ident(name) => self.var_slot(name),
            iecst::ExprKind::DirectAddress(addr) => Some(encode_io_address(addr) as usize),
            iecst::ExprKind::ArrayIndex { .. } => {
                // Special sentinel: emit StoreArrayIndex
                Some(usize::MAX)
            }
            iecst::ExprKind::MemberAccess {
                expr: object,
                member,
            } => {
                if let iecst::ExprKind::Ident(name) = &object.kind {
                    if let Some(&(_idx, q_slot, _second)) = self.timers.get(name) {
                        return Some(match member.as_str() {
                            "Q" => q_slot,
                            "ET" => self.timers[name].2,
                            _ => q_slot,
                        });
                    }
                    if let Some(&(_idx, q_slot, _second)) = self.counters.get(name) {
                        return Some(match member.as_str() {
                            "Q" => q_slot,
                            "CV" => self.counters[name].2,
                            _ => q_slot,
                        });
                    }
                    if let Some(&(_idx, q_slot, _is_rising)) = self.edges.get(name) {
                        if member == "Q" {
                            return Some(q_slot);
                        }
                    }
                    // Struct field store: use sentinel for StoreStructField path
                    if let Some(&var_slot) = self.var_map.get(name) {
                        if self.find_struct_field_index(var_slot, member).is_some() {
                            return Some(usize::MAX - 1); // sentinel for struct field store
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn timer_type(&self, name: &str) -> &str {
        self.timer_kinds
            .get(name)
            .map(|s| s.as_str())
            .unwrap_or("TON")
    }

    fn counter_type(&self, name: &str) -> &str {
        self.counter_kinds
            .get(name)
            .map(|s| s.as_str())
            .unwrap_or("CTU")
    }

    /// Find the field index within a struct variable by looking up the type definition.
    fn find_struct_field_index(&self, var_slot: usize, field_name: &str) -> Option<usize> {
        let (_, value) = self.variables.get(var_slot)?;
        if let PlcValue::Struct(fields) = value {
            fields.iter().position(|(n, _)| n == field_name)
        } else {
            None
        }
    }
}

fn binary_op_to_opcode(op: &iecst::BinaryOp) -> Opcode {
    match op {
        iecst::BinaryOp::Add => Opcode::Add,
        iecst::BinaryOp::Sub => Opcode::Sub,
        iecst::BinaryOp::Mul => Opcode::Mul,
        iecst::BinaryOp::Div => Opcode::Div,
        iecst::BinaryOp::Mod => Opcode::Modulo,
        iecst::BinaryOp::Power => Opcode::Pow,
        iecst::BinaryOp::Eq => Opcode::Eq,
        iecst::BinaryOp::Ne => Opcode::Ne,
        iecst::BinaryOp::Lt => Opcode::Lt,
        iecst::BinaryOp::Le => Opcode::Le,
        iecst::BinaryOp::Gt => Opcode::Gt,
        iecst::BinaryOp::Ge => Opcode::Ge,
        iecst::BinaryOp::And => Opcode::And,
        iecst::BinaryOp::Or => Opcode::Or,
        iecst::BinaryOp::Xor => Opcode::Xor,
    }
}

fn type_spec_to_default_with_defs(
    ts: &iecst::TypeSpec,
    struct_defs: &HashMap<String, Vec<(String, iecst::TypeSpec)>>,
    enum_defs: &HashMap<String, Vec<String>>,
) -> PlcValue {
    match &ts.kind {
        iecst::TypeKind::Simple(name) => {
            let upper = name.to_uppercase();
            match upper.as_str() {
                "BOOL" => PlcValue::Bool(false),
                "BYTE" => PlcValue::Byte(0),
                "SINT" | "INT" => PlcValue::Int(0),
                "DINT" | "LINT" | "UDINT" | "ULINT" => PlcValue::Dint(0),
                "USINT" | "UINT" => PlcValue::Int(0),
                "REAL" | "LREAL" => PlcValue::Real(0.0),
                "TIME" => PlcValue::Time(0),
                "STRING" | "WSTRING" => PlcValue::String(String::new()),
                "WORD" | "DWORD" | "LWORD" => PlcValue::Word(0),
                _ => {
                    // Check user-defined struct
                    if let Some(fields) = struct_defs.get(&upper) {
                        let field_values: Vec<(String, PlcValue)> = fields
                            .iter()
                            .map(|(fname, ftype)| {
                                (
                                    fname.clone(),
                                    type_spec_to_default_with_defs(ftype, struct_defs, enum_defs),
                                )
                            })
                            .collect();
                        return PlcValue::Struct(field_values);
                    }
                    // Check user-defined enum
                    if let Some(values) = enum_defs.get(&upper) {
                        return PlcValue::Enum(values.first().cloned().unwrap_or_default());
                    }
                    PlcValue::Dint(0)
                }
            }
        }
        iecst::TypeKind::String { .. } => PlcValue::String(String::new()),
        iecst::TypeKind::WString { .. } => PlcValue::String(String::new()),
        iecst::TypeKind::Array { ranges, element } => {
            let len = array_total_length(ranges);
            let elem_default = type_spec_to_default_with_defs(element, struct_defs, enum_defs);
            PlcValue::Array(vec![elem_default; len as usize])
        }
        _ => PlcValue::Dint(0),
    }
}

fn array_total_length(ranges: &[iecst::ArrayRange]) -> i32 {
    ranges.iter().fold(1, |acc, range| {
        let low = eval_as_int(&range.low);
        let high = eval_as_int(&range.high);
        let dim = (high - low + 1).max(0);
        acc * dim
    })
}

fn eval_as_int(expr: &iecst::Expr) -> i32 {
    match &expr.kind {
        iecst::ExprKind::IntLiteral(v) => *v as i32,
        _ => 0,
    }
}

fn parse_time_literal(s: &str) -> i32 {
    let s = s.trim();
    let mut total_ms: i32 = 0;
    let mut num_buf = String::new();

    for ch in s.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            num_buf.push(ch);
        } else {
            if !num_buf.is_empty() {
                if let Ok(val) = num_buf.parse::<f64>() {
                    match ch {
                        'd' | 'D' => total_ms += (val * 86400000.0) as i32,
                        'h' | 'H' => total_ms += (val * 3600000.0) as i32,
                        'm' | 'M' => total_ms += (val * 60000.0) as i32,
                        's' | 'S' => total_ms += (val * 1000.0) as i32,
                        _ => total_ms += val as i32,
                    }
                }
                num_buf.clear();
            }
        }
    }

    if !num_buf.is_empty() {
        if let Ok(val) = num_buf.parse::<i32>() {
            total_ms += val;
        }
    }

    total_ms
}

fn parse_address_index(addr: &str) -> usize {
    let parts: Vec<&str> = addr.split('.').collect();
    if parts.len() == 2 {
        if let (Ok(byte_offset), Ok(bit_offset)) =
            (parts[0].parse::<usize>(), parts[1].parse::<usize>())
        {
            return byte_offset * 8 + bit_offset;
        }
    }
    addr.parse::<usize>().unwrap_or(0)
}

/// Encode IEC 61131-3 direct address into a packed i32 operand.
/// Bits 31..28: area (1=Input, 2=Output, 3=Memory)
/// Bits 27..24: size (1=Bit, 2=Byte, 3=Word, 4=DWord)
/// Bits 23..0:  flat address index
fn encode_io_address(addr: &iecst::DirectAddress) -> i32 {
    let area: i32 = match addr.location {
        iecst::LocationPrefix::Input => 1,
        iecst::LocationPrefix::Output => 2,
        iecst::LocationPrefix::Memory => 3,
    };
    let size: i32 = match addr.size {
        iecst::SizePrefix::Bit => 1,
        iecst::SizePrefix::Byte => 2,
        iecst::SizePrefix::Word => 3,
        iecst::SizePrefix::DoubleWord => 4,
        iecst::SizePrefix::LongWord => 5,
    };
    let idx = parse_address_index(&addr.address) as i32;
    (area << 28) | (size << 24) | (idx & 0x00FF_FFFF)
}

fn eval_const_expr(expr: &iecst::Expr) -> PlcValue {
    match &expr.kind {
        iecst::ExprKind::IntLiteral(val) => PlcValue::Dint(*val as i32),
        iecst::ExprKind::RealLiteral(val) => PlcValue::Real(*val as f32),
        iecst::ExprKind::BoolLiteral(val) => PlcValue::Bool(*val),
        iecst::ExprKind::TimeLiteral(val) => PlcValue::Time(parse_time_literal(val)),
        iecst::ExprKind::UnaryOp { op, expr: inner } => match op {
            iecst::UnaryOp::Neg => {
                let v = eval_const_expr(inner);
                match v {
                    PlcValue::Int(i) => PlcValue::Int(-i),
                    PlcValue::Dint(i) => PlcValue::Dint(-i),
                    PlcValue::Real(f) => PlcValue::Real(-f),
                    _ => v,
                }
            }
            iecst::UnaryOp::Not => {
                let v = eval_const_expr(inner);
                PlcValue::Bool(!v.to_bool())
            }
        },
        _ => PlcValue::Dint(0),
    }
}

fn get_target_name(expr: &iecst::Expr) -> String {
    match &expr.kind {
        iecst::ExprKind::Ident(name) => name.clone(),
        iecst::ExprKind::MemberAccess { expr: _, member } => member.clone(),
        _ => "?".into(),
    }
}

fn stdlib_opcode(name: &str) -> Option<Opcode> {
    match name.to_uppercase().as_str() {
        "SQRT" => Some(Opcode::Sqrt),
        "LN" => Some(Opcode::Ln),
        "LOG" => Some(Opcode::Log),
        "EXP" => Some(Opcode::Exp),
        "SIN" => Some(Opcode::Sin),
        "COS" => Some(Opcode::Cos),
        "TAN" => Some(Opcode::Tan),
        "ASIN" => Some(Opcode::Asin),
        "ACOS" => Some(Opcode::Acos),
        "ATAN" => Some(Opcode::Atan),
        "ABS" => Some(Opcode::Abs),
        "TRUNC" => Some(Opcode::Trunc),
        "FLOOR" => Some(Opcode::Floor),
        "CEIL" => Some(Opcode::Ceil),
        "EXPT" => Some(Opcode::Expt),
        "LEN" => Some(Opcode::StrLen),
        "LEFT" => Some(Opcode::StrLeft),
        "RIGHT" => Some(Opcode::StrRight),
        "MID" => Some(Opcode::StrMid),
        "CONCAT" => Some(Opcode::Concat),
        "INSERT" => Some(Opcode::StrInsert),
        "DELETE" => Some(Opcode::StrDelete),
        "REPLACE" => Some(Opcode::StrReplace),
        "FIND" => Some(Opcode::StrFind),
        "INT_TO_REAL" | "INT2REAL" => Some(Opcode::IntToReal),
        "REAL_TO_INT" | "REAL2INT" => Some(Opcode::RealToInt),
        "INT_TO_BOOL" | "INT2BOOL" => Some(Opcode::IntToBool),
        "BOOL_TO_INT" | "BOOL2INT" => Some(Opcode::BoolToInt),
        "INT_TO_STRING" | "INT2STRING" => Some(Opcode::IntToString),
        "REAL_TO_STRING" | "REAL2STRING" => Some(Opcode::RealToString),
        "STRING_TO_INT" | "STRING2INT" => Some(Opcode::StringToInt),
        "STRING_TO_REAL" | "STRING2REAL" => Some(Opcode::StringToReal),
        "WORD_TO_INT" | "WORD2INT" => Some(Opcode::WordToInt),
        "DWORD_TO_INT" | "DWORD2INT" => Some(Opcode::DwordToInt),
        "INT_TO_WORD" | "INT2WORD" => Some(Opcode::IntToWord),
        "INT_TO_DWORD" | "INT2DWORD" => Some(Opcode::IntToDword),
        "DWORD_TO_REAL" | "DWORD2REAL" => Some(Opcode::DwordToReal),
        "REAL_TO_DWORD" | "REAL2DWORD" => Some(Opcode::RealToDword),
        "SEL" => Some(Opcode::Sel),
        "MUX" => Some(Opcode::Mux),
        "LIMIT" => Some(Opcode::Limit),
        "MAX" => Some(Opcode::Max),
        "MIN" => Some(Opcode::Min),
        "MOVE" => Some(Opcode::Move),
        "SHL" => Some(Opcode::Shl),
        "SHR" => Some(Opcode::Shr),
        "ROL" => Some(Opcode::Rol),
        "ROR" => Some(Opcode::Ror),
        _ => None,
    }
}

fn coerce_to_type(val: PlcValue, var_type: &iecst::TypeSpec) -> PlcValue {
    match &var_type.kind {
        iecst::TypeKind::Simple(name) => match name.to_uppercase().as_str() {
            "BOOL" => PlcValue::Bool(val.to_bool()),
            "BYTE" => PlcValue::Byte(val.to_int() as u8),
            "SINT" | "INT" => PlcValue::Int(val.to_int() as i16),
            "DINT" | "LINT" | "UDINT" | "ULINT" => PlcValue::Dint(val.to_int()),
            "USINT" | "UINT" => PlcValue::Int(val.to_int() as i16),
            "REAL" | "LREAL" => PlcValue::Real(val.to_real()),
            "TIME" => PlcValue::Time(val.to_int()),
            "STRING" | "WSTRING" => match val {
                PlcValue::String(s) => PlcValue::String(s),
                _ => PlcValue::String(String::new()),
            },
            "WORD" | "DWORD" | "LWORD" => PlcValue::Word(val.to_int() as u16),
            _ => val,
        },
        _ => val,
    }
}
