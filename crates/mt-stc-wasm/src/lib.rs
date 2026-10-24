use mt_plc_types::CompiledModule;
use mt_stc::{compile, compile_project};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct WasmCompileOutput {
    success: bool,
    diagnostics_json: String,
    pou_count: usize,
    variable_count: usize,
    instructions_json: String,
    variables_json: String,
    line_map_json: String,
    module_json: String,
}

#[wasm_bindgen]
impl WasmCompileOutput {
    #[wasm_bindgen(getter)]
    pub fn success(&self) -> bool {
        self.success
    }

    #[wasm_bindgen(getter)]
    pub fn diagnostics(&self) -> String {
        self.diagnostics_json.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn pou_count(&self) -> usize {
        self.pou_count
    }

    #[wasm_bindgen(getter)]
    pub fn variable_count(&self) -> usize {
        self.variable_count
    }

    #[wasm_bindgen(getter)]
    pub fn instructions(&self) -> String {
        self.instructions_json.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn variables(&self) -> String {
        self.variables_json.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn line_map(&self) -> String {
        self.line_map_json.clone()
    }

    /// Full module as serde `CompiledModule` JSON ("" when compilation produced
    /// no module). This is the shape `WasmPlcRuntime::new` consumes.
    #[wasm_bindgen(getter)]
    pub fn module_json(&self) -> String {
        self.module_json.clone()
    }
}

#[derive(serde::Serialize)]
struct InstructionJson {
    opcode: String,
    operand: Option<i32>,
}

#[derive(serde::Serialize)]
struct VariableJson {
    name: String,
    value: String,
    type_name: String,
}

fn serialize_module(module: &CompiledModule) -> (String, String, String) {
    let instructions: Vec<InstructionJson> = module
        .instructions
        .iter()
        .map(|i| InstructionJson {
            opcode: format!("{:?}", i.opcode),
            operand: i.operand,
        })
        .collect();

    let variables: Vec<VariableJson> = module
        .variable_slots
        .iter()
        .map(|(name, val)| VariableJson {
            name: name.clone(),
            value: format!("{:?}", val),
            type_name: val.type_name().to_string(),
        })
        .collect();

    let instructions_json = serde_json::to_string(&instructions).unwrap_or_default();
    let variables_json = serde_json::to_string(&variables).unwrap_or_default();
    let line_map_json = serde_json::to_string(&module.line_map).unwrap_or_default();

    (instructions_json, variables_json, line_map_json)
}

#[wasm_bindgen]
pub fn wasm_compile(source: &str) -> WasmCompileOutput {
    let result = compile(source);

    let (instructions_json, variables_json, line_map_json) = result
        .module
        .as_ref()
        .map(serialize_module)
        .unwrap_or_default();
    let module_json = result
        .module
        .as_ref()
        .map(|m| serde_json::to_string(m).unwrap_or_default())
        .unwrap_or_default();

    WasmCompileOutput {
        success: result.success,
        diagnostics_json: serde_json::to_string(&result.diagnostics).unwrap_or_default(),
        pou_count: result.pou_count,
        variable_count: result.variable_count,
        instructions_json,
        variables_json,
        line_map_json,
        module_json,
    }
}

#[wasm_bindgen]
pub fn wasm_compile_project(sources_json: &str) -> Result<WasmCompileOutput, JsValue> {
    let sources: Vec<(String, String)> = serde_json::from_str(sources_json)
        .map_err(|e| JsValue::from_str(&format!("Invalid sources JSON: {}", e)))?;

    let result = compile_project(&sources);

    let (instructions_json, variables_json, line_map_json) = result
        .module
        .as_ref()
        .map(serialize_module)
        .unwrap_or_default();
    let module_json = result
        .module
        .as_ref()
        .map(|m| serde_json::to_string(m).unwrap_or_default())
        .unwrap_or_default();

    Ok(WasmCompileOutput {
        success: result.success,
        diagnostics_json: serde_json::to_string(&result.diagnostics).unwrap_or_default(),
        pou_count: result.pou_count,
        variable_count: result.variable_count,
        instructions_json,
        variables_json,
        line_map_json,
        module_json,
    })
}

#[wasm_bindgen]
pub fn wasm_standard_functions() -> String {
    let funcs: Vec<&str> = mt_stc::stdlib::standard_functions()
        .iter()
        .map(|f| f.name)
        .collect();
    serde_json::to_string(&funcs).unwrap_or_default()
}
