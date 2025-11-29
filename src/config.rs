//! Configuration types for parsing realization JSON files.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::error::{BmiError, BmiResult};

/// Root configuration structure.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RealizationConfig {
    pub global: GlobalConfig,
    pub time: TimeConfig,
    #[serde(default)]
    pub output_root: String,
}

/// Global configuration including formulations and forcing.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GlobalConfig {
    pub formulations: Vec<FormulationConfig>,
    pub forcing: ForcingConfig,
}

/// Time configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TimeConfig {
    pub start_time: String,
    pub end_time: String,
    #[serde(default = "default_output_interval")]
    pub output_interval: i64,
}

fn default_output_interval() -> i64 {
    3600
}

/// Forcing configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForcingConfig {
    pub path: String,
}

/// A formulation (can be single model or multi-model).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FormulationConfig {
    pub name: String,
    pub params: FormulationParams,
}

/// Parameters for a formulation.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FormulationParams {
    pub name: String,
    #[serde(default)]
    pub model_type_name: String,
    #[serde(default)]
    pub main_output_variable: String,
    #[serde(default)]
    pub output_variables: Vec<String>,
    /// For bmi_multi: list of sub-modules
    #[serde(default)]
    pub modules: Vec<ModuleConfig>,
    /// For single models: library file path
    #[serde(default)]
    pub library_file: String,
    /// For single models: init config path (may contain {{id}})
    #[serde(default)]
    pub init_config: String,
    /// For C models: registration function name
    #[serde(default)]
    pub registration_function: String,
    /// Variable name mapping: model_input -> source_variable
    #[serde(default)]
    pub variables_names_map: HashMap<String, String>,
    /// Model parameters to set after initialization
    #[serde(default)]
    pub model_params: HashMap<String, f64>,
}

/// Configuration for a single BMI module within a multi-model setup.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModuleConfig {
    pub name: String,
    pub params: ModuleParams,
}

/// Parameters for a BMI module.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModuleParams {
    #[serde(default)]
    pub model_type_name: String,
    #[serde(default)]
    pub library_file: String,
    #[serde(default)]
    pub init_config: String,
    #[serde(default)]
    pub registration_function: String,
    #[serde(default)]
    pub main_output_variable: String,
    #[serde(default)]
    pub variables_names_map: HashMap<String, String>,
    /// Model parameters - stored as JSON values to handle both numbers and strings
    #[serde(default)]
    pub model_params: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub allow_exceed_end_time: bool,
    #[serde(default)]
    pub fixed_time_step: bool,
    #[serde(default)]
    pub uses_forcing_file: bool,
}

impl ModuleParams {
    /// Get model_params as f64 values (for non-SLOTH models).
    pub fn get_model_params_f64(&self) -> HashMap<String, f64> {
        self.model_params
            .iter()
            .filter_map(|(k, v)| {
                let f64_val = match v {
                    serde_json::Value::Number(n) => n.as_f64(),
                    serde_json::Value::String(s) => s.parse::<f64>().ok(),
                    _ => None,
                };
                f64_val.map(|val| (k.clone(), val))
            })
            .collect()
    }

    /// Get model_params as string values (for SLOTH models).
    pub fn get_model_params_string(&self) -> HashMap<String, String> {
        self.model_params
            .iter()
            .map(|(k, v)| {
                let str_val = match v {
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::String(s) => s.clone(),
                    _ => v.to_string(),
                };
                (k.clone(), str_val)
            })
            .collect()
    }
}

/// Type of BMI adapter to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BmiAdapterType {
    C,
    Fortran,
}

impl BmiAdapterType {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "bmi_c" => Some(BmiAdapterType::C),
            "bmi_fortran" => Some(BmiAdapterType::Fortran),
            _ => None,
        }
    }
}

impl RealizationConfig {
    /// Load configuration from a JSON file.
    pub fn from_file(path: impl AsRef<Path>) -> BmiResult<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).map_err(|e| BmiError::ConfigFileNotFound {
            path: format!("{}: {}", path.display(), e),
        })?;

        serde_json::from_str(&content).map_err(|e| BmiError::BmiFunctionFailed {
            model: "config".to_string(),
            func: format!("Failed to parse config: {}", e),
        })
    }

    /// Get all modules from the formulation (handles both single and multi).
    pub fn get_modules(&self) -> Vec<&ModuleConfig> {
        let mut modules = Vec::new();
        for formulation in &self.global.formulations {
            if formulation.name == "bmi_multi" {
                modules.extend(formulation.params.modules.iter());
            }
        }
        modules
    }

    /// Get the main output variable for the formulation.
    pub fn get_main_output_variable(&self) -> Option<&str> {
        self.global
            .formulations
            .first()
            .map(|f| f.params.main_output_variable.as_str())
    }

    /// Get the output variables list.
    pub fn get_output_variables(&self) -> Vec<&str> {
        self.global
            .formulations
            .first()
            .map(|f| {
                f.params
                    .output_variables
                    .iter()
                    .map(|s| s.as_str())
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl ModuleParams {
    /// Get the init config path with {{id}} replaced.
    pub fn get_init_config(&self, location_id: &str) -> String {
        self.init_config.replace("{{id}}", location_id)
    }
}

/// Parse a datetime string like "2010-10-01 00:00:00" to epoch seconds.
pub fn parse_datetime(datetime_str: &str) -> BmiResult<i64> {
    // Simple parser for "YYYY-MM-DD HH:MM:SS" format
    let parts: Vec<&str> = datetime_str.split(&[' ', '-', ':'][..]).collect();
    if parts.len() != 6 {
        return Err(BmiError::BmiFunctionFailed {
            model: "config".to_string(),
            func: format!("Invalid datetime format: {}", datetime_str),
        });
    }

    let year: i32 = parts[0].parse().map_err(|_| BmiError::BmiFunctionFailed {
        model: "config".to_string(),
        func: format!("Invalid year: {}", parts[0]),
    })?;
    let month: u32 = parts[1].parse().map_err(|_| BmiError::BmiFunctionFailed {
        model: "config".to_string(),
        func: format!("Invalid month: {}", parts[1]),
    })?;
    let day: u32 = parts[2].parse().map_err(|_| BmiError::BmiFunctionFailed {
        model: "config".to_string(),
        func: format!("Invalid day: {}", parts[2]),
    })?;
    let hour: u32 = parts[3].parse().map_err(|_| BmiError::BmiFunctionFailed {
        model: "config".to_string(),
        func: format!("Invalid hour: {}", parts[3]),
    })?;
    let min: u32 = parts[4].parse().map_err(|_| BmiError::BmiFunctionFailed {
        model: "config".to_string(),
        func: format!("Invalid minute: {}", parts[4]),
    })?;
    let sec: u32 = parts[5].parse().map_err(|_| BmiError::BmiFunctionFailed {
        model: "config".to_string(),
        func: format!("Invalid second: {}", parts[5]),
    })?;

    // Calculate days since epoch (1970-01-01)
    // Simplified calculation - doesn't handle all edge cases
    let mut days: i64 = 0;

    // Years
    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }

    // Months
    let days_in_month = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        days += days_in_month[(m - 1) as usize] as i64;
        if m == 2 && is_leap_year(year) {
            days += 1;
        }
    }

    // Days
    days += (day - 1) as i64;

    // Convert to seconds
    let seconds = days * 86400 + (hour as i64) * 3600 + (min as i64) * 60 + (sec as i64);

    Ok(seconds)
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_datetime() {
        // 2010-10-01 00:00:00
        let epoch = parse_datetime("2010-10-01 00:00:00").unwrap();
        assert_eq!(epoch, 1285891200);
    }
}
