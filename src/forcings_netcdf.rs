//! NetCDF implementation of the Forcings trait.
//!
//! Reads forcing data from NetCDF files with dimensions (catchment-id, time).

use std::collections::HashMap;
use std::path::Path;

use netcdf::{self, File as NetCdfFile};

use crate::error::{BmiError, BmiResult};
use crate::forcings::Forcings;
use crate::traits::VarType;

/// NetCDF-based forcing data provider.
///
/// Expects NetCDF files with:
/// - Dimensions: `catchment-id` (or similar), `time`
/// - Variables: forcing variables with shape (catchment-id, time)
/// - An `ids` variable containing location/catchment identifiers
/// - A `Time` variable containing timestamps
pub struct NetCdfForcings {
    name: String,
    file: Option<NetCdfFile>,
    initialized: bool,

    // Cached metadata
    location_ids: Vec<String>,
    location_index: HashMap<String, usize>,
    var_names: Vec<String>,
    var_info: HashMap<String, VarInfo>,
    var_type_cache: HashMap<String, VarType>,

    // Time info
    timestep_count: usize,
    time_step: f64,
    start_time: f64,
    end_time: f64,
    time_units: String,

    // Configuration
    config: NetCdfForcingsConfig,
}

/// Configuration for NetCDF forcing reader.
#[derive(Debug, Clone)]
pub struct NetCdfForcingsConfig {
    /// Name of the dimension for locations/catchments (default: "catchment-id")
    pub location_dim: String,
    /// Name of the dimension for time (default: "time")
    pub time_dim: String,
    /// Name of the variable containing location IDs (default: "ids")
    pub ids_var: String,
    /// Name of the variable containing time values (default: "Time")
    pub time_var: String,
    /// Variables to exclude from forcing output (e.g., coordinate variables)
    pub exclude_vars: Vec<String>,
}

impl Default for NetCdfForcingsConfig {
    fn default() -> Self {
        Self {
            location_dim: "catchment-id".to_string(),
            time_dim: "time".to_string(),
            ids_var: "ids".to_string(),
            time_var: "Time".to_string(),
            exclude_vars: vec!["ids".to_string(), "Time".to_string()],
        }
    }
}

#[derive(Debug, Clone)]
struct VarInfo {
    units: String,
    var_type: VarType,
    itemsize: i32,
    type_str: String,
}

impl NetCdfForcings {
    /// Create a new NetCDF forcing provider with default configuration.
    pub fn new(name: impl Into<String>) -> Self {
        Self::with_config(name, NetCdfForcingsConfig::default())
    }

    /// Create a new NetCDF forcing provider with custom configuration.
    pub fn with_config(name: impl Into<String>, config: NetCdfForcingsConfig) -> Self {
        Self {
            name: name.into(),
            file: None,
            initialized: false,
            location_ids: Vec::new(),
            location_index: HashMap::new(),
            var_names: Vec::new(),
            var_info: HashMap::new(),
            var_type_cache: HashMap::new(),
            timestep_count: 0,
            time_step: 0.0,
            start_time: 0.0,
            end_time: 0.0,
            time_units: String::new(),
            config,
        }
    }

    /// Get a reference to the underlying NetCDF file.
    pub fn file(&self) -> Option<&NetCdfFile> {
        self.file.as_ref()
    }

    fn require_initialized(&self) -> BmiResult<()> {
        if !self.initialized {
            Err(BmiError::NotInitialized {
                model: self.name.clone(),
            })
        } else {
            Ok(())
        }
    }

    fn load_metadata(&mut self) -> BmiResult<()> {
        let file = self.file.as_ref().ok_or_else(|| BmiError::NotInitialized {
            model: self.name.clone(),
        })?;

        // Load location IDs
        let ids_var =
            file.variable(&self.config.ids_var)
                .ok_or_else(|| BmiError::BmiFunctionFailed {
                    model: self.name.clone(),
                    func: format!("Couldn't find variable '{}'", self.config.ids_var),
                })?;

        let num_locations = ids_var.len();
        self.location_ids.clear();
        self.location_index.clear();

        for i in 0..num_locations {
            let id = ids_var
                .get_string(i)
                .map_err(|e| BmiError::BmiFunctionFailed {
                    model: self.name.clone(),
                    func: format!("Failed to get location ID at index {}: {}", i, e),
                })?;
            self.location_index.insert(id.clone(), i);
            self.location_ids.push(id);
        }

        // Get timestep count
        self.timestep_count = file
            .dimension(&self.config.time_dim)
            .map(|d| d.len())
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Couldn't find dimension '{}'", self.config.time_dim),
            })?;

        // Load time information
        self.load_time_info()?;

        // Discover forcing variables
        self.discover_variables()?;

        Ok(())
    }

    fn load_time_info(&mut self) -> BmiResult<()> {
        let file = self.file.as_ref().ok_or_else(|| BmiError::NotInitialized {
            model: self.name.clone(),
        })?;

        // Try to get time variable
        if let Some(time_var) = file.variable(&self.config.time_var) {
            // Get time units from attribute
            self.time_units = time_var
                .attribute("units")
                .and_then(|a| a.value().ok())
                .and_then(|v| match v {
                    netcdf::AttributeValue::Str(s) => Some(s),
                    _ => None,
                })
                .unwrap_or_else(|| "s".to_string());

            // Read first location's time values to get start/end/step
            if self.timestep_count > 0 && !self.location_ids.is_empty() {
                // Get time values - assume 2D (location, time) or 1D (time)
                let dims = time_var.dimensions();

                let times: Vec<i64> = if dims.len() == 2 {
                    // 2D: get first location's times
                    time_var.get_values::<i64, _>((0usize, ..)).map_err(|e| {
                        BmiError::BmiFunctionFailed {
                            model: self.name.clone(),
                            func: format!("Failed to read time values: {}", e),
                        }
                    })?
                } else if dims.len() == 1 {
                    // 1D: get all times
                    time_var
                        .get_values::<i64, _>(..)
                        .map_err(|e| BmiError::BmiFunctionFailed {
                            model: self.name.clone(),
                            func: format!("Failed to read time values: {}", e),
                        })?
                } else {
                    return Err(BmiError::BmiFunctionFailed {
                        model: self.name.clone(),
                        func: format!("Unexpected time variable dimensions: {}", dims.len()),
                    });
                };

                if !times.is_empty() {
                    self.start_time = times[0] as f64;
                    self.end_time = times[times.len() - 1] as f64;

                    if times.len() > 1 {
                        self.time_step = (times[1] - times[0]) as f64;
                    }
                }
            }
        } else {
            // No time variable - use defaults
            self.time_units = "s".to_string();
            self.start_time = 0.0;
            self.time_step = 3600.0; // Default 1 hour
            self.end_time = self.start_time + (self.timestep_count as f64 - 1.0) * self.time_step;
        }

        Ok(())
    }

    fn discover_variables(&mut self) -> BmiResult<()> {
        let file = self.file.as_ref().ok_or_else(|| BmiError::NotInitialized {
            model: self.name.clone(),
        })?;

        self.var_names.clear();
        self.var_info.clear();
        self.var_type_cache.clear();

        for var in file.variables() {
            let var_name = var.name();

            // Skip excluded variables
            if self.config.exclude_vars.contains(&var_name) {
                continue;
            }

            // Check if this is a forcing variable (has location and time dimensions)
            let dims: Vec<String> = var.dimensions().iter().map(|d| d.name()).collect();

            // Must have both location and time dimensions
            if !dims.contains(&self.config.location_dim) || !dims.contains(&self.config.time_dim) {
                continue;
            }

            // Get variable metadata
            let units = var
                .attribute("units")
                .and_then(|a| a.value().ok())
                .map(|v| match v {
                    netcdf::AttributeValue::Str(s) => s,
                    _ => String::new(),
                })
                .unwrap_or_default();

            // Determine type from NetCDF type
            let (type_str, itemsize, var_type) = Self::get_var_type_info(&var);

            self.var_names.push(var_name.clone());
            self.var_info.insert(
                var_name.clone(),
                VarInfo {
                    units,
                    var_type: var_type.clone(),
                    itemsize,
                    type_str,
                },
            );
            self.var_type_cache.insert(var_name, var_type);
        }

        Ok(())
    }

    fn get_var_type_info(var: &netcdf::Variable) -> (String, i32, VarType) {
        use netcdf::types::{FloatType, IntType, NcVariableType};

        match var.vartype() {
            NcVariableType::Float(FloatType::F32) => ("float".to_string(), 4, VarType::Float),
            NcVariableType::Float(FloatType::F64) => ("double".to_string(), 8, VarType::Double),
            NcVariableType::Int(IntType::I32) => ("int".to_string(), 4, VarType::Int),
            NcVariableType::Int(IntType::I16) => ("short".to_string(), 2, VarType::Int),
            NcVariableType::Int(IntType::I8) => ("byte".to_string(), 1, VarType::Int),
            NcVariableType::Int(IntType::U8) => ("ubyte".to_string(), 1, VarType::Int),
            NcVariableType::Int(IntType::U16) => ("ushort".to_string(), 2, VarType::Int),
            NcVariableType::Int(IntType::U32) => ("uint".to_string(), 4, VarType::Int),
            NcVariableType::Int(IntType::I64) => ("int64".to_string(), 8, VarType::Int),
            NcVariableType::Int(IntType::U64) => ("uint64".to_string(), 8, VarType::Int),
            NcVariableType::String => (
                "string".to_string(),
                0,
                VarType::Unknown("string".to_string()),
            ),
            NcVariableType::Char => ("char".to_string(), 1, VarType::Unknown("char".to_string())),
            _ => (
                "unknown".to_string(),
                4,
                VarType::Unknown("unknown".to_string()),
            ),
        }
    }

    fn get_location_idx(&self, location_id: &str) -> BmiResult<usize> {
        self.location_index
            .get(location_id)
            .copied()
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Unknown location ID: '{}'", location_id),
            })
    }
}

impl Forcings for NetCdfForcings {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn initialize(&mut self, config: &str) -> BmiResult<()> {
        if self.initialized {
            return Err(BmiError::AlreadyInitialized {
                model: self.name.clone(),
            });
        }

        let path = Path::new(config);
        if !path.exists() {
            return Err(BmiError::ConfigFileNotFound {
                path: config.to_string(),
            });
        }

        // Open the NetCDF file
        self.file = Some(netcdf::open(path).map_err(|e| BmiError::BmiFunctionFailed {
            model: self.name.clone(),
            func: format!("Failed to open NetCDF file: {}", e),
        })?);

        // Load metadata
        self.load_metadata()?;

        self.initialized = true;
        Ok(())
    }

    fn finalize(&mut self) -> BmiResult<()> {
        self.file = None;
        self.initialized = false;
        self.location_ids.clear();
        self.location_index.clear();
        self.var_names.clear();
        self.var_info.clear();
        self.var_type_cache.clear();
        Ok(())
    }

    fn get_component_name(&self) -> BmiResult<String> {
        Ok(format!("NetCDF Forcings: {}", self.name))
    }

    fn get_output_item_count(&self) -> BmiResult<i32> {
        self.require_initialized()?;
        Ok(self.var_names.len() as i32)
    }

    fn get_output_var_names(&self) -> BmiResult<Vec<String>> {
        self.require_initialized()?;
        Ok(self.var_names.clone())
    }

    fn get_var_type(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;
        self.var_info
            .get(name)
            .map(|info| info.type_str.clone())
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Unknown variable: '{}'", name),
            })
    }

    fn get_var_units(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;
        self.var_info
            .get(name)
            .map(|info| info.units.clone())
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Unknown variable: '{}'", name),
            })
    }

    fn get_var_itemsize(&self, name: &str) -> BmiResult<i32> {
        self.require_initialized()?;
        self.var_info
            .get(name)
            .map(|info| info.itemsize)
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Unknown variable: '{}'", name),
            })
    }

    fn get_var_nbytes(&self, name: &str) -> BmiResult<i32> {
        self.require_initialized()?;
        self.var_info
            .get(name)
            .map(|info| info.itemsize) // Single value per timestep
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Unknown variable: '{}'", name),
            })
    }

    fn get_start_time(&self) -> BmiResult<f64> {
        self.require_initialized()?;
        Ok(self.start_time)
    }

    fn get_end_time(&self) -> BmiResult<f64> {
        self.require_initialized()?;
        Ok(self.end_time)
    }

    fn get_time_units(&self) -> BmiResult<String> {
        self.require_initialized()?;
        Ok(self.time_units.clone())
    }

    fn get_time_step(&self) -> BmiResult<f64> {
        self.require_initialized()?;
        Ok(self.time_step)
    }

    fn get_timestep_count(&self) -> BmiResult<usize> {
        self.require_initialized()?;
        Ok(self.timestep_count)
    }

    fn get_location_ids(&self) -> BmiResult<Vec<String>> {
        self.require_initialized()?;
        Ok(self.location_ids.clone())
    }

    fn get_location_count(&self) -> BmiResult<usize> {
        self.require_initialized()?;
        Ok(self.location_ids.len())
    }

    fn get_location_index(&self, id: &str) -> BmiResult<usize> {
        self.require_initialized()?;
        self.get_location_idx(id)
    }

    fn get_value_f64(&self, name: &str, location_id: &str) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;
        let idx = self.get_location_idx(location_id)?;

        let file = self.file.as_ref().ok_or_else(|| BmiError::NotInitialized {
            model: self.name.clone(),
        })?;

        let var = file
            .variable(name)
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Variable '{}' not found", name),
            })?;

        var.get_values::<f64, _>((idx, ..))
            .map_err(|e| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Failed to read '{}': {}", name, e),
            })
    }

    fn get_value_f32(&self, name: &str, location_id: &str) -> BmiResult<Vec<f32>> {
        self.require_initialized()?;
        let idx = self.get_location_idx(location_id)?;

        let file = self.file.as_ref().ok_or_else(|| BmiError::NotInitialized {
            model: self.name.clone(),
        })?;

        let var = file
            .variable(name)
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Variable '{}' not found", name),
            })?;

        var.get_values::<f32, _>((idx, ..))
            .map_err(|e| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Failed to read '{}': {}", name, e),
            })
    }

    fn get_value_i32(&self, name: &str, location_id: &str) -> BmiResult<Vec<i32>> {
        self.require_initialized()?;
        let idx = self.get_location_idx(location_id)?;

        let file = self.file.as_ref().ok_or_else(|| BmiError::NotInitialized {
            model: self.name.clone(),
        })?;

        let var = file
            .variable(name)
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Variable '{}' not found", name),
            })?;

        var.get_values::<i32, _>((idx, ..))
            .map_err(|e| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Failed to read '{}': {}", name, e),
            })
    }

    fn get_value_at_index_f64(
        &self,
        name: &str,
        location_id: &str,
        time_index: usize,
    ) -> BmiResult<f64> {
        self.require_initialized()?;
        let loc_idx = self.get_location_idx(location_id)?;

        let file = self.file.as_ref().ok_or_else(|| BmiError::NotInitialized {
            model: self.name.clone(),
        })?;

        let var = file
            .variable(name)
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Variable '{}' not found", name),
            })?;

        let values: Vec<f64> = var
            .get_values((loc_idx, time_index..time_index + 1))
            .map_err(|e| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Failed to read '{}' at index {}: {}", name, time_index, e),
            })?;

        values
            .into_iter()
            .next()
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: "Empty result".to_string(),
            })
    }

    fn get_value_at_index_f32(
        &self,
        name: &str,
        location_id: &str,
        time_index: usize,
    ) -> BmiResult<f32> {
        self.require_initialized()?;
        let loc_idx = self.get_location_idx(location_id)?;

        let file = self.file.as_ref().ok_or_else(|| BmiError::NotInitialized {
            model: self.name.clone(),
        })?;

        let var = file
            .variable(name)
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Variable '{}' not found", name),
            })?;

        let values: Vec<f32> = var
            .get_values((loc_idx, time_index..time_index + 1))
            .map_err(|e| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Failed to read '{}' at index {}: {}", name, time_index, e),
            })?;

        values
            .into_iter()
            .next()
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: "Empty result".to_string(),
            })
    }

    fn get_value_range_f64(
        &self,
        name: &str,
        location_id: &str,
        start_index: usize,
        end_index: usize,
    ) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;
        let loc_idx = self.get_location_idx(location_id)?;

        let file = self.file.as_ref().ok_or_else(|| BmiError::NotInitialized {
            model: self.name.clone(),
        })?;

        let var = file
            .variable(name)
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Variable '{}' not found", name),
            })?;

        var.get_values((loc_idx, start_index..end_index))
            .map_err(|e| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Failed to read '{}': {}", name, e),
            })
    }

    fn get_value_range_f32(
        &self,
        name: &str,
        location_id: &str,
        start_index: usize,
        end_index: usize,
    ) -> BmiResult<Vec<f32>> {
        self.require_initialized()?;
        let loc_idx = self.get_location_idx(location_id)?;

        let file = self.file.as_ref().ok_or_else(|| BmiError::NotInitialized {
            model: self.name.clone(),
        })?;

        let var = file
            .variable(name)
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Variable '{}' not found", name),
            })?;

        var.get_values((loc_idx, start_index..end_index))
            .map_err(|e| BmiError::BmiFunctionFailed {
                model: self.name.clone(),
                func: format!("Failed to read '{}': {}", name, e),
            })
    }

    fn get_var_type_cache(&self) -> Option<&HashMap<String, VarType>> {
        if self.var_type_cache.is_empty() {
            None
        } else {
            Some(&self.var_type_cache)
        }
    }
}

impl Drop for NetCdfForcings {
    fn drop(&mut self) {
        let _ = self.finalize();
    }
}
