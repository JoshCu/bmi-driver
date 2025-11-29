//! Forcings trait and types for reading forcing data.
//!
//! This module provides an interface similar to BMI but focused on
//! reading time-series forcing data (e.g., from NetCDF files).

use crate::error::{BmiError, BmiResult};
use crate::traits::{BmiValue, VarType};
use std::collections::HashMap;

/// Information about a forcing variable.
#[derive(Debug, Clone)]
pub struct ForcingVarInfo {
    /// Variable name
    pub name: String,
    /// Units string
    pub units: String,
    /// Data type
    pub var_type: VarType,
    /// Size of one element in bytes
    pub itemsize: i32,
    /// Total size in bytes (itemsize * count)
    pub nbytes: i32,
}

/// Common interface for forcing data providers.
///
/// This trait is similar to BMI but focused on reading forcing data
/// rather than running a model. It provides:
/// - Time-series data access
/// - Variable metadata
/// - Support for multiple locations (catchments)
pub trait Forcings {
    /// Get the name/identifier of this forcing provider.
    fn name(&self) -> &str;

    /// Check if the forcing provider has been initialized.
    fn is_initialized(&self) -> bool;

    /// Initialize the forcing provider.
    ///
    /// For file-based providers, this opens the file and reads metadata.
    /// The `config` parameter is provider-specific (e.g., file path).
    fn initialize(&mut self, config: &str) -> BmiResult<()>;

    /// Finalize and clean up resources.
    fn finalize(&mut self) -> BmiResult<()>;

    /// Get the component/source name of the forcing data.
    fn get_component_name(&self) -> BmiResult<String>;

    // =========================================================================
    // Variable Information
    // =========================================================================

    /// Get the number of output (forcing) variables.
    fn get_output_item_count(&self) -> BmiResult<i32>;

    /// Get the names of all output variables.
    fn get_output_var_names(&self) -> BmiResult<Vec<String>>;

    /// Get the data type of a variable as a string.
    fn get_var_type(&self, name: &str) -> BmiResult<String>;

    /// Get the units of a variable.
    fn get_var_units(&self, name: &str) -> BmiResult<String>;

    /// Get the size in bytes of one element of a variable.
    fn get_var_itemsize(&self, name: &str) -> BmiResult<i32>;

    /// Get the total size in bytes of a variable (for one timestep, one location).
    fn get_var_nbytes(&self, name: &str) -> BmiResult<i32>;

    // =========================================================================
    // Time Information
    // =========================================================================

    /// Get the start time (first timestep) in the forcing data.
    fn get_start_time(&self) -> BmiResult<f64>;

    /// Get the end time (last timestep) in the forcing data.
    fn get_end_time(&self) -> BmiResult<f64>;

    /// Get the time units (e.g., "s", "seconds since epoch").
    fn get_time_units(&self) -> BmiResult<String>;

    /// Get the time step size.
    fn get_time_step(&self) -> BmiResult<f64>;

    /// Get the total number of timesteps.
    fn get_timestep_count(&self) -> BmiResult<usize>;

    // =========================================================================
    // Location/Catchment Information
    // =========================================================================

    /// Get the list of location/catchment IDs.
    fn get_location_ids(&self) -> BmiResult<Vec<String>>;

    /// Get the number of locations/catchments.
    fn get_location_count(&self) -> BmiResult<usize>;

    /// Get the index for a location ID.
    fn get_location_index(&self, id: &str) -> BmiResult<usize>;

    // =========================================================================
    // Value Getters
    // =========================================================================

    /// Get all values for a variable at a specific location (all timesteps).
    fn get_value_f64(&self, name: &str, location_id: &str) -> BmiResult<Vec<f64>>;

    /// Get all values for a variable at a specific location (all timesteps).
    fn get_value_f32(&self, name: &str, location_id: &str) -> BmiResult<Vec<f32>>;

    /// Get all values for a variable at a specific location (all timesteps).
    fn get_value_i32(&self, name: &str, location_id: &str) -> BmiResult<Vec<i32>>;

    /// Get a single value for a variable at a specific location and timestep.
    fn get_value_at_index_f64(
        &self,
        name: &str,
        location_id: &str,
        time_index: usize,
    ) -> BmiResult<f64>;

    /// Get a single value for a variable at a specific location and timestep.
    fn get_value_at_index_f32(
        &self,
        name: &str,
        location_id: &str,
        time_index: usize,
    ) -> BmiResult<f32>;

    /// Get values for a range of timesteps.
    fn get_value_range_f64(
        &self,
        name: &str,
        location_id: &str,
        start_index: usize,
        end_index: usize,
    ) -> BmiResult<Vec<f64>>;

    /// Get values for a range of timesteps.
    fn get_value_range_f32(
        &self,
        name: &str,
        location_id: &str,
        start_index: usize,
        end_index: usize,
    ) -> BmiResult<Vec<f32>>;

    // =========================================================================
    // Variable Type Cache
    // =========================================================================

    /// Get the cached variable types.
    fn get_var_type_cache(&self) -> Option<&HashMap<String, VarType>>;

    /// Get the cached type for a variable.
    fn get_cached_var_type(&self, name: &str) -> Option<VarType> {
        self.get_var_type_cache()
            .and_then(|cache| cache.get(name).cloned())
    }
}

/// Extension trait providing convenience methods for Forcings.
pub trait ForcingsExt: Forcings {
    /// Get variable value with automatic type handling.
    fn get_value(&self, name: &str, location_id: &str) -> BmiResult<BmiValue> {
        let var_type = if let Some(vt) = self.get_cached_var_type(name) {
            vt
        } else {
            let type_str = self.get_var_type(name)?;
            let item_size = self.get_var_itemsize(name)?;
            VarType::from_bmi_type(&type_str, item_size)
        };

        match var_type {
            VarType::Float => Ok(BmiValue::Float(self.get_value_f32(name, location_id)?)),
            VarType::Double => Ok(BmiValue::Double(self.get_value_f64(name, location_id)?)),
            VarType::Int => Ok(BmiValue::Int(self.get_value_i32(name, location_id)?)),
            VarType::Unknown(t) => Err(BmiError::BmiFunctionFailed {
                model: self.name().to_string(),
                func: format!("get_value: unknown type '{}'", t),
            }),
        }
    }

    /// Get a single value at a timestep with automatic type handling.
    fn get_value_at_index(
        &self,
        name: &str,
        location_id: &str,
        time_index: usize,
    ) -> BmiResult<f64> {
        let var_type = if let Some(vt) = self.get_cached_var_type(name) {
            vt
        } else {
            let type_str = self.get_var_type(name)?;
            let item_size = self.get_var_itemsize(name)?;
            VarType::from_bmi_type(&type_str, item_size)
        };

        match var_type {
            VarType::Float => {
                Ok(self.get_value_at_index_f32(name, location_id, time_index)? as f64)
            }
            VarType::Double => self.get_value_at_index_f64(name, location_id, time_index),
            VarType::Int => Err(BmiError::BmiFunctionFailed {
                model: self.name().to_string(),
                func: "get_value_at_index: integer type not supported for scalar".to_string(),
            }),
            VarType::Unknown(t) => Err(BmiError::BmiFunctionFailed {
                model: self.name().to_string(),
                func: format!("get_value_at_index: unknown type '{}'", t),
            }),
        }
    }

    /// Get parsed variable type.
    fn get_var_type_parsed(&self, name: &str) -> BmiResult<VarType> {
        if let Some(vt) = self.get_cached_var_type(name) {
            return Ok(vt);
        }
        let type_str = self.get_var_type(name)?;
        let item_size = self.get_var_itemsize(name)?;
        Ok(VarType::from_bmi_type(&type_str, item_size))
    }

    /// Check if a variable exists.
    fn has_variable(&self, name: &str) -> BmiResult<bool> {
        let names = self.get_output_var_names()?;
        Ok(names.iter().any(|n| n == name))
    }

    /// Get info about all variables.
    fn get_all_var_info(&self) -> BmiResult<Vec<ForcingVarInfo>> {
        let names = self.get_output_var_names()?;
        let mut infos = Vec::with_capacity(names.len());

        for name in names {
            let units = self.get_var_units(&name)?;
            let type_str = self.get_var_type(&name)?;
            let itemsize = self.get_var_itemsize(&name)?;
            let nbytes = self.get_var_nbytes(&name)?;
            let var_type = VarType::from_bmi_type(&type_str, itemsize);

            infos.push(ForcingVarInfo {
                name,
                units,
                var_type,
                itemsize,
                nbytes,
            });
        }

        Ok(infos)
    }
}

// Blanket implementation
impl<T: Forcings + ?Sized> ForcingsExt for T {}
