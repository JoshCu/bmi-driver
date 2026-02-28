use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use crate::error::{BmiError, BmiResult};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RealizationConfig {
    pub global: GlobalConfig,
    pub time: TimeConfig,
    #[serde(default)]
    pub output_root: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GlobalConfig {
    pub formulations: Vec<FormulationConfig>,
    pub forcing: ForcingConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TimeConfig {
    pub start_time: String,
    pub end_time: String,
    #[serde(default = "default_interval")]
    pub output_interval: i64,
}

fn default_interval() -> i64 { 3600 }

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ForcingConfig {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FormulationConfig {
    pub name: String,
    pub params: FormulationParams,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FormulationParams {
    pub name: String,
    #[serde(default)]
    pub model_type_name: String,
    #[serde(default)]
    pub main_output_variable: String,
    #[serde(default)]
    pub output_variables: Vec<String>,
    #[serde(default)]
    pub modules: Vec<ModuleConfig>,
    #[serde(default)]
    pub library_file: String,
    #[serde(default)]
    pub init_config: String,
    #[serde(default)]
    pub registration_function: String,
    #[serde(default)]
    pub variables_names_map: HashMap<String, String>,
    #[serde(default)]
    pub model_params: HashMap<String, f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModuleConfig {
    pub name: String,
    pub params: ModuleParams,
}

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
    pub fn params_f64(&self) -> HashMap<String, f64> {
        self.model_params.iter().filter_map(|(k, v)| {
            let val = match v {
                serde_json::Value::Number(n) => n.as_f64(),
                serde_json::Value::String(s) => s.parse().ok(),
                _ => None,
            };
            val.map(|f| (k.clone(), f))
        }).collect()
    }

    pub fn params_string(&self) -> HashMap<String, String> {
        self.model_params.iter().map(|(k, v)| {
            let s = match v {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => v.to_string(),
            };
            (k.clone(), s)
        }).collect()
    }

    pub fn init_config(&self, loc_id: &str) -> String {
        self.init_config.replace("{{id}}", loc_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BmiAdapterType {
    C,
    #[cfg(feature = "fortran")]
    Fortran,
    #[cfg(feature = "python")]
    Python,
}

impl BmiAdapterType {
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "bmi_c" => Some(Self::C),
            #[cfg(feature = "fortran")]
            "bmi_fortran" => Some(Self::Fortran),
            #[cfg(feature = "python")]
            "bmi_python" => Some(Self::Python),
            _ => None,
        }
    }
}

impl RealizationConfig {
    pub fn from_file(path: impl AsRef<Path>) -> BmiResult<Self> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .map_err(|e| BmiError::ConfigNotFound { path: format!("{}: {}", path.display(), e) })?;
        serde_json::from_str(&content)
            .map_err(|e| BmiError::FunctionFailed { model: "config".into(), func: format!("parse: {}", e) })
    }

    pub fn modules(&self) -> Vec<&ModuleConfig> {
        self.global.formulations.iter()
            .filter(|f| f.name == "bmi_multi")
            .flat_map(|f| f.params.modules.iter())
            .collect()
    }

    pub fn main_output(&self) -> Option<&str> {
        self.global.formulations.first().map(|f| f.params.main_output_variable.as_str())
    }
}

pub fn parse_datetime(s: &str) -> BmiResult<i64> {
    let p: Vec<&str> = s.split(&[' ', '-', ':'][..]).collect();
    if p.len() != 6 {
        return Err(BmiError::FunctionFailed { model: "config".into(), func: format!("Invalid datetime: {}", s) });
    }

    let parse = |i: usize| -> BmiResult<i64> {
        p[i].parse().map_err(|_| BmiError::FunctionFailed {
            model: "config".into(), func: format!("Invalid datetime part: {}", p[i])
        })
    };

    let (year, month, day) = (parse(0)? as i32, parse(1)? as u32, parse(2)? as u32);
    let (hour, min, sec) = (parse(3)?, parse(4)?, parse(5)?);

    let leap = |y: i32| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days_in_month = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    let mut days: i64 = 0;
    for y in 1970..year {
        days += if leap(y) { 366 } else { 365 };
    }
    for m in 1..month {
        days += days_in_month[(m - 1) as usize] as i64;
        if m == 2 && leap(year) { days += 1; }
    }
    days += (day - 1) as i64;

    Ok(days * 86400 + hour * 3600 + min * 60 + sec)
}
