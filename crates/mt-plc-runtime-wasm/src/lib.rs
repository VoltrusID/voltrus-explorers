use wasm_bindgen::prelude::*;

use mt_plc_runtime::{PlcRuntime, RuntimeConfig};
use mt_plc_types::{CompiledModule, PlcValue};

fn err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

fn to_json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// Debugger facade over `PlcRuntime` for the Voltrus cloud IDE.
/// Complex payloads cross the boundary as JSON strings.
#[wasm_bindgen]
pub struct WasmPlcRuntime {
    runtime: PlcRuntime,
}

#[wasm_bindgen]
impl WasmPlcRuntime {
    #[wasm_bindgen(constructor)]
    pub fn new(module_json: String, config_json: String) -> Result<WasmPlcRuntime, JsValue> {
        let module: CompiledModule = serde_json::from_str(&module_json)
            .map_err(|e| err(format!("Invalid module JSON: {}", e)))?;
        let config: RuntimeConfig = serde_json::from_str(&config_json)
            .map_err(|e| err(format!("Invalid config JSON: {}", e)))?;
        Ok(WasmPlcRuntime {
            runtime: PlcRuntime::new(config, module),
        })
    }

    pub fn execute_cycle(&mut self) -> Result<String, JsValue> {
        self.runtime.execute_cycle().map(|s| to_json(&s)).map_err(err)
    }

    pub fn step_instruction(&mut self) -> Result<String, JsValue> {
        self.runtime.step_instruction().map(|s| to_json(&s)).map_err(err)
    }

    pub fn continue_execution(&mut self) -> Result<String, JsValue> {
        self.runtime
            .continue_from_breakpoint()
            .map(|s| to_json(&s))
            .map_err(err)
    }

    pub fn set_breakpoint_line(&mut self, line: usize) {
        self.runtime.set_breakpoint_line(line);
    }

    pub fn clear_breakpoint_line(&mut self, line: usize) {
        self.runtime.clear_breakpoint_line(line);
    }

    pub fn clear_all_breakpoints(&mut self) {
        self.runtime.clear_all_breakpoints();
    }

    /// Source line of the program counter, or `undefined` when unknown.
    pub fn current_pc_line(&self) -> Option<usize> {
        self.runtime.current_pc_line()
    }

    /// Effective value of a variable as PlcValue JSON, or `undefined` if absent.
    pub fn read_variable(&self, name: &str) -> Option<String> {
        self.runtime.read_variable(name).map(to_json)
    }

    pub fn write_variable(&mut self, name: &str, value_json: &str) -> Result<(), JsValue> {
        let value: PlcValue = serde_json::from_str(value_json)
            .map_err(|e| err(format!("Invalid value JSON: {}", e)))?;
        self.runtime.write_variable(name, value).map_err(err)
    }

    pub fn force_variable(&mut self, name: &str, value_json: &str) -> Result<(), JsValue> {
        let value: PlcValue = serde_json::from_str(value_json)
            .map_err(|e| err(format!("Invalid value JSON: {}", e)))?;
        self.runtime.force_variable(name, value).map_err(err)
    }

    pub fn unforce_variable(&mut self, name: &str) {
        self.runtime.unforce_variable(name);
    }

    /// All variables with type/quality info as `Vec<WatchedVariable>` JSON.
    pub fn watch_variables(&self) -> String {
        to_json(&self.runtime.watch_variables())
    }

    /// Variable values as `Vec<(String, PlcValue)>` JSON for persistence.
    pub fn snapshot_variables(&self) -> String {
        to_json(&self.runtime.snapshot_variables())
    }

    pub fn restore_variables(&mut self, snapshot_json: &str) -> Result<(), JsValue> {
        let snapshot: Vec<(String, PlcValue)> = serde_json::from_str(snapshot_json)
            .map_err(|e| err(format!("Invalid snapshot JSON: {}", e)))?;
        self.runtime.restore_variables(&snapshot);
        Ok(())
    }

    pub fn cycle_count(&self) -> u64 {
        self.runtime.cycle_count()
    }

    /// Current runtime state as `RuntimeState` JSON.
    pub fn state(&self) -> String {
        to_json(self.runtime.state())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COUNTER_ST: &str = r#"
        PROGRAM Main
        VAR
            counter : INT;
            enable : BOOL;
        END_VAR
            enable := TRUE;
            counter := counter + 1;
        END_PROGRAM
    "#;

    fn make_runtime() -> (WasmPlcRuntime, CompiledModule) {
        let result = mt_stc::compile(COUNTER_ST);
        assert!(result.success, "Compile failed: {:?}", result.diagnostics);
        let module = result.module.expect("Expected compiled module");
        let runtime = WasmPlcRuntime::new(
            serde_json::to_string(&module).unwrap(),
            serde_json::to_string(&RuntimeConfig::default()).unwrap(),
        )
        .expect("runtime constructed");
        (runtime, module)
    }

    #[test]
    fn cycles_execute_and_count() {
        let (mut rt, _) = make_runtime();
        assert_eq!(rt.state(), "\"Stopped\"");
        for _ in 0..5 {
            let stats: serde_json::Value =
                serde_json::from_str(&rt.execute_cycle().expect("cycle ok")).unwrap();
            assert!(stats["last_cycle_ms"].as_f64().unwrap() >= 0.0);
            assert!(stats["cycle_count"].as_u64().unwrap() >= 1);
        }
        assert_eq!(rt.cycle_count(), 5);
        assert_eq!(rt.state(), "\"Running\"");
    }

    #[test]
    fn write_and_read_variable_roundtrip() {
        let (mut rt, _) = make_runtime();
        rt.execute_cycle().unwrap();
        rt.write_variable("counter", r#"{"Int":10}"#).unwrap();
        assert_eq!(rt.read_variable("counter").as_deref(), Some(r#"{"Int":10}"#));
        assert_eq!(rt.read_variable("nope"), None);
    }

    #[test]
    fn force_pins_variable_against_program_writes() {
        let (mut rt, _) = make_runtime();
        for _ in 0..3 {
            rt.execute_cycle().unwrap();
        }
        assert_eq!(rt.read_variable("counter").as_deref(), Some(r#"{"Real":3.0}"#));

        rt.force_variable("counter", r#"{"Int":42}"#).unwrap();
        rt.execute_cycle().unwrap();
        let watch: Vec<serde_json::Value> =
            serde_json::from_str(&rt.watch_variables()).unwrap();
        let counter = watch
            .iter()
            .find(|w| w["name"] == "counter")
            .expect("counter in watch list");
        assert_eq!(counter["quality"], "forced");
        assert_eq!(counter["value"], serde_json::json!({"Int": 42}));

        rt.unforce_variable("counter");
        rt.execute_cycle().unwrap();
        assert_eq!(rt.read_variable("counter").as_deref(), Some(r#"{"Real":4.0}"#));
    }

    #[test]
    fn breakpoint_step_and_continue() {
        let (mut rt, module) = make_runtime();
        let bp_line = module.line_map[0].1;

        rt.set_breakpoint_line(bp_line);
        let state: serde_json::Value =
            serde_json::from_str(&rt.step_instruction().expect("step ok")).unwrap();
        assert!(state.get("BreakpointHit").is_some(), "expected BreakpointHit, got {}", state);
        assert_eq!(rt.current_pc_line(), Some(bp_line));

        rt.clear_all_breakpoints();
        let state = rt.continue_execution().expect("continue ok");
        assert_eq!(state, "\"Done\"");
    }

    #[test]
    fn snapshot_restore_roundtrip() {
        let (mut rt, _) = make_runtime();
        rt.execute_cycle().unwrap();

        let snapshot = rt.snapshot_variables();
        let parsed: Vec<(String, serde_json::Value)> =
            serde_json::from_str(&snapshot).unwrap();
        assert!(parsed.iter().any(|(n, _)| n == "counter"));

        rt.write_variable("counter", r#"{"Int":99}"#).unwrap();
        rt.restore_variables(&snapshot).unwrap();
        assert_eq!(rt.read_variable("counter").as_deref(), Some(r#"{"Real":1.0}"#));
    }
}
