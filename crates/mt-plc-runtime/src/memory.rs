use crate::types::PlcValue;

pub struct MemoryModel {
    /// Named variables with their slot indices
    variables: Vec<(String, PlcValue)>,
    /// Forced variable flags
    forced: Vec<bool>,
    /// Forced values
    forced_values: Vec<Option<PlcValue>>,
    /// Process image input buffer (%I area)
    input_image: Vec<PlcValue>,
    /// Process image output buffer (%Q area)
    output_image: Vec<PlcValue>,
    /// Memory area buffer (%M area)
    memory_image: Vec<PlcValue>,
}

impl MemoryModel {
    pub fn new(variable_names: Vec<(String, PlcValue)>) -> Self {
        let len = variable_names.len();
        Self {
            variables: variable_names,
            forced: vec![false; len],
            forced_values: vec![None; len],
            input_image: vec![PlcValue::Dint(0); 256],
            output_image: vec![PlcValue::Dint(0); 256],
            memory_image: vec![PlcValue::Dint(0); 256],
        }
    }

    pub fn get_variable(&self, name: &str) -> Option<&PlcValue> {
        self.variables
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }

    pub fn get_variable_index(&self, name: &str) -> Option<usize> {
        self.variables.iter().position(|(n, _)| n == name)
    }

    pub fn set_variable(&mut self, name: &str, value: PlcValue) -> Result<(), String> {
        if let Some(slot) = self.variables.iter().position(|(n, _)| n == name) {
            if self.forced.get(slot).copied().unwrap_or(false) {
                return Ok(()); // Skip writes to forced variables
            }
            self.variables[slot].1 = value;
            Ok(())
        } else {
            Err(format!("Variable '{}' not found", name))
        }
    }

    pub fn get_by_index(&self, index: usize) -> Option<&PlcValue> {
        if index < self.variables.len() {
            if self.forced.get(index).copied().unwrap_or(false) {
                return self.forced_values.get(index).and_then(|v| v.as_ref());
            }
            Some(&self.variables[index].1)
        } else {
            None
        }
    }

    pub fn set_by_index(&mut self, index: usize, value: PlcValue) -> Result<(), String> {
        if index >= self.variables.len() {
            return Err(format!("Variable index {} out of range", index));
        }
        if self.forced.get(index).copied().unwrap_or(false) {
            return Ok(()); // Skip writes to forced variables
        }
        self.variables[index].1 = value;
        Ok(())
    }

    pub fn force_variable(&mut self, name: &str, value: PlcValue) -> Result<(), String> {
        let index = self
            .variables
            .iter()
            .position(|(n, _)| n == name)
            .ok_or_else(|| format!("Variable '{}' not found", name))?;
        self.forced[index] = true;
        self.forced_values[index] = Some(value);
        Ok(())
    }

    pub fn unforce_variable(&mut self, name: &str) {
        if let Some(index) = self.variables.iter().position(|(n, _)| n == name) {
            self.forced[index] = false;
            self.forced_values[index] = None;
        }
    }

    pub fn read_input(&self, address: usize) -> &PlcValue {
        self.input_image
            .get(address)
            .unwrap_or(&PlcValue::Bool(false))
    }

    pub fn write_output(&mut self, address: usize, value: PlcValue) {
        if address < self.output_image.len() {
            self.output_image[address] = value;
        }
    }

    pub fn read_output(&self, address: usize) -> &PlcValue {
        self.output_image
            .get(address)
            .unwrap_or(&PlcValue::Bool(false))
    }

    pub fn write_input(&mut self, address: usize, value: PlcValue) {
        if address < self.input_image.len() {
            self.input_image[address] = value;
        }
    }

    pub fn read_memory(&self, address: usize) -> &PlcValue {
        self.memory_image
            .get(address)
            .unwrap_or(&PlcValue::Bool(false))
    }

    pub fn write_memory(&mut self, address: usize, value: PlcValue) {
        if address < self.memory_image.len() {
            self.memory_image[address] = value;
        }
    }

    pub fn variables_snapshot(&self) -> Vec<(String, PlcValue)> {
        self.variables.clone()
    }

    pub fn variable_names(&self) -> Vec<String> {
        self.variables.iter().map(|(n, _)| n.clone()).collect()
    }

    pub fn get_watch_info(&self, name: &str) -> (PlcValue, String, String) {
        if let Some((_, value)) = self.variables.iter().find(|(n, _)| n == name) {
            let index = self.variables.iter().position(|(n, _)| n == name).unwrap();
            let is_forced = self.forced.get(index).copied().unwrap_or(false);
            let actual_value = if is_forced {
                self.forced_values
                    .get(index)
                    .and_then(|v| v.clone())
                    .unwrap_or_else(|| value.clone())
            } else {
                value.clone()
            };
            let quality = if is_forced { "forced" } else { "good" };
            (
                actual_value,
                value.type_name().to_string(),
                quality.to_string(),
            )
        } else {
            (PlcValue::Dint(0), "UNKNOWN".to_string(), "bad".to_string())
        }
    }
}
