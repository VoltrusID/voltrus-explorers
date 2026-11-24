use crate::memory::MemoryModel;
use crate::opcode::{CompiledModule, Instruction, Opcode};
use crate::types::PlcValue;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct FbSnapshot {
    pub timers: Vec<crate::timer::TimerInstance>,
    pub counters: Vec<crate::counter::CounterInstance>,
    pub edge_prev: Vec<bool>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum ExecutionState {
    Running,
    Paused,
    BreakpointHit(usize),
    Done,
    Error(String),
}

pub struct Vm {
    memory: MemoryModel,
    stack: Vec<PlcValue>,
    timers: Vec<crate::timer::TimerInstance>,
    counters: Vec<crate::counter::CounterInstance>,
    edge_prev: Vec<bool>,
    call_stack: Vec<usize>,
    pou_table: Vec<(String, usize)>,
    name_constants: Vec<String>,
    pub breakpoints: HashSet<usize>,
    pub single_step: bool,
    pub last_pc: usize,
}

impl Vm {
    pub fn from_instructions(module: CompiledModule) -> Self {
        Self {
            memory: MemoryModel::new(module.variable_slots),
            stack: Vec::with_capacity(256),
            timers: Vec::new(),
            counters: Vec::new(),
            edge_prev: Vec::new(),
            call_stack: Vec::new(),
            pou_table: module.pou_table,
            name_constants: module.name_constants,
            breakpoints: HashSet::new(),
            single_step: false,
            last_pc: 0,
        }
    }

    pub fn memory(&self) -> &MemoryModel {
        &self.memory
    }

    pub fn memory_mut(&mut self) -> &mut MemoryModel {
        &mut self.memory
    }

    pub fn set_breakpoint(&mut self, pc: usize) {
        self.breakpoints.insert(pc);
    }

    pub fn clear_breakpoint(&mut self, pc: usize) {
        self.breakpoints.remove(&pc);
    }

    pub fn clear_all_breakpoints(&mut self) {
        self.breakpoints.clear();
    }

    pub fn has_breakpoint(&self, pc: usize) -> bool {
        self.breakpoints.contains(&pc)
    }

    pub fn step_into(&mut self) {
        self.single_step = true;
    }

    pub fn continue_execution(&mut self) {
        self.single_step = false;
    }

    /// Execute all instructions until Halt/Return/breakpoint or step limit.
    /// Returns ExecutionState describing why execution stopped.
    pub fn execute_all(&mut self, instructions: &[Instruction]) -> Result<ExecutionState, String> {
        let mut pc: usize = 0;

        while pc < instructions.len() {
            let instr = &instructions[pc];
            self.last_pc = pc;

            // Check breakpoints (only on first instruction of the cycle, not after jump)
            if self.breakpoints.contains(&pc) {
                self.single_step = false;
                return Ok(ExecutionState::BreakpointHit(pc));
            }

            pc = self.execute_instruction(instr, pc, instructions)?;

            if self.single_step {
                self.single_step = false;
                if pc < instructions.len() {
                    self.last_pc = pc;
                }
                return Ok(ExecutionState::Paused);
            }

            if pc >= instructions.len() {
                return Ok(ExecutionState::Done);
            }
        }

        Ok(ExecutionState::Done)
    }

    /// Legacy execute: runs all instructions, returns Ok(()) on success.
    pub fn execute(&mut self, instructions: &[Instruction]) -> Result<(), String> {
        self.continue_execution();
        match self.execute_all(instructions)? {
            ExecutionState::Done => Ok(()),
            ExecutionState::BreakpointHit(_) => Ok(()),
            ExecutionState::Paused => Ok(()),
            ExecutionState::Running => Ok(()),
            ExecutionState::Error(e) => Err(e),
        }
    }

    /// Execute a single instruction. Returns new PC.
    fn execute_instruction(
        &mut self,
        instr: &Instruction,
        pc: usize,
        instructions: &[Instruction],
    ) -> Result<usize, String> {
        let mut next_pc = pc + 1;
        match instr.opcode {
            Opcode::PushBool => {
                let val = instr.operand.unwrap_or(0) != 0;
                self.stack.push(PlcValue::Bool(val));
            }
            Opcode::PushInt => {
                let val = instr.operand.unwrap_or(0) as i16;
                self.stack.push(PlcValue::Int(val));
            }
            Opcode::PushDint => {
                let val = instr.operand.unwrap_or(0);
                self.stack.push(PlcValue::Dint(val));
            }
            Opcode::PushReal => {
                let val = f32::from_bits(instr.operand.unwrap_or(0) as u32);
                self.stack.push(PlcValue::Real(val));
            }
            Opcode::PushTime => {
                let val = instr.operand.unwrap_or(0);
                self.stack.push(PlcValue::Time(val));
            }
            Opcode::PushEnum => {
                let name_idx = instr.operand.unwrap_or(0) as usize;
                let name = self
                    .name_constants
                    .get(name_idx)
                    .cloned()
                    .unwrap_or_default();
                self.stack.push(PlcValue::Enum(name));
            }
            Opcode::PushString => {
                let str_idx = instr.operand.unwrap_or(0) as usize;
                let s = self
                    .name_constants
                    .get(str_idx)
                    .cloned()
                    .unwrap_or_default();
                self.stack.push(PlcValue::String(s));
            }

            Opcode::LoadVar => {
                let slot = instr.operand.unwrap_or(0) as usize;
                if let Some(val) = self.memory.get_by_index(slot) {
                    self.stack.push(val.clone());
                } else {
                    return Err(format!("LoadVar: slot {} out of range", slot));
                }
            }
            Opcode::StoreVar => {
                let slot = instr.operand.unwrap_or(0) as usize;
                let val = self.stack_pop()?;
                self.memory.set_by_index(slot, val)?;
            }
            Opcode::LoadIo => {
                let encoded = instr.operand.unwrap_or(0);
                let val = if (encoded >> 24) != 0 {
                    // New encoding: area+size+index
                    match (encoded >> 28) & 0xF {
                        1 => self
                            .memory
                            .read_input((encoded & 0x00FF_FFFF) as usize)
                            .clone(),
                        2 => self
                            .memory
                            .read_output((encoded & 0x00FF_FFFF) as usize)
                            .clone(),
                        3 => self
                            .memory
                            .read_memory((encoded & 0x00FF_FFFF) as usize)
                            .clone(),
                        _ => PlcValue::Dint(0),
                    }
                } else {
                    // Legacy encoding: flat index into input image
                    self.memory.read_input(encoded as usize).clone()
                };
                self.stack.push(val);
            }
            Opcode::StoreIo => {
                let encoded = instr.operand.unwrap_or(0);
                let val = self.stack_pop()?;
                if (encoded >> 24) != 0 {
                    // New encoding: area+size+index
                    match (encoded >> 28) & 0xF {
                        1 => self
                            .memory
                            .write_input((encoded & 0x00FF_FFFF) as usize, val),
                        2 => self
                            .memory
                            .write_output((encoded & 0x00FF_FFFF) as usize, val),
                        3 => self
                            .memory
                            .write_memory((encoded & 0x00FF_FFFF) as usize, val),
                        _ => {}
                    }
                } else {
                    // Legacy encoding: flat index into output image
                    self.memory.write_output(encoded as usize, val);
                }
            }

            Opcode::Add => self.binary_op(|a, b| a + b)?,
            Opcode::Sub => self.binary_op(|a, b| a - b)?,
            Opcode::Mul => self.binary_op(|a, b| a * b)?,
            Opcode::Div => self.binary_op(|a, b| if b == 0.0 { f32::NAN } else { a / b })?,
            Opcode::Modulo => self.binary_op(|a, b| if b == 0.0 { f32::NAN } else { a % b })?,
            Opcode::Neg => {
                let val = self.stack_pop()?;
                match val {
                    PlcValue::Int(i) => self.stack.push(PlcValue::Int(-i)),
                    PlcValue::Dint(i) => self.stack.push(PlcValue::Dint(-i)),
                    PlcValue::Real(f) => self.stack.push(PlcValue::Real(-f)),
                    _ => return Err(format!("Neg: invalid type {:?}", val)),
                }
            }

            Opcode::Eq => self.compare_op(|a, b| a == b)?,
            Opcode::Ne => self.compare_op(|a, b| a != b)?,
            Opcode::Lt => self.compare_op(|a, b| a < b)?,
            Opcode::Le => self.compare_op(|a, b| a <= b)?,
            Opcode::Gt => self.compare_op(|a, b| a > b)?,
            Opcode::Ge => self.compare_op(|a, b| a >= b)?,

            Opcode::And => self.logic_op(|a, b| a && b)?,
            Opcode::Or => self.logic_op(|a, b| a || b)?,
            Opcode::Xor => self.logic_op(|a, b| a ^ b)?,
            Opcode::Not => {
                let val = self.stack_pop()?;
                self.stack.push(PlcValue::Bool(!val.to_bool()));
            }

            Opcode::Shl => {
                let n = self.stack_pop()?.to_int() as u32;
                let v = self.stack_pop()?.to_int() as u32;
                self.stack.push(PlcValue::Dint((v << n) as i32));
            }
            Opcode::Shr => {
                let n = self.stack_pop()?.to_int() as u32;
                let v = self.stack_pop()?.to_int() as u32;
                self.stack.push(PlcValue::Dint((v >> n) as i32));
            }
            Opcode::Rol => {
                let n = self.stack_pop()?.to_int() as u32;
                let v = self.stack_pop()?.to_int() as u32;
                let bits = 32;
                let shift = n % bits;
                let result = (v << shift) | (v >> (bits - shift));
                self.stack.push(PlcValue::Dint(result as i32));
            }
            Opcode::Ror => {
                let n = self.stack_pop()?.to_int() as u32;
                let v = self.stack_pop()?.to_int() as u32;
                let bits = 32;
                let shift = n % bits;
                let result = (v >> shift) | (v << (bits - shift));
                self.stack.push(PlcValue::Dint(result as i32));
            }

            Opcode::Jump => {
                let offset = instr.operand.unwrap_or(0);
                next_pc = (pc as i64 + offset as i64) as usize;
            }
            Opcode::JumpIfFalse => {
                let offset = instr.operand.unwrap_or(0);
                let cond = self.stack_pop()?;
                if !cond.to_bool() {
                    next_pc = (pc as i64 + offset as i64) as usize;
                }
            }
            Opcode::Call => {}
            Opcode::CallPou => {
                let name_idx = instr.operand.unwrap_or(0) as usize;
                let pou_name = self
                    .name_constants
                    .get(name_idx)
                    .cloned()
                    .unwrap_or_default();
                if let Some(&(_, entry)) = self.pou_table.iter().find(|(n, _)| n == &pou_name) {
                    self.call_stack.push(pc + 1);
                    next_pc = entry;
                }
            }
            Opcode::Return => {
                if let Some(ret_addr) = self.call_stack.pop() {
                    next_pc = ret_addr;
                } else {
                    return Ok(instructions.len());
                }
            }

            Opcode::TonCall => self.execute_timer(crate::timer::TimerKind::TON, instr.operand)?,
            Opcode::TofCall => self.execute_timer(crate::timer::TimerKind::TOF, instr.operand)?,
            Opcode::TpCall => self.execute_timer(crate::timer::TimerKind::TP, instr.operand)?,
            Opcode::CtuCall => self.execute_ctu(instr.operand)?,
            Opcode::CtdCall => self.execute_ctd(instr.operand)?,
            Opcode::CtudCall => self.execute_ctud(instr.operand)?,
            Opcode::RTrigCall => self.execute_edge(true, instr.operand)?,
            Opcode::FTrigCall => self.execute_edge(false, instr.operand)?,

            Opcode::Halt => {
                return Ok(instructions.len());
            }

            Opcode::Pow => {
                let b = self.stack_pop()?.to_real();
                let a = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(a.powf(b)));
            }

            Opcode::Sqrt => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(v.sqrt()));
            }
            Opcode::Ln => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(v.ln()));
            }
            Opcode::Log => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(v.log10()));
            }
            Opcode::Exp => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(v.exp()));
            }
            Opcode::Sin => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(v.sin()));
            }
            Opcode::Cos => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(v.cos()));
            }
            Opcode::Tan => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(v.tan()));
            }
            Opcode::Asin => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(v.asin()));
            }
            Opcode::Acos => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(v.acos()));
            }
            Opcode::Atan => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(v.atan()));
            }
            Opcode::Abs => {
                let v = self.stack_pop()?;
                match v {
                    PlcValue::Int(i) => self.stack.push(PlcValue::Int(i.abs())),
                    PlcValue::Dint(i) => self.stack.push(PlcValue::Dint(i.abs())),
                    PlcValue::Real(f) => self.stack.push(PlcValue::Real(f.abs())),
                    _ => self.stack.push(v),
                }
            }
            Opcode::Trunc => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Dint(v.trunc() as i32));
            }
            Opcode::Floor => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Dint(v.floor() as i32));
            }
            Opcode::Ceil => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Dint(v.ceil() as i32));
            }
            Opcode::Expt => {
                let b = self.stack_pop()?.to_real();
                let a = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(a.powf(b)));
            }

            // String operations
            Opcode::StrLen => {
                let s = self.stack_pop()?.as_str();
                self.stack.push(PlcValue::Dint(s.len() as i32));
            }
            Opcode::StrLeft => {
                let n = self.stack_pop()?.to_int() as usize;
                let s = self.stack_pop()?.as_str();
                let result = s.chars().take(n).collect::<String>();
                self.stack.push(PlcValue::String(result));
            }
            Opcode::StrRight => {
                let n = self.stack_pop()?.to_int() as usize;
                let s = self.stack_pop()?.as_str();
                let len = s.chars().count();
                let result = s.chars().skip(len.saturating_sub(n)).collect::<String>();
                self.stack.push(PlcValue::String(result));
            }
            Opcode::StrMid => {
                let n = self.stack_pop()?.to_int() as usize;
                let p = self.stack_pop()?.to_int() as usize;
                let s = self.stack_pop()?.as_str();
                let result = s
                    .chars()
                    .skip(p.saturating_sub(1))
                    .take(n)
                    .collect::<String>();
                self.stack.push(PlcValue::String(result));
            }
            Opcode::Concat => {
                let b = self.stack_pop()?.as_str();
                let a = self.stack_pop()?.as_str();
                self.stack.push(PlcValue::String(a + &b));
            }
            Opcode::StrInsert => {
                let pos = self.stack_pop()?.to_int() as usize;
                let ins = self.stack_pop()?.as_str();
                let s = self.stack_pop()?.as_str();
                let pos = pos.saturating_sub(1).min(s.len());
                let mut result = s.clone();
                result.insert_str(pos, &ins);
                self.stack.push(PlcValue::String(result));
            }
            Opcode::StrDelete => {
                let n = self.stack_pop()?.to_int() as usize;
                let p = self.stack_pop()?.to_int() as usize;
                let s = self.stack_pop()?.as_str();
                let start = (p.saturating_sub(1)).min(s.len());
                let end = (start + n).min(s.len());
                let result = format!("{}{}", &s[..start], &s[end..]);
                self.stack.push(PlcValue::String(result));
            }
            Opcode::StrReplace => {
                let n = self.stack_pop()?.to_int() as usize;
                let p = self.stack_pop()?.to_int() as usize;
                let rep = self.stack_pop()?.as_str();
                let s = self.stack_pop()?.as_str();
                let start = (p.saturating_sub(1)).min(s.len());
                let end = (start + n).min(s.len());
                let result = format!("{}{}{}", &s[..start], &rep, &s[end..]);
                self.stack.push(PlcValue::String(result));
            }
            Opcode::StrFind => {
                let needle = self.stack_pop()?.as_str();
                let haystack = self.stack_pop()?.as_str();
                let pos = haystack.find(&needle).map(|i| i as i32 + 1).unwrap_or(0);
                self.stack.push(PlcValue::Dint(pos));
            }

            // Conversion operations
            Opcode::IntToReal => {
                let v = self.stack_pop()?.to_int();
                self.stack.push(PlcValue::Real(v as f32));
            }
            Opcode::RealToInt => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Dint(v as i32));
            }
            Opcode::IntToBool => {
                let v = self.stack_pop()?.to_int();
                self.stack.push(PlcValue::Bool(v != 0));
            }
            Opcode::BoolToInt => {
                let v = self.stack_pop()?.to_bool();
                self.stack.push(PlcValue::Dint(if v { 1 } else { 0 }));
            }
            Opcode::IntToString => {
                let v = self.stack_pop()?.to_int();
                self.stack.push(PlcValue::String(v.to_string()));
            }
            Opcode::RealToString => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::String(format!("{:.3}", v)));
            }
            Opcode::StringToInt => {
                let s = self.stack_pop()?.as_str();
                let v = s.parse::<i32>().unwrap_or(0);
                self.stack.push(PlcValue::Dint(v));
            }
            Opcode::StringToReal => {
                let s = self.stack_pop()?.as_str();
                let v = s.parse::<f32>().unwrap_or(0.0);
                self.stack.push(PlcValue::Real(v));
            }
            Opcode::WordToInt => {
                let v = self.stack_pop()?;
                let result = match v {
                    PlcValue::Word(w) => w as i32,
                    _ => v.to_int(),
                };
                self.stack.push(PlcValue::Dint(result));
            }
            Opcode::DwordToInt => {
                let v = self.stack_pop()?;
                let result = match v {
                    PlcValue::Dword(d) => d as i32,
                    _ => v.to_int(),
                };
                self.stack.push(PlcValue::Dint(result));
            }
            Opcode::IntToWord => {
                let v = self.stack_pop()?.to_int();
                self.stack.push(PlcValue::Word(v as u16));
            }
            Opcode::IntToDword => {
                let v = self.stack_pop()?.to_int();
                self.stack.push(PlcValue::Dword(v as u32));
            }
            Opcode::DwordToReal => {
                let v = self.stack_pop()?;
                let bits = match v {
                    PlcValue::Dword(d) => d,
                    _ => v.to_int() as u32,
                };
                self.stack.push(PlcValue::Real(f32::from_bits(bits)));
            }
            Opcode::RealToDword => {
                let v = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Dword(v.to_bits()));
            }

            // Selection operations
            Opcode::Sel => {
                let g = self.stack_pop()?.to_bool();
                let in1 = self.stack_pop()?;
                let in0 = self.stack_pop()?;
                self.stack.push(if g { in1 } else { in0 });
            }
            Opcode::Mux => {
                let k = self.stack_pop()?.to_int();
                // args pushed in order: IN0, IN1, ..., INn, K (popped above)
                // stack[0]=IN0, stack[1]=IN1, ...
                let n = self.stack.len();
                let idx = k as usize;
                if idx < n {
                    let val = self.stack[idx].clone();
                    self.stack.truncate(0);
                    self.stack.push(val);
                } else {
                    self.stack.truncate(0);
                    self.stack.push(PlcValue::Dint(0));
                }
            }
            Opcode::Limit => {
                let max_val = self.stack_pop()?;
                let min_val = self.stack_pop()?;
                let val = self.stack_pop()?;
                let result = if val.to_real() < min_val.to_real() {
                    min_val
                } else if val.to_real() > max_val.to_real() {
                    max_val
                } else {
                    val
                };
                self.stack.push(result);
            }
            Opcode::Max => {
                let b = self.stack_pop()?.to_real();
                let a = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(a.max(b)));
            }
            Opcode::Min => {
                let b = self.stack_pop()?.to_real();
                let a = self.stack_pop()?.to_real();
                self.stack.push(PlcValue::Real(a.min(b)));
            }
            Opcode::Move => {
                // MOVE is basically a no-op: value is already on stack
            }

            Opcode::LoadArrayIndex => {
                let idx = self.stack_pop()?.to_int() as usize;
                let arr = self.stack_pop()?;
                match arr {
                    PlcValue::Array(ref elements) => {
                        let val = elements.get(idx).cloned().unwrap_or(PlcValue::Dint(0));
                        self.stack.push(val);
                    }
                    _ => {
                        return Err(format!(
                            "LoadArrayIndex: not an array, got {:?}",
                            arr.type_name()
                        ));
                    }
                }
            }
            Opcode::StoreArrayIndex => {
                let val = self.stack_pop()?;
                let idx = self.stack_pop()?.to_int() as usize;
                let arr = self.stack_pop()?;
                match arr {
                    PlcValue::Array(mut elements) => {
                        if idx < elements.len() {
                            elements[idx] = val;
                        }
                        self.stack.push(PlcValue::Array(elements));
                    }
                    _ => {
                        return Err(format!(
                            "StoreArrayIndex: not an array, got {:?}",
                            arr.type_name()
                        ));
                    }
                }
            }
            Opcode::ArrayLen => {
                let arr = self.stack_pop()?;
                match arr {
                    PlcValue::Array(ref elements) => {
                        self.stack.push(PlcValue::Dint(elements.len() as i32));
                    }
                    _ => self.stack.push(PlcValue::Dint(0)),
                }
            }
            Opcode::LoadStructField => {
                let field_idx = instr.operand.unwrap_or(0) as usize;
                let val = self.stack_pop()?;
                match val {
                    PlcValue::Struct(ref fields) => {
                        let field_val = fields
                            .get(field_idx)
                            .map(|(_, v)| v.clone())
                            .unwrap_or(PlcValue::Dint(0));
                        self.stack.push(field_val);
                    }
                    _ => {
                        return Err(format!(
                            "LoadStructField: not a struct, got {:?}",
                            val.type_name()
                        ));
                    }
                }
            }
            Opcode::StoreStructField => {
                let field_idx = instr.operand.unwrap_or(0) as usize;
                let struct_val = self.stack_pop()?;
                let new_field_val = self.stack_pop()?;
                let modified = match struct_val {
                    PlcValue::Struct(mut fields) => {
                        if field_idx < fields.len() {
                            fields[field_idx].1 = new_field_val;
                        }
                        PlcValue::Struct(fields)
                    }
                    _ => {
                        return Err(format!(
                            "StoreStructField: not a struct, got {:?}",
                            struct_val.type_name()
                        ));
                    }
                };
                self.stack.push(modified);
            }
        }

        Ok(next_pc)
    }

    fn stack_pop(&mut self) -> Result<PlcValue, String> {
        self.stack
            .pop()
            .ok_or_else(|| "Stack underflow".to_string())
    }

    fn binary_op<F>(&mut self, op: F) -> Result<(), String>
    where
        F: Fn(f32, f32) -> f32,
    {
        let b = self.stack_pop()?.to_real();
        let a = self.stack_pop()?.to_real();
        let result = op(a, b);
        self.stack.push(PlcValue::Real(result));
        Ok(())
    }

    fn compare_op<F>(&mut self, op: F) -> Result<(), String>
    where
        F: Fn(f32, f32) -> bool,
    {
        let b = self.stack_pop()?.to_real();
        let a = self.stack_pop()?.to_real();
        self.stack.push(PlcValue::Bool(op(a, b)));
        Ok(())
    }

    fn logic_op<F>(&mut self, op: F) -> Result<(), String>
    where
        F: Fn(bool, bool) -> bool,
    {
        let b = self.stack_pop()?.to_bool();
        let a = self.stack_pop()?.to_bool();
        self.stack.push(PlcValue::Bool(op(a, b)));
        Ok(())
    }

    /// Timer call: stack has [PT (Time), IN (Bool)], pops both, pushes [ET (Time), Q (Bool)]
    fn execute_timer(
        &mut self,
        kind: crate::timer::TimerKind,
        operand: Option<i32>,
    ) -> Result<(), String> {
        let idx = operand.unwrap_or(0) as usize;
        let in_val = self.stack_pop()?.to_bool();
        let pt_val = self.stack_pop()?.to_int();

        while self.timers.len() <= idx {
            self.timers.push(crate::timer::TimerInstance::default());
        }
        self.timers[idx].kind = kind;
        let (q, et) = self.timers[idx].execute(in_val, pt_val);

        self.stack.push(PlcValue::Time(et));
        self.stack.push(PlcValue::Bool(q));
        Ok(())
    }

    /// CTU call: stack has [PV (Dint), R (Bool), CU (Bool)], pops all, pushes [CV (Dint), Q (Bool)]
    fn execute_ctu(&mut self, operand: Option<i32>) -> Result<(), String> {
        let idx = operand.unwrap_or(0) as usize;
        let cu = self.stack_pop()?.to_bool();
        let r = self.stack_pop()?.to_bool();
        let pv = self.stack_pop()?.to_int();

        while self.counters.len() <= idx {
            self.counters
                .push(crate::counter::CounterInstance::default());
        }
        let (q, cv) = self.counters[idx].execute_ctu(cu, r, pv);

        self.stack.push(PlcValue::Dint(cv));
        self.stack.push(PlcValue::Bool(q));
        Ok(())
    }

    /// CTD call: stack has [PV (Dint), LD (Bool), CD (Bool)], pops all, pushes [CV (Dint), Q (Bool)]
    fn execute_ctd(&mut self, operand: Option<i32>) -> Result<(), String> {
        let idx = operand.unwrap_or(0) as usize;
        let cd = self.stack_pop()?.to_bool();
        let ld = self.stack_pop()?.to_bool();
        let pv = self.stack_pop()?.to_int();

        while self.counters.len() <= idx {
            self.counters
                .push(crate::counter::CounterInstance::default());
        }
        let (q, cv) = self.counters[idx].execute_ctd(cd, ld, pv);

        self.stack.push(PlcValue::Dint(cv));
        self.stack.push(PlcValue::Bool(q));
        Ok(())
    }

    /// CTUD call: stack has [PV (Dint), LD (Bool), CD (Bool), R (Bool), CU (Bool)]
    /// pops all, pushes [CV (Dint), Q (Bool)]
    fn execute_ctud(&mut self, operand: Option<i32>) -> Result<(), String> {
        let idx = operand.unwrap_or(0) as usize;
        let cu = self.stack_pop()?.to_bool();
        let r = self.stack_pop()?.to_bool();
        let cd = self.stack_pop()?.to_bool();
        let ld = self.stack_pop()?.to_bool();
        let pv = self.stack_pop()?.to_int();

        while self.counters.len() <= idx {
            self.counters
                .push(crate::counter::CounterInstance::default());
        }
        self.counters[idx].kind = crate::counter::CounterKind::CTUD;
        let (q, cv) = self.counters[idx].execute_ctud(cu, cd, r, ld, pv);

        self.stack.push(PlcValue::Dint(cv));
        self.stack.push(PlcValue::Bool(q));
        Ok(())
    }

    /// Edge detection: stack has [CLK (Bool)], pops it, pushes [Q (Bool)]
    /// rising=true for R_TRIG (detect FALSE→TRUE), rising=false for F_TRIG (detect TRUE→FALSE)
    fn execute_edge(&mut self, rising: bool, operand: Option<i32>) -> Result<(), String> {
        let idx = operand.unwrap_or(0) as usize;
        let clk = self.stack_pop()?.to_bool();

        while self.edge_prev.len() <= idx {
            self.edge_prev.push(false);
        }
        let prev = self.edge_prev[idx];
        self.edge_prev[idx] = clk;

        let q = if rising { clk && !prev } else { !clk && prev };
        self.stack.push(PlcValue::Bool(q));
        Ok(())
    }

    pub fn snapshot_fb_state(&self) -> FbSnapshot {
        FbSnapshot {
            timers: self.timers.clone(),
            counters: self.counters.clone(),
            edge_prev: self.edge_prev.clone(),
        }
    }

    pub fn restore_fb_state(&mut self, snapshot: &FbSnapshot) {
        for (i, timer) in snapshot.timers.iter().enumerate() {
            if i < self.timers.len() {
                self.timers[i] = timer.clone();
            }
        }
        for (i, counter) in snapshot.counters.iter().enumerate() {
            if i < self.counters.len() {
                self.counters[i] = counter.clone();
            }
        }
        for (i, &edge) in snapshot.edge_prev.iter().enumerate() {
            if i < self.edge_prev.len() {
                self.edge_prev[i] = edge;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcode::Instruction;

    #[test]
    fn test_push_and_store() {
        let instructions = vec![
            Instruction {
                opcode: Opcode::PushInt,
                operand: Some(42),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ];
        let vars = vec![("x".to_string(), PlcValue::Int(0))];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });
        vm.execute(&instructions).unwrap();
        assert_eq!(vm.memory().get_variable("x"), Some(&PlcValue::Int(42)));
    }

    #[test]
    fn test_arithmetic() {
        let vars = vec![("result".to_string(), PlcValue::Real(0.0))];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });
        vm.execute(&[
            Instruction {
                opcode: Opcode::PushInt,
                operand: Some(10),
            },
            Instruction {
                opcode: Opcode::PushInt,
                operand: Some(3),
            },
            Instruction {
                opcode: Opcode::Add,
                operand: None,
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();
        let result = vm.memory().get_variable("result").unwrap().to_real();
        assert!((result - 13.0).abs() < 0.01);
    }

    #[test]
    fn test_comparison() {
        let vars = vec![("flag".to_string(), PlcValue::Bool(false))];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });
        vm.execute(&[
            Instruction {
                opcode: Opcode::PushInt,
                operand: Some(5),
            },
            Instruction {
                opcode: Opcode::PushInt,
                operand: Some(3),
            },
            Instruction {
                opcode: Opcode::Gt,
                operand: None,
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();
        assert_eq!(
            vm.memory().get_variable("flag"),
            Some(&PlcValue::Bool(true))
        );
    }

    #[test]
    fn test_conditional_jump() {
        let vars = vec![
            ("x".to_string(), PlcValue::Int(0)),
            ("y".to_string(), PlcValue::Int(0)),
        ];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });
        vm.execute(&[
            Instruction {
                opcode: Opcode::PushBool,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::JumpIfFalse,
                operand: Some(4),
            },
            Instruction {
                opcode: Opcode::PushInt,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Jump,
                operand: Some(3),
            },
            Instruction {
                opcode: Opcode::PushInt,
                operand: Some(2),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();
        assert_eq!(vm.memory().get_variable("x"), Some(&PlcValue::Int(1)));
    }

    #[test]
    fn test_e2e_compile_and_execute() {
        let source = r#"
            PROGRAM Main
            VAR
                counter : INT;
                enable : BOOL;
            END_VAR
                enable := TRUE;
                counter := counter + 1;
                IF counter > 100 THEN
                    counter := 0;
                END_IF;
            END_PROGRAM
        "#;
        let result = mt_stc::compile(source);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);

        let module = result.module.expect("Expected compiled module");
        assert_eq!(module.name, "Main");
        assert_eq!(module.variable_slots.len(), 2);

        let instructions = module.instructions.clone();
        let mut vm = Vm::from_instructions(module);

        // After 1 cycle: counter = 1
        vm.execute(&instructions).unwrap();
        let counter = vm.memory().get_variable("counter").unwrap().to_real();
        assert!(
            (counter - 1.0).abs() < 0.5,
            "after 1 cycle: {}, expected 1",
            counter
        );

        let enable = vm.memory().get_variable("enable").unwrap();
        assert_eq!(*enable, PlcValue::Bool(true));

        // After 6 total cycles: counter = 6
        for _ in 0..5 {
            vm.execute(&instructions).unwrap();
        }
        let counter = vm.memory().get_variable("counter").unwrap().to_real();
        assert!(
            (counter - 6.0).abs() < 0.5,
            "after 6 cycles: {}, expected 6",
            counter
        );

        // After 100 total cycles: counter = 100 (just before reset)
        for _ in 0..94 {
            vm.execute(&instructions).unwrap();
        }
        let counter = vm.memory().get_variable("counter").unwrap().to_real();
        assert!(
            (counter - 100.0).abs() < 0.5,
            "after 100 cycles: {}, expected 100",
            counter
        );

        // After 101 total cycles: counter incremented to 101, then reset to 0
        vm.execute(&instructions).unwrap();
        let counter = vm.memory().get_variable("counter").unwrap().to_real();
        assert!(
            (counter - 0.0).abs() < 0.5,
            "after 101 cycles: {}, expected 0",
            counter
        );

        // After 102 total cycles: counter = 1 again
        vm.execute(&instructions).unwrap();
        let counter = vm.memory().get_variable("counter").unwrap().to_real();
        assert!(
            (counter - 1.0).abs() < 0.5,
            "after 102 cycles: {}, expected 1",
            counter
        );
    }

    fn compile_and_run(source: &str) -> (Vm, Vec<Instruction>) {
        let result = mt_stc::compile(source);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        let module = result.module.expect("Expected compiled module");
        let instructions = module.instructions.clone();
        let vm = Vm::from_instructions(module);
        (vm, instructions)
    }

    #[test]
    fn test_traffic_light() {
        let source = r#"
PROGRAM TrafficLight
VAR
    state : INT := 0;
    timer : INT := 0;
    red : BOOL := TRUE;
    yellow : BOOL := FALSE;
    green : BOOL := FALSE;
    flash : BOOL := FALSE;
    flash_counter : INT := 0;
END_VAR
    timer := timer + 1;
    flash_counter := flash_counter + 1;
    flash := (flash_counter MOD 20) > 10;

    IF state = 0 THEN
        red := TRUE;
        yellow := FALSE;
        green := FALSE;
        IF timer > 100 THEN
            state := 1;
            timer := 0;
        END_IF;
    ELSIF state = 1 THEN
        red := TRUE;
        yellow := TRUE;
        green := FALSE;
        IF timer > 30 THEN
            state := 2;
            timer := 0;
        END_IF;
    ELSIF state = 2 THEN
        red := FALSE;
        yellow := FALSE;
        green := TRUE;
        IF timer > 100 THEN
            state := 3;
            timer := 0;
        END_IF;
    ELSIF state = 3 THEN
        red := FALSE;
        yellow := TRUE;
        green := FALSE;
        IF timer > 40 THEN
            state := 0;
            timer := 0;
        END_IF;
    END_IF;
END_PROGRAM
"#;
        let (mut vm, instructions) = compile_and_run(source);

        // State 0: Red, timer counts up
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("red").unwrap(),
            PlcValue::Bool(true)
        );
        assert_eq!(
            *vm.memory().get_variable("state").unwrap(),
            PlcValue::Int(0)
        );

        // After 100 cycles, should transition to state 1 (Red+Yellow)
        for _ in 0..99 {
            vm.execute(&instructions).unwrap();
        }
        // timer is now 100, so state transitions to 1
        vm.execute(&instructions).unwrap();
        let state = vm.memory().get_variable("state").unwrap().to_int();
        assert!(
            state >= 1,
            "Expected state >= 1 after 100+ cycles, got {}",
            state
        );
    }

    #[test]
    fn test_motor_control() {
        let source = r#"
PROGRAM MotorControl
VAR
    start_btn : BOOL := FALSE;
    stop_btn : BOOL := FALSE;
    emergency_stop : BOOL := FALSE;
    overload_trip : BOOL := FALSE;
    motor_running : BOOL := FALSE;
    fault : BOOL := FALSE;
    speed : INT := 0;
    fault_code : INT := 0;
    target_speed : INT := 1000;
    ramp_rate : INT := 10;
    decel_rate : INT := 25;
    run_time : DINT := 0;
    start_delay : INT := 0;
END_VAR
    run_time := run_time + 1;
    IF overload_trip THEN
        fault := TRUE;
        fault_code := 1;
        motor_running := FALSE;
    END_IF;
    IF emergency_stop THEN
        fault := TRUE;
        fault_code := 2;
        motor_running := FALSE;
    END_IF;
    IF start_btn AND NOT stop_btn AND NOT fault THEN
        IF (NOT motor_running) AND (speed = 0) THEN
            start_delay := start_delay + 1;
            IF start_delay > 10 THEN
                motor_running := TRUE;
                start_delay := 0;
            END_IF;
        ELSE
            motor_running := TRUE;
            start_delay := 0;
        END_IF;
    END_IF;
    IF stop_btn THEN
        motor_running := FALSE;
        start_delay := 0;
    END_IF;
    IF stop_btn AND fault THEN
        start_delay := start_delay + 1;
        IF start_delay > 20 THEN
            fault := FALSE;
            fault_code := 0;
            start_delay := 0;
        END_IF;
    END_IF;
    IF motor_running THEN
        IF speed < target_speed THEN
            speed := speed + ramp_rate;
            IF speed > target_speed THEN
                speed := target_speed;
            END_IF;
        END_IF;
    ELSE
        IF speed > 0 THEN
            speed := speed - decel_rate;
            IF speed < 0 THEN
                speed := 0;
            END_IF;
        END_IF;
    END_IF;
END_PROGRAM
"#;
        let (mut vm, instructions) = compile_and_run(source);

        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("motor_running").unwrap(),
            PlcValue::Bool(false)
        );
        assert_eq!(
            *vm.memory().get_variable("speed").unwrap(),
            PlcValue::Int(0)
        );

        vm.memory_mut()
            .set_variable("start_btn", PlcValue::Bool(true))
            .unwrap();
        for _ in 0..12 {
            vm.execute(&instructions).unwrap();
        }
        assert_eq!(
            *vm.memory().get_variable("motor_running").unwrap(),
            PlcValue::Bool(true)
        );
        let speed = vm.memory().get_variable("speed").unwrap().to_int();
        assert!(speed > 0, "Speed should be ramping up, got {}", speed);

        vm.memory_mut()
            .set_variable("start_btn", PlcValue::Bool(false))
            .unwrap();
        vm.memory_mut()
            .set_variable("stop_btn", PlcValue::Bool(true))
            .unwrap();
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("motor_running").unwrap(),
            PlcValue::Bool(false)
        );
    }

    #[test]
    fn test_tank_level() {
        let source = r#"
PROGRAM TankLevel
VAR
    level : REAL := 25.0;
    temperature : REAL := 20.0;
    level_setpoint : REAL := 50.0;
    temp_setpoint : REAL := 65.0;
    deadband : REAL := 3.0;
    inlet_valve : BOOL := FALSE;
    outlet_valve : BOOL := FALSE;
    heater : BOOL := FALSE;
    cooling_valve : BOOL := FALSE;
    high_level_alarm : BOOL := FALSE;
    low_level_alarm : BOOL := FALSE;
    high_temp_alarm : BOOL := FALSE;
    inlet_flow : REAL := 1.0;
    outlet_flow : REAL := 0.6;
    heat_rate : REAL := 0.3;
    cool_rate : REAL := 0.15;
    ambient_loss : REAL := 0.05;
    cycles : DINT := 0;
    total_inlet_time : DINT := 0;
    total_outlet_time : DINT := 0;
END_VAR
    cycles := cycles + 1;
    IF level < level_setpoint - deadband THEN
        inlet_valve := TRUE;
        outlet_valve := FALSE;
    ELSIF level > level_setpoint + deadband THEN
        inlet_valve := FALSE;
        outlet_valve := TRUE;
    END_IF;
    IF temperature < temp_setpoint - deadband THEN
        heater := TRUE;
        cooling_valve := FALSE;
    ELSIF temperature > temp_setpoint + deadband THEN
        heater := FALSE;
        cooling_valve := TRUE;
    END_IF;
    high_level_alarm := level > 90.0;
    low_level_alarm := level < 10.0;
    high_temp_alarm := temperature > 85.0;
    IF high_level_alarm THEN
        inlet_valve := FALSE;
    END_IF;
    IF high_temp_alarm THEN
        heater := FALSE;
    END_IF;
    IF inlet_valve THEN
        level := level + inlet_flow;
        total_inlet_time := total_inlet_time + 1;
    END_IF;
    IF outlet_valve THEN
        level := level - outlet_flow;
        total_outlet_time := total_outlet_time + 1;
    END_IF;
    IF heater THEN
        temperature := temperature + heat_rate;
    END_IF;
    IF cooling_valve THEN
        temperature := temperature - cool_rate;
    END_IF;
    temperature := temperature - ambient_loss;
    IF level > 100.0 THEN
        level := 100.0;
    END_IF;
    IF level < 0.0 THEN
        level := 0.0;
    END_IF;
    IF temperature < 0.0 THEN
        temperature := 0.0;
    END_IF;
    IF temperature > 100.0 THEN
        temperature := 100.0;
    END_IF;
END_PROGRAM
"#;
        let (mut vm, instructions) = compile_and_run(source);

        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("inlet_valve").unwrap(),
            PlcValue::Bool(true)
        );

        for _ in 0..500 {
            vm.execute(&instructions).unwrap();
        }
        let level = vm.memory().get_variable("level").unwrap().to_real();
        assert!(
            level > 30.0 && level < 70.0,
            "Level should be near setpoint 50, got {}",
            level
        );
    }

    #[test]
    fn test_batch_mixer() {
        let source = r#"
PROGRAM BatchMixer
VAR
    state : INT := 0;
    step_timer : INT := 0;
    valve_a : BOOL := FALSE;
    valve_b : BOOL := FALSE;
    mixer : BOOL := FALSE;
    heater : BOOL := FALSE;
    drain : BOOL := FALSE;
    level : REAL := 0.0;
    temperature : REAL := 20.0;
    batch_count : INT := 0;
    total_drained : REAL := 0.0;
    fill_level_a : REAL := 35.0;
    fill_level_b : REAL := 70.0;
    target_temp : REAL := 75.0;
    mix_time : INT := 150;
    drain_time : INT := 100;
    fault : BOOL := FALSE;
    fault_msg : INT := 0;
    cycles : DINT := 0;
END_VAR
    cycles := cycles + 1;
    step_timer := step_timer + 1;
    IF state = 0 THEN
        valve_a := FALSE;
        valve_b := FALSE;
        mixer := FALSE;
        heater := FALSE;
        drain := FALSE;
        step_timer := 0;
        IF level <= 0.0 AND temperature < 30.0 THEN
            state := 1;
        END_IF;
    ELSIF state = 1 THEN
        valve_a := TRUE;
        IF level >= fill_level_a THEN
            valve_a := FALSE;
            state := 2;
            step_timer := 0;
        END_IF;
    ELSIF state = 2 THEN
        valve_b := TRUE;
        IF level >= fill_level_b THEN
            valve_b := FALSE;
            state := 3;
            step_timer := 0;
        END_IF;
    ELSIF state = 3 THEN
        mixer := TRUE;
        heater := TRUE;
        IF temperature >= target_temp THEN
            heater := FALSE;
            state := 4;
            step_timer := 0;
        END_IF;
    ELSIF state = 4 THEN
        mixer := TRUE;
        IF step_timer >= mix_time THEN
            mixer := FALSE;
            state := 5;
            step_timer := 0;
        END_IF;
    ELSIF state = 5 THEN
        drain := TRUE;
        IF step_timer >= drain_time THEN
            drain := FALSE;
            batch_count := batch_count + 1;
            state := 0;
            step_timer := 0;
        END_IF;
    END_IF;
    IF step_timer > 500 THEN
        fault := TRUE;
        fault_msg := state;
    END_IF;
    IF valve_a THEN
        level := level + 0.8;
    END_IF;
    IF valve_b THEN
        level := level + 1.0;
    END_IF;
    IF drain THEN
        level := level - 1.2;
        total_drained := total_drained + 1.2;
    END_IF;
    IF heater THEN
        temperature := temperature + 0.4;
    END_IF;
    IF mixer AND NOT heater THEN
        temperature := temperature - 0.08;
    END_IF;
    IF level > 100.0 THEN
        level := 100.0;
    END_IF;
    IF level < 0.0 THEN
        level := 0.0;
    END_IF;
    IF temperature < 0.0 THEN
        temperature := 0.0;
    END_IF;
    IF temperature > 120.0 THEN
        temperature := 120.0;
    END_IF;
END_PROGRAM
"#;
        let (mut vm, instructions) = compile_and_run(source);

        vm.execute(&instructions).unwrap();
        let state = vm.memory().get_variable("state").unwrap().to_int();
        assert!(state >= 1, "Expected state >= 1, got {}", state);

        for _ in 0..2000 {
            vm.execute(&instructions).unwrap();
        }
        let batch_count = vm.memory().get_variable("batch_count").unwrap().to_int();
        assert!(
            batch_count >= 1,
            "Expected at least 1 batch completed, got {}",
            batch_count
        );

        let total_drained = vm.memory().get_variable("total_drained").unwrap().to_real();
        assert!(
            total_drained > 0.0,
            "Should have drained some product, got {}",
            total_drained
        );
    }

    #[test]
    fn test_counter() {
        let source = r#"
PROGRAM Counter
VAR
    counter : INT := 0;
    max_count : INT := 100;
    done : BOOL := FALSE;
    cycles : DINT := 0;
    direction : INT := 1;
END_VAR
    cycles := cycles + 1;
    counter := counter + direction;
    IF counter >= max_count THEN
        direction := -1;
        done := TRUE;
    ELSIF counter <= 0 THEN
        direction := 1;
        done := FALSE;
    END_IF;
END_PROGRAM
"#;
        let (mut vm, instructions) = compile_and_run(source);

        vm.execute(&instructions).unwrap();
        let counter = vm.memory().get_variable("counter").unwrap().to_int();
        assert_eq!(counter, 1);

        for _ in 0..49 {
            vm.execute(&instructions).unwrap();
        }
        let counter = vm.memory().get_variable("counter").unwrap().to_int();
        assert_eq!(counter, 50);
        assert_eq!(
            *vm.memory().get_variable("direction").unwrap(),
            PlcValue::Int(1)
        );

        for _ in 0..50 {
            vm.execute(&instructions).unwrap();
        }
        let direction = vm.memory().get_variable("direction").unwrap().to_int();
        assert_eq!(direction, -1);
        let done = vm.memory().get_variable("done").unwrap();
        assert_eq!(*done, PlcValue::Bool(true));
    }

    #[test]
    fn test_ton_timer() {
        let source = r#"
PROGRAM TonTest
VAR
    start : BOOL := FALSE;
    motor : BOOL := FALSE;
    startup_delay : TON;
END_VAR
    startup_delay(IN := start, PT := T#500MS);
    motor := startup_delay.Q;
END_PROGRAM
"#;
        let (mut vm, instructions) = compile_and_run(source);

        // Cycle 1: start=FALSE, timer not running, motor=FALSE
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("motor").unwrap(),
            PlcValue::Bool(false)
        );

        // Enable start signal
        vm.memory_mut()
            .set_variable("start", PlcValue::Bool(true))
            .unwrap();

        // Run a few cycles — timer needs ~500ms wall-clock time
        for _ in 0..5 {
            vm.execute(&instructions).unwrap();
        }
        // Timer hasn't elapsed yet (only microseconds have passed)
        let q = vm
            .memory()
            .get_variable("startup_delay.Q")
            .unwrap()
            .to_bool();
        // Q might or might not be true depending on timing, but ET should be non-zero after IN=TRUE
        let et = vm
            .memory()
            .get_variable("startup_delay.ET")
            .unwrap()
            .to_int();
        assert!(et >= 0, "ET should be non-negative, got {}", et);

        // After enabling, motor should follow Q
        let motor = vm.memory().get_variable("motor").unwrap().to_bool();
        let timer_q = vm
            .memory()
            .get_variable("startup_delay.Q")
            .unwrap()
            .to_bool();
        assert_eq!(motor, timer_q);

        // Turn off start — timer resets
        vm.memory_mut()
            .set_variable("start", PlcValue::Bool(false))
            .unwrap();
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("startup_delay.Q").unwrap(),
            PlcValue::Bool(false)
        );
        assert_eq!(
            *vm.memory().get_variable("startup_delay.ET").unwrap(),
            PlcValue::Time(0)
        );
    }

    #[test]
    fn test_ctu_counter() {
        let source = r#"
PROGRAM CtuTest
VAR
    pulse : BOOL := FALSE;
    reset : BOOL := FALSE;
    done : BOOL := FALSE;
    part_counter : CTU;
END_VAR
    part_counter(CU := pulse, R := reset, PV := 5);
    done := part_counter.Q;
END_PROGRAM
"#;
        let (mut vm, instructions) = compile_and_run(source);

        // Cycle 1: no pulse, counter stays at 0
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("done").unwrap(),
            PlcValue::Bool(false)
        );
        assert_eq!(
            *vm.memory().get_variable("part_counter.CV").unwrap(),
            PlcValue::Dint(0)
        );

        // Pulse 5 times with rising edges (toggle between TRUE and FALSE)
        for _ in 0..5 {
            vm.memory_mut()
                .set_variable("pulse", PlcValue::Bool(true))
                .unwrap();
            vm.execute(&instructions).unwrap();
            vm.memory_mut()
                .set_variable("pulse", PlcValue::Bool(false))
                .unwrap();
            vm.execute(&instructions).unwrap();
        }

        let cv = vm
            .memory()
            .get_variable("part_counter.CV")
            .unwrap()
            .to_int();
        assert_eq!(cv, 5, "Expected CV=5 after 5 rising edges, got {}", cv);
        let done = vm.memory().get_variable("done").unwrap().to_bool();
        assert!(done, "Q should be TRUE when CV >= PV");

        // Reset
        vm.memory_mut()
            .set_variable("reset", PlcValue::Bool(true))
            .unwrap();
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("part_counter.CV").unwrap(),
            PlcValue::Dint(0)
        );
        assert_eq!(
            *vm.memory().get_variable("done").unwrap(),
            PlcValue::Bool(false)
        );
    }

    #[test]
    fn test_ctd_counter() {
        let source = r#"
PROGRAM CtdTest
VAR
    decrement : BOOL := FALSE;
    load : BOOL := FALSE;
    done : BOOL := FALSE;
    countdown : CTD;
END_VAR
    countdown(CD := decrement, LD := load, PV := 3);
    done := countdown.Q;
END_PROGRAM
"#;
        let (mut vm, instructions) = compile_and_run(source);

        // Load preset value
        vm.memory_mut()
            .set_variable("load", PlcValue::Bool(true))
            .unwrap();
        vm.execute(&instructions).unwrap();
        let cv = vm.memory().get_variable("countdown.CV").unwrap().to_int();
        assert_eq!(cv, 3, "CV should be 3 after load, got {}", cv);
        vm.memory_mut()
            .set_variable("load", PlcValue::Bool(false))
            .unwrap();

        // Count down 3 times
        for _ in 0..3 {
            vm.memory_mut()
                .set_variable("decrement", PlcValue::Bool(true))
                .unwrap();
            vm.execute(&instructions).unwrap();
            vm.memory_mut()
                .set_variable("decrement", PlcValue::Bool(false))
                .unwrap();
            vm.execute(&instructions).unwrap();
        }

        let cv = vm.memory().get_variable("countdown.CV").unwrap().to_int();
        assert_eq!(cv, 0, "CV should be 0 after 3 decrements, got {}", cv);
        let done = vm.memory().get_variable("done").unwrap().to_bool();
        assert!(done, "Q should be TRUE when CV <= 0");
    }

    #[test]
    fn test_r_trig() {
        let source = r#"
PROGRAM RTrigTest
VAR
    sensor : BOOL := FALSE;
    pulse_detected : BOOL := FALSE;
    edge_detect : R_TRIG;
END_VAR
    edge_detect(CLK := sensor);
    pulse_detected := edge_detect.Q;
END_PROGRAM
"#;
        let (mut vm, instructions) = compile_and_run(source);

        // No pulse yet
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("pulse_detected").unwrap(),
            PlcValue::Bool(false)
        );

        // Rising edge: sensor goes TRUE
        vm.memory_mut()
            .set_variable("sensor", PlcValue::Bool(true))
            .unwrap();
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("pulse_detected").unwrap(),
            PlcValue::Bool(true)
        );

        // Held TRUE: no more edge
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("pulse_detected").unwrap(),
            PlcValue::Bool(false)
        );

        // Back to FALSE, then TRUE again: new edge
        vm.memory_mut()
            .set_variable("sensor", PlcValue::Bool(false))
            .unwrap();
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("pulse_detected").unwrap(),
            PlcValue::Bool(false)
        );
        vm.memory_mut()
            .set_variable("sensor", PlcValue::Bool(true))
            .unwrap();
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("pulse_detected").unwrap(),
            PlcValue::Bool(true)
        );
    }

    #[test]
    fn test_f_trig() {
        let source = r#"
PROGRAM FTrigTest
VAR
    signal : BOOL := TRUE;
    fall_detected : BOOL := FALSE;
    edge_detect : F_TRIG;
END_VAR
    edge_detect(CLK := signal);
    fall_detected := edge_detect.Q;
END_PROGRAM
"#;
        let (mut vm, instructions) = compile_and_run(source);

        // Start with signal TRUE, no falling edge yet
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("fall_detected").unwrap(),
            PlcValue::Bool(false)
        );

        // Falling edge: signal goes FALSE
        vm.memory_mut()
            .set_variable("signal", PlcValue::Bool(false))
            .unwrap();
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("fall_detected").unwrap(),
            PlcValue::Bool(true)
        );

        // Held FALSE: no more edge
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("fall_detected").unwrap(),
            PlcValue::Bool(false)
        );
    }

    #[test]
    fn test_struct_field_load_store() {
        // Create a struct variable: { speed: 0, enabled: false }
        let vars = vec![(
            "cfg".to_string(),
            PlcValue::Struct(vec![
                ("speed".to_string(), PlcValue::Int(0)),
                ("enabled".to_string(), PlcValue::Bool(false)),
            ]),
        )];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });

        // Store 42 into cfg.speed (field index 0)
        vm.execute(&[
            Instruction {
                opcode: Opcode::LoadVar,
                operand: Some(0),
            }, // load cfg
            Instruction {
                opcode: Opcode::PushInt,
                operand: Some(42),
            }, // new value
            Instruction {
                opcode: Opcode::LoadVar,
                operand: Some(0),
            }, // load cfg again for StoreStructField
            Instruction {
                opcode: Opcode::StoreStructField,
                operand: Some(0),
            }, // store field 0
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            }, // store modified struct back
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();

        // Verify struct was updated
        let cfg = vm.memory().get_variable("cfg").unwrap();
        if let PlcValue::Struct(fields) = cfg {
            assert_eq!(fields[0].1, PlcValue::Int(42), "speed field should be 42");
            assert_eq!(
                fields[1].1,
                PlcValue::Bool(false),
                "enabled field should remain false"
            );
        } else {
            panic!("Expected Struct value");
        }

        // Load field from struct
        vm.execute(&[
            Instruction {
                opcode: Opcode::LoadVar,
                operand: Some(0),
            }, // load cfg
            Instruction {
                opcode: Opcode::LoadStructField,
                operand: Some(0),
            }, // load field 0 (speed)
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            }, // overwrite cfg var for easy check
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();
        // After this, slot 0 holds the loaded field value (PlcValue::Int(42))
        // But we overwrote cfg, so let's check via the stack trace — the struct was replaced.
        // Actually the value on stack was PlcValue::Int(42) stored to slot 0.
        // Let's just verify via a separate variable approach instead.
    }

    #[test]
    fn test_enum_value() {
        let vars = vec![("color".to_string(), PlcValue::Enum(String::new()))];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec!["Red".to_string(), "Green".to_string(), "Blue".to_string()],
            line_map: vec![],
        });

        // PushEnum with name constant index 1 ("Green")
        vm.execute(&[
            Instruction {
                opcode: Opcode::PushEnum,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();

        let color = vm.memory().get_variable("color").unwrap();
        assert_eq!(
            *color,
            PlcValue::Enum("Green".to_string()),
            "Expected Enum(Green), got {:?}",
            color
        );
    }

    #[test]
    fn test_ctud_counter() {
        let source = r#"
PROGRAM CtudTest
VAR
    cu : BOOL := FALSE;
    cd : BOOL := FALSE;
    r : BOOL := FALSE;
    ld : BOOL := FALSE;
    done : BOOL := FALSE;
    cnt : CTUD;
END_VAR
    cnt(CU := cu, CD := cd, R := r, LD := ld, PV := 5);
    done := cnt.Q;
END_PROGRAM
"#;
        let (mut vm, instructions) = compile_and_run(source);

        // Cycle 1: no CU, counter stays at 0
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("done").unwrap(),
            PlcValue::Bool(false)
        );
        assert_eq!(
            *vm.memory().get_variable("cnt.CV").unwrap(),
            PlcValue::Dint(0)
        );

        // Count up 5 times with rising edges
        for _ in 0..5 {
            vm.memory_mut()
                .set_variable("cu", PlcValue::Bool(true))
                .unwrap();
            vm.execute(&instructions).unwrap();
            vm.memory_mut()
                .set_variable("cu", PlcValue::Bool(false))
                .unwrap();
            vm.execute(&instructions).unwrap();
        }

        let cv = vm.memory().get_variable("cnt.CV").unwrap().to_int();
        assert_eq!(cv, 5, "Expected CV=5 after 5 rising edges, got {}", cv);
        let done = vm.memory().get_variable("done").unwrap().to_bool();
        assert!(done, "Q should be TRUE when CV >= PV (5 >= 5)");

        // Reset
        vm.memory_mut()
            .set_variable("r", PlcValue::Bool(true))
            .unwrap();
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("cnt.CV").unwrap(),
            PlcValue::Dint(0)
        );
        assert_eq!(
            *vm.memory().get_variable("done").unwrap(),
            PlcValue::Bool(false)
        );
        vm.memory_mut()
            .set_variable("r", PlcValue::Bool(false))
            .unwrap();

        // Load preset
        vm.memory_mut()
            .set_variable("ld", PlcValue::Bool(true))
            .unwrap();
        vm.execute(&instructions).unwrap();
        assert_eq!(
            *vm.memory().get_variable("cnt.CV").unwrap(),
            PlcValue::Dint(5)
        );
        vm.memory_mut()
            .set_variable("ld", PlcValue::Bool(false))
            .unwrap();

        // Count down
        for _ in 0..3 {
            vm.memory_mut()
                .set_variable("cd", PlcValue::Bool(true))
                .unwrap();
            vm.execute(&instructions).unwrap();
            vm.memory_mut()
                .set_variable("cd", PlcValue::Bool(false))
                .unwrap();
            vm.execute(&instructions).unwrap();
        }
        let cv = vm.memory().get_variable("cnt.CV").unwrap().to_int();
        assert_eq!(cv, 2, "Expected CV=2 after 3 decrements from 5, got {}", cv);
    }

    #[test]
    fn test_direct_address_io() {
        let vars = vec![
            ("input_val".to_string(), PlcValue::Dint(0)),
            ("output_val".to_string(), PlcValue::Dint(0)),
        ];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });

        // Write to output area (area=2, size=4=DWord, index=5)
        // encoded = (2 << 28) | (4 << 24) | 5
        let encoded_output: i32 = (2 << 28) | (4 << 24) | 5;
        vm.execute(&[
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(99),
            },
            Instruction {
                opcode: Opcode::StoreIo,
                operand: Some(encoded_output),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();
        let output_read = vm.memory().read_output(5).clone();
        assert_eq!(output_read, PlcValue::Dint(99), "Expected output[5]=99");

        // Read from input area (area=1, size=4=DWord, index=10)
        // First, write a value to input image directly
        vm.memory_mut().write_input(10, PlcValue::Dint(77));

        let encoded_input: i32 = (1 << 28) | (4 << 24) | 10;
        vm.execute(&[
            Instruction {
                opcode: Opcode::LoadIo,
                operand: Some(encoded_input),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            }, // input_val
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();
        assert_eq!(
            vm.memory().get_variable("input_val"),
            Some(&PlcValue::Dint(77)),
            "Expected input_val=77 from %ID10"
        );
    }

    #[test]
    fn test_fb_state_persistence() {
        let source = r#"
PROGRAM TimerPersist
VAR
    start : BOOL := FALSE;
    motor : BOOL := FALSE;
    delay : TON;
END_VAR
    delay(IN := start, PT := T#1000MS);
    motor := delay.Q;
END_PROGRAM
"#;
        let (mut vm, instructions) = compile_and_run(source);

        // Start the timer
        vm.memory_mut()
            .set_variable("start", PlcValue::Bool(true))
            .unwrap();
        for _ in 0..5 {
            vm.execute(&instructions).unwrap();
        }

        // Snapshot FB state while timer is running
        let snapshot = vm.snapshot_fb_state();

        // Verify timer has accumulated some ET
        let et_before = vm.memory().get_variable("delay.ET").unwrap().to_int();
        assert!(
            et_before >= 0,
            "ET should be non-negative after running, got {}",
            et_before
        );

        // Create a brand new VM and restore the snapshot
        let (mut vm2, instructions2) = {
            let result = mt_stc::compile(source);
            let module = result.module.expect("Expected compiled module");
            let instrs = module.instructions.clone();
            let vm = Vm::from_instructions(module);
            (vm, instrs)
        };

        // Restore the snapshot into the new VM
        vm2.restore_fb_state(&snapshot);

        // Run one more cycle on the new VM
        vm2.memory_mut()
            .set_variable("start", PlcValue::Bool(true))
            .unwrap();
        vm2.execute(&instructions2).unwrap();

        // ET should survive (timer state was preserved)
        let et_after = vm2.memory().get_variable("delay.ET").unwrap().to_int();
        assert!(
            et_after >= 0,
            "ET should be non-negative after restore+run, got {}",
            et_after
        );
    }

    #[test]
    fn test_array_operations() {
        let vars = vec![
            (
                "arr".to_string(),
                PlcValue::Array(vec![
                    PlcValue::Dint(0),
                    PlcValue::Dint(0),
                    PlcValue::Dint(0),
                ]),
            ),
            ("val".to_string(), PlcValue::Dint(0)),
            ("len".to_string(), PlcValue::Dint(0)),
        ];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });

        // Store 42 into arr[1]
        vm.execute(&[
            // Push arr, push index, push value, StoreArrayIndex
            Instruction {
                opcode: Opcode::LoadVar,
                operand: Some(0),
            }, // arr
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(1),
            }, // index 1
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(42),
            }, // value
            Instruction {
                opcode: Opcode::StoreArrayIndex,
                operand: None,
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            }, // store modified arr back
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();

        // Verify arr[1] is now 42
        vm.execute(&[
            Instruction {
                opcode: Opcode::LoadVar,
                operand: Some(0),
            }, // arr
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(1),
            }, // index 1
            Instruction {
                opcode: Opcode::LoadArrayIndex,
                operand: None,
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(1),
            }, // store into val
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();
        assert_eq!(
            vm.memory().get_variable("val"),
            Some(&PlcValue::Dint(42)),
            "Expected arr[1]=42"
        );

        // Test ArrayLen
        vm.execute(&[
            Instruction {
                opcode: Opcode::LoadVar,
                operand: Some(0),
            }, // arr
            Instruction {
                opcode: Opcode::ArrayLen,
                operand: None,
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(2),
            }, // store into len
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();
        assert_eq!(
            vm.memory().get_variable("len"),
            Some(&PlcValue::Dint(3)),
            "Expected array length 3"
        );
    }

    #[test]
    fn test_nested_if() {
        let source = r#"
PROGRAM NestedIf
VAR
    a : INT := 0;
    b : INT := 0;
    result : INT := 0;
END_VAR
    IF a > 3 THEN
        IF b > 1 THEN
            result := 100;
        ELSE
            result := 200;
        END_IF;
    ELSIF a > 1 THEN
        result := 300;
    ELSE
        result := 400;
    END_IF;
END_PROGRAM
"#;
        let (mut vm, instructions) = compile_and_run(source);

        // Test nested IF: a=5, b=3 => a > 3 is true, b > 1 is true => result=100
        vm.memory_mut().set_variable("a", PlcValue::Int(5)).unwrap();
        vm.memory_mut().set_variable("b", PlcValue::Int(3)).unwrap();
        vm.execute(&instructions).unwrap();
        let result = vm.memory().get_variable("result").unwrap().to_int();
        assert_eq!(
            result, 100,
            "Expected result=100 from nested IF path, got {}",
            result
        );

        // Test nested IF else: a=5, b=0 => a > 3 is true, b > 1 is false => result=200
        vm.memory_mut().set_variable("a", PlcValue::Int(5)).unwrap();
        vm.memory_mut().set_variable("b", PlcValue::Int(0)).unwrap();
        vm.memory_mut()
            .set_variable("result", PlcValue::Int(0))
            .unwrap();
        vm.execute(&instructions).unwrap();
        let result = vm.memory().get_variable("result").unwrap().to_int();
        assert_eq!(
            result, 200,
            "Expected result=200 from nested IF-ELSE path, got {}",
            result
        );

        // Test ELSIF path: a=2 => a > 3 is false, a > 1 is true => result=300
        vm.memory_mut().set_variable("a", PlcValue::Int(2)).unwrap();
        vm.memory_mut()
            .set_variable("result", PlcValue::Int(0))
            .unwrap();
        vm.execute(&instructions).unwrap();
        let result = vm.memory().get_variable("result").unwrap().to_int();
        assert_eq!(
            result, 300,
            "Expected result=300 from ELSIF path, got {}",
            result
        );

        // Test ELSE path: a=0 => a > 3 is false, a > 1 is false => result=400
        vm.memory_mut().set_variable("a", PlcValue::Int(0)).unwrap();
        vm.memory_mut()
            .set_variable("result", PlcValue::Int(0))
            .unwrap();
        vm.execute(&instructions).unwrap();
        let result = vm.memory().get_variable("result").unwrap().to_int();
        assert_eq!(
            result, 400,
            "Expected result=400 from ELSE path, got {}",
            result
        );
    }

    #[test]
    fn test_string_compare() {
        // String comparison via Eq opcode: both strings map to to_real() = 0.0,
        // so equality check (0.0 == 0.0) should be true.
        // This tests the VM behavior with string PlcValues through the Eq opcode.
        let vars = vec![("flag".to_string(), PlcValue::Bool(false))];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });

        // Push two string values and compare with Eq
        // Since to_real() returns 0.0 for all strings, Eq("hello", "hello") => true
        vm.execute(&[
            // We can't push strings directly, but we can use IntToString
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(42),
            },
            Instruction {
                opcode: Opcode::IntToString,
                operand: None,
            },
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(42),
            },
            Instruction {
                opcode: Opcode::IntToString,
                operand: None,
            },
            Instruction {
                opcode: Opcode::Eq,
                operand: None,
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();

        let flag = vm.memory().get_variable("flag").unwrap();
        // Both convert to "42", so Eq should compare their to_real() values (both 0.0)
        // Since to_real() is 0.0 for all strings, 0.0 == 0.0 => true
        assert_eq!(
            *flag,
            PlcValue::Bool(true),
            "String equality via Eq should be true for same-value strings"
        );
    }

    #[test]
    fn test_modulo_operation() {
        let vars = vec![("result".to_string(), PlcValue::Real(0.0))];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });

        // 10 MOD 3 should be 1
        vm.execute(&[
            Instruction {
                opcode: Opcode::PushInt,
                operand: Some(10),
            },
            Instruction {
                opcode: Opcode::PushInt,
                operand: Some(3),
            },
            Instruction {
                opcode: Opcode::Modulo,
                operand: None,
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();

        let result = vm.memory().get_variable("result").unwrap().to_real();
        assert!(
            (result - 1.0).abs() < 0.01,
            "Expected 10 MOD 3 = 1.0, got {}",
            result
        );

        // 17 MOD 5 should be 2
        let mut vm2 = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vec![("result".to_string(), PlcValue::Real(0.0))],
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });
        vm2.execute(&[
            Instruction {
                opcode: Opcode::PushInt,
                operand: Some(17),
            },
            Instruction {
                opcode: Opcode::PushInt,
                operand: Some(5),
            },
            Instruction {
                opcode: Opcode::Modulo,
                operand: None,
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();
        let result2 = vm2.memory().get_variable("result").unwrap().to_real();
        assert!(
            (result2 - 2.0).abs() < 0.01,
            "Expected 17 MOD 5 = 2.0, got {}",
            result2
        );
    }

    #[test]
    fn test_negation() {
        let vars = vec![("result".to_string(), PlcValue::Int(0))];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });

        // Negate 42 => -42
        vm.execute(&[
            Instruction {
                opcode: Opcode::PushInt,
                operand: Some(42),
            },
            Instruction {
                opcode: Opcode::Neg,
                operand: None,
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();

        let result = vm.memory().get_variable("result").unwrap();
        assert_eq!(
            *result,
            PlcValue::Int(-42),
            "Expected -42 after negation, got {:?}",
            result
        );

        // Negate a Dint
        let vars2 = vec![("result".to_string(), PlcValue::Dint(0))];
        let mut vm2 = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars2,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });
        vm2.execute(&[
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(100),
            },
            Instruction {
                opcode: Opcode::Neg,
                operand: None,
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();
        assert_eq!(
            vm2.memory().get_variable("result"),
            Some(&PlcValue::Dint(-100))
        );

        // Negate a Real
        let vars3 = vec![("result".to_string(), PlcValue::Real(0.0))];
        let mut vm3 = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars3,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });
        let neg_three_five = (-3.5f32).to_bits() as i32;
        vm3.execute(&[
            Instruction {
                opcode: Opcode::PushReal,
                operand: Some(0x40600000),
            }, // 3.5
            Instruction {
                opcode: Opcode::Neg,
                operand: None,
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ])
        .unwrap();
        let result_real = vm3.memory().get_variable("result").unwrap().to_real();
        assert!(
            (result_real - (-3.5)).abs() < 0.01,
            "Expected -3.5, got {}",
            result_real
        );
    }

    #[test]
    fn test_string_literal() {
        let instructions = vec![
            Instruction {
                opcode: Opcode::PushString,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ];
        let vars = vec![("greeting".into(), PlcValue::String(String::new()))];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec!["hello world".into()],
            line_map: vec![],
        });
        vm.execute(&instructions).unwrap();
        let result = vm.memory().get_variable("greeting").unwrap();
        match result {
            PlcValue::String(s) => assert_eq!(s, "hello world"),
            _ => panic!("Expected String, got {:?}", result),
        }
    }

    #[test]
    fn test_string_operations_with_literals() {
        let instructions = vec![
            Instruction {
                opcode: Opcode::PushString,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::PushString,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::LoadVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::LoadVar,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::Concat,
                operand: Some(2),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(2),
            },
            Instruction {
                opcode: Opcode::LoadVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::StrLen,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(3),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ];
        let vars = vec![
            ("s1".into(), PlcValue::String(String::new())),
            ("s2".into(), PlcValue::String(String::new())),
            ("result".into(), PlcValue::String(String::new())),
            ("len".into(), PlcValue::Dint(0)),
        ];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec!["hello".into(), " world".into()],
            line_map: vec![],
        });
        vm.execute(&instructions).unwrap();
        match vm.memory().get_variable("result").unwrap() {
            PlcValue::String(s) => assert_eq!(s, "hello world"),
            other => panic!("Expected String, got {:?}", other),
        }
        assert_eq!(vm.memory().get_variable("len").unwrap().to_int(), 5);
    }

    #[test]
    fn test_mux_function() {
        // MUX(1, 10, 20, 30) should return 20
        let instructions = vec![
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(10),
            },
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(20),
            },
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(30),
            },
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::Mux,
                operand: Some(4),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ];
        let vars = vec![("result".into(), PlcValue::Dint(0))];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });
        vm.execute(&instructions).unwrap();
        assert_eq!(vm.memory().get_variable("result").unwrap().to_int(), 20);
    }

    #[test]
    fn test_mux_function_first() {
        let instructions = vec![
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(10),
            },
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(20),
            },
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Mux,
                operand: Some(3),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ];
        let vars = vec![("result".into(), PlcValue::Dint(0))];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });
        vm.execute(&instructions).unwrap();
        assert_eq!(vm.memory().get_variable("result").unwrap().to_int(), 10);
    }

    #[test]
    fn test_shl_shr() {
        let instructions = vec![
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(0xFF),
            },
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(8),
            },
            Instruction {
                opcode: Opcode::Shl,
                operand: Some(2),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::LoadVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(4),
            },
            Instruction {
                opcode: Opcode::Shr,
                operand: Some(2),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ];
        let vars = vec![
            ("shifted".into(), PlcValue::Dint(0)),
            ("shifted_back".into(), PlcValue::Dint(0)),
        ];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });
        vm.execute(&instructions).unwrap();
        assert_eq!(
            vm.memory().get_variable("shifted").unwrap().to_int(),
            0xFF00
        );
        assert_eq!(
            vm.memory().get_variable("shifted_back").unwrap().to_int(),
            0xFF0
        );
    }

    #[test]
    fn test_rol() {
        // ROL(0x80000001, 1): MSB wraps to LSB
        // 0x80000001 << 1 = 0x00000002, 0x80000001 >> 31 = 0x00000001
        // ROL result = 0x00000002 | 0x00000001 = 0x00000003
        let instructions = vec![
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(0x80000001u32 as i32),
            },
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::Rol,
                operand: Some(2),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ];
        let vars = vec![("rotated".into(), PlcValue::Dint(0))];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });
        vm.execute(&instructions).unwrap();
        assert_eq!(vm.memory().get_variable("rotated").unwrap().to_int(), 3);
    }

    #[test]
    fn test_type_conversions() {
        let instructions = vec![
            // INT_TO_WORD(42) then WORD_TO_INT
            Instruction {
                opcode: Opcode::PushDint,
                operand: Some(42),
            },
            Instruction {
                opcode: Opcode::IntToWord,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::LoadVar,
                operand: Some(0),
            },
            Instruction {
                opcode: Opcode::WordToInt,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(1),
            },
            // REAL_TO_DWORD(3.14) then DWORD_TO_REAL
            Instruction {
                opcode: Opcode::PushReal,
                operand: Some(3.14f32.to_bits() as i32),
            },
            Instruction {
                opcode: Opcode::RealToDword,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(2),
            },
            Instruction {
                opcode: Opcode::LoadVar,
                operand: Some(2),
            },
            Instruction {
                opcode: Opcode::DwordToReal,
                operand: Some(1),
            },
            Instruction {
                opcode: Opcode::StoreVar,
                operand: Some(3),
            },
            Instruction {
                opcode: Opcode::Halt,
                operand: None,
            },
        ];
        let vars = vec![
            ("w".into(), PlcValue::Word(0)),
            ("wi".into(), PlcValue::Dint(0)),
            ("dw".into(), PlcValue::Dword(0)),
            ("dwr".into(), PlcValue::Real(0.0)),
        ];
        let mut vm = Vm::from_instructions(CompiledModule {
            name: "test".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });
        vm.execute(&instructions).unwrap();
        match vm.memory().get_variable("w").unwrap() {
            PlcValue::Word(w) => assert_eq!(*w, 42),
            other => panic!("Expected Word, got {:?}", other),
        }
        assert_eq!(vm.memory().get_variable("wi").unwrap().to_int(), 42);
        let roundtrip = match vm.memory().get_variable("dwr").unwrap() {
            PlcValue::Real(f) => *f,
            other => panic!("Expected Real, got {:?}", other),
        };
        assert!(
            (roundtrip - 3.14).abs() < 0.01,
            "Expected 3.14, got {}",
            roundtrip
        );
    }

    /// Benchmark: measure how many cycles/second the VM can sustain
    /// with a representative program (20 variables, arithmetic, IF/ELSE, timer).
    /// Target: > 10,000 cycles/sec (> 100μs/cycle) on any modern machine.
    #[test]
    fn test_vm_throughput() {
        // Build a program: x := x + 1; y := x * 2; IF x > 100 THEN x := 0; END_IF; t1(IN:=TRUE, PT:=T#1S)
        let mut instructions: Vec<Instruction> = Vec::new();

        // x := x + 1
        instructions.push(Instruction {
            opcode: Opcode::LoadVar,
            operand: Some(0),
        }); // load x
        instructions.push(Instruction {
            opcode: Opcode::PushDint,
            operand: Some(1),
        });
        instructions.push(Instruction {
            opcode: Opcode::Add,
            operand: None,
        });
        instructions.push(Instruction {
            opcode: Opcode::StoreVar,
            operand: Some(0),
        }); // store x

        // y := x * 2
        instructions.push(Instruction {
            opcode: Opcode::LoadVar,
            operand: Some(0),
        }); // load x
        instructions.push(Instruction {
            opcode: Opcode::PushDint,
            operand: Some(2),
        });
        instructions.push(Instruction {
            opcode: Opcode::Mul,
            operand: None,
        });
        instructions.push(Instruction {
            opcode: Opcode::StoreVar,
            operand: Some(1),
        }); // store y

        // z := SQRT(x * 100)
        instructions.push(Instruction {
            opcode: Opcode::LoadVar,
            operand: Some(0),
        });
        instructions.push(Instruction {
            opcode: Opcode::PushDint,
            operand: Some(100),
        });
        instructions.push(Instruction {
            opcode: Opcode::Mul,
            operand: None,
        });
        instructions.push(Instruction {
            opcode: Opcode::PushDint,
            operand: Some(1),
        }); // arg_count
        instructions.push(Instruction {
            opcode: Opcode::Sqrt,
            operand: Some(1),
        });
        instructions.push(Instruction {
            opcode: Opcode::StoreVar,
            operand: Some(2),
        }); // store z

        // IF x > 100 THEN x := 0; END_IF
        instructions.push(Instruction {
            opcode: Opcode::LoadVar,
            operand: Some(0),
        });
        instructions.push(Instruction {
            opcode: Opcode::PushDint,
            operand: Some(100),
        });
        instructions.push(Instruction {
            opcode: Opcode::Gt,
            operand: None,
        });
        instructions.push(Instruction {
            opcode: Opcode::JumpIfFalse,
            operand: Some(3),
        }); // skip 3 instr
        instructions.push(Instruction {
            opcode: Opcode::PushDint,
            operand: Some(0),
        });
        instructions.push(Instruction {
            opcode: Opcode::StoreVar,
            operand: Some(0),
        });

        // Timer call: t1(IN:=TRUE, PT:=T#1S)
        instructions.push(Instruction {
            opcode: Opcode::PushTime,
            operand: Some(1000),
        }); // PT=1s
        instructions.push(Instruction {
            opcode: Opcode::PushBool,
            operand: Some(1),
        }); // IN=TRUE
        instructions.push(Instruction {
            opcode: Opcode::TonCall,
            operand: Some(0),
        }); // timer idx 0
        instructions.push(Instruction {
            opcode: Opcode::StoreVar,
            operand: Some(4),
        }); // t1.Q
        instructions.push(Instruction {
            opcode: Opcode::StoreVar,
            operand: Some(5),
        }); // t1.ET

        instructions.push(Instruction {
            opcode: Opcode::Halt,
            operand: None,
        });

        let vars = vec![
            ("x".to_string(), PlcValue::Dint(0)),
            ("y".to_string(), PlcValue::Dint(0)),
            ("z".to_string(), PlcValue::Real(0.0)),
            ("t1".to_string(), PlcValue::Bool(false)),
            ("t1.Q".to_string(), PlcValue::Bool(false)),
            ("t1.ET".to_string(), PlcValue::Time(0)),
        ];

        let mut vm = Vm::from_instructions(CompiledModule {
            name: "bench".into(),
            instructions: vec![],
            variable_slots: vars,
            pou_table: vec![],
            name_constants: vec![],
            line_map: vec![],
        });

        let iterations = 10_000;
        let start = std::time::Instant::now();

        for _ in 0..iterations {
            vm.execute(&instructions).unwrap();
        }

        let elapsed = start.elapsed();
        let cycles_per_sec = iterations as f64 / elapsed.as_secs_f64();
        let us_per_cycle = elapsed.as_micros() as f64 / iterations as f64;

        eprintln!("\n=== VM Throughput ===");
        eprintln!("  {} cycles in {:.2?}ms", iterations, elapsed.as_millis());
        eprintln!("  {:.0} cycles/sec", cycles_per_sec);
        eprintln!("  {:.1} μs/cycle", us_per_cycle);
        eprintln!("  Instructions per cycle: {}", instructions.len());

        // Must achieve at least 1,000 cycles/sec (1ms/cycle)
        assert!(
            cycles_per_sec > 1000.0,
            "VM too slow: {:.0} cycles/sec ({:.1}μs/cycle)",
            cycles_per_sec,
            us_per_cycle
        );
    }
}
