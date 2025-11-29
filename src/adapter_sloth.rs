//! SLOTH adapter - a dummy model that provides constant values.
//!
//! SLOTH (Simple Lookup Of Transformed Hydrological values) is used to provide
//! constant values or pass-through values in a model chain. Instead of loading
//! an actual shared library, this adapter just stores and returns configured values.
//!
//! The model_params format is: "varname(count,type,units,location)": "value"
//! For example: "rain(1,double,mm/s,node)": "0.0002"

use std::collections::HashMap;

use crate::error::{BmiError, BmiResult};
use crate::traits::{Bmi, VarType};

/// Variable information parsed from the SLOTH config format.
#[derive(Debug, Clone)]
struct SlothVar {
    name: String,
    count: usize,
    var_type: VarType,
    type_str: String,
    units: String,
    location: String,
    value: f64,
}

/// SLOTH dummy model adapter.
///
/// Provides constant values configured via model_params without loading
/// any shared library.
pub struct BmiSloth {
    model_name: String,
    initialized: bool,
    variables: HashMap<String, SlothVar>,
    var_names: Vec<String>,
    var_type_cache: Option<HashMap<String, VarType>>,
    current_time: f64,
    time_step: f64,
    end_time: f64,
}

impl BmiSloth {
    /// Create a new SLOTH model.
    pub fn new(model_name: impl Into<String>) -> Self {
        Self {
            model_name: model_name.into(),
            initialized: false,
            variables: HashMap::new(),
            var_names: Vec::new(),
            var_type_cache: None,
            current_time: 0.0,
            time_step: 3600.0,
            end_time: f64::MAX,
        }
    }

    /// Configure the SLOTH model with parameters.
    ///
    /// Parameters should be in the format: "varname(count,type,units,location)": "value"
    pub fn configure(&mut self, params: &HashMap<String, String>) -> BmiResult<()> {
        self.variables.clear();
        self.var_names.clear();

        for (key, value_str) in params {
            if let Some(var) = Self::parse_param(key, value_str)? {
                self.var_names.push(var.name.clone());
                self.variables.insert(var.name.clone(), var);
            }
        }

        // Build type cache
        let mut cache = HashMap::new();
        for (name, var) in &self.variables {
            cache.insert(name.clone(), var.var_type.clone());
        }
        self.var_type_cache = Some(cache);

        Ok(())
    }

    /// Configure from f64 params (for compatibility with existing config parsing).
    pub fn configure_from_f64(&mut self, params: &HashMap<String, f64>) -> BmiResult<()> {
        // Convert f64 params to string params
        let string_params: HashMap<String, String> = params
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string()))
            .collect();
        self.configure(&string_params)
    }

    /// Parse a single parameter in SLOTH format.
    ///
    /// Format: "varname(count,type,units,location)" -> value
    fn parse_param(key: &str, value_str: &str) -> BmiResult<Option<SlothVar>> {
        // Parse the key format: varname(count,type,units,location)
        let paren_start = match key.find('(') {
            Some(idx) => idx,
            None => {
                // Not in SLOTH format, skip
                return Ok(None);
            }
        };

        let paren_end = match key.find(')') {
            Some(idx) => idx,
            None => return Ok(None),
        };

        let name = key[..paren_start].to_string();
        let params_str = &key[paren_start + 1..paren_end];
        let parts: Vec<&str> = params_str.split(',').collect();

        if parts.len() != 4 {
            return Err(BmiError::BmiFunctionFailed {
                model: "SLOTH".to_string(),
                func: format!("Invalid SLOTH param format: {}", key),
            });
        }

        let count: usize = parts[0].trim().parse().unwrap_or(1);
        let type_str = parts[1].trim().to_string();
        let units = parts[2].trim().to_string();
        let location = parts[3].trim().to_string();

        // Parse the type
        let var_type = match type_str.as_str() {
            "double" | "float64" | "real8" => VarType::Double,
            "float" | "float32" | "real" | "real4" => VarType::Float,
            "int" | "integer" | "int32" => VarType::Int,
            _ => VarType::Double, // Default to double
        };

        // Parse the value
        let value: f64 = value_str
            .trim()
            .parse()
            .map_err(|_| BmiError::BmiFunctionFailed {
                model: "SLOTH".to_string(),
                func: format!("Invalid value '{}' for {}", value_str, name),
            })?;

        Ok(Some(SlothVar {
            name,
            count,
            var_type,
            type_str,
            units,
            location,
            value,
        }))
    }

    fn require_initialized(&self) -> BmiResult<()> {
        if !self.initialized {
            Err(BmiError::NotInitialized {
                model: self.model_name.clone(),
            })
        } else {
            Ok(())
        }
    }

    fn get_var(&self, name: &str) -> BmiResult<&SlothVar> {
        self.variables
            .get(name)
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: format!("Unknown variable: {}", name),
            })
    }
}

impl Bmi for BmiSloth {
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn initialize(&mut self, _config_file: &str) -> BmiResult<()> {
        // SLOTH doesn't need a config file - it's configured via model_params
        self.initialized = true;
        self.current_time = 0.0;
        Ok(())
    }

    fn update(&mut self) -> BmiResult<()> {
        self.require_initialized()?;
        self.current_time += self.time_step;
        Ok(())
    }

    fn update_until(&mut self, time: f64) -> BmiResult<()> {
        self.require_initialized()?;
        self.current_time = time;
        Ok(())
    }

    fn update_for_duration_seconds(&mut self, duration_seconds: f64) -> BmiResult<()> {
        self.require_initialized()?;
        self.current_time += duration_seconds;
        Ok(())
    }

    fn finalize(&mut self) -> BmiResult<()> {
        self.initialized = false;
        Ok(())
    }

    fn get_component_name(&self) -> BmiResult<String> {
        Ok(format!("SLOTH ({})", self.model_name))
    }

    fn get_input_item_count(&self) -> BmiResult<i32> {
        Ok(0) // SLOTH has no inputs
    }

    fn get_output_item_count(&self) -> BmiResult<i32> {
        self.require_initialized()?;
        Ok(self.var_names.len() as i32)
    }

    fn get_input_var_names(&self) -> BmiResult<Vec<String>> {
        Ok(vec![]) // SLOTH has no inputs
    }

    fn get_output_var_names(&self) -> BmiResult<Vec<String>> {
        self.require_initialized()?;
        Ok(self.var_names.clone())
    }

    fn get_var_grid(&self, _name: &str) -> BmiResult<i32> {
        Ok(0)
    }

    fn get_var_type(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;
        let var = self.get_var(name)?;
        Ok(var.type_str.clone())
    }

    fn get_var_units(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;
        let var = self.get_var(name)?;
        Ok(var.units.clone())
    }

    fn get_var_itemsize(&self, name: &str) -> BmiResult<i32> {
        self.require_initialized()?;
        let var = self.get_var(name)?;
        match var.var_type {
            VarType::Double => Ok(8),
            VarType::Float => Ok(4),
            VarType::Int => Ok(4),
            VarType::Unknown(_) => Ok(8),
        }
    }

    fn get_var_nbytes(&self, name: &str) -> BmiResult<i32> {
        self.require_initialized()?;
        let var = self.get_var(name)?;
        let itemsize = match var.var_type {
            VarType::Double => 8,
            VarType::Float => 4,
            VarType::Int => 4,
            VarType::Unknown(_) => 8,
        };
        Ok((var.count * itemsize) as i32)
    }

    fn get_var_location(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;
        let var = self.get_var(name)?;
        Ok(var.location.clone())
    }

    fn get_current_time(&self) -> BmiResult<f64> {
        self.require_initialized()?;
        Ok(self.current_time)
    }

    fn get_start_time(&self) -> BmiResult<f64> {
        Ok(0.0)
    }

    fn get_end_time(&self) -> BmiResult<f64> {
        Ok(self.end_time)
    }

    fn get_time_units(&self) -> BmiResult<String> {
        Ok("s".to_string())
    }

    fn get_time_step(&self) -> BmiResult<f64> {
        Ok(self.time_step)
    }

    fn get_time_convert_factor(&self) -> f64 {
        1.0
    }

    fn convert_model_time_to_seconds(&self, model_time: f64) -> f64 {
        model_time
    }

    fn convert_seconds_to_model_time(&self, seconds: f64) -> f64 {
        seconds
    }

    fn get_value_f64(&self, name: &str) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;
        let var = self.get_var(name)?;
        Ok(vec![var.value; var.count])
    }

    fn get_value_f32(&self, name: &str) -> BmiResult<Vec<f32>> {
        self.require_initialized()?;
        let var = self.get_var(name)?;
        Ok(vec![var.value as f32; var.count])
    }

    fn get_value_i32(&self, name: &str) -> BmiResult<Vec<i32>> {
        self.require_initialized()?;
        let var = self.get_var(name)?;
        Ok(vec![var.value as i32; var.count])
    }

    fn get_value_at_indices_f64(&self, name: &str, indices: &[i32]) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;
        let var = self.get_var(name)?;
        Ok(vec![var.value; indices.len()])
    }

    fn set_value_f64(&mut self, name: &str, values: &[f64]) -> BmiResult<()> {
        self.require_initialized()?;
        if let Some(var) = self.variables.get_mut(name) {
            if let Some(&value) = values.first() {
                var.value = value;
            }
        }
        Ok(())
    }

    fn set_value_f32(&mut self, name: &str, values: &[f32]) -> BmiResult<()> {
        self.require_initialized()?;
        if let Some(var) = self.variables.get_mut(name) {
            if let Some(&value) = values.first() {
                var.value = value as f64;
            }
        }
        Ok(())
    }

    fn set_value_i32(&mut self, name: &str, values: &[i32]) -> BmiResult<()> {
        self.require_initialized()?;
        if let Some(var) = self.variables.get_mut(name) {
            if let Some(&value) = values.first() {
                var.value = value as f64;
            }
        }
        Ok(())
    }

    fn set_value_at_indices_f64(
        &mut self,
        name: &str,
        _indices: &[i32],
        values: &[f64],
    ) -> BmiResult<()> {
        self.set_value_f64(name, values)
    }

    fn get_grid_rank(&self, _grid: i32) -> BmiResult<i32> {
        Ok(1)
    }

    fn get_grid_size(&self, _grid: i32) -> BmiResult<i32> {
        Ok(1)
    }

    fn get_grid_type(&self, _grid: i32) -> BmiResult<String> {
        Ok("scalar".to_string())
    }

    fn get_grid_shape(&self, _grid: i32) -> BmiResult<Vec<i32>> {
        Ok(vec![1])
    }

    fn get_grid_spacing(&self, _grid: i32) -> BmiResult<Vec<f64>> {
        Ok(vec![1.0])
    }

    fn get_grid_origin(&self, _grid: i32) -> BmiResult<Vec<f64>> {
        Ok(vec![0.0])
    }

    fn get_var_type_cache(&self) -> Option<&HashMap<String, VarType>> {
        self.var_type_cache.as_ref()
    }

    fn get_var_type_cache_mut(&mut self) -> &mut Option<HashMap<String, VarType>> {
        &mut self.var_type_cache
    }
}
