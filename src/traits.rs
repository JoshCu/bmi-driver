//! Common trait definition for BMI models.
//!
//! This trait provides a unified interface that both C and Fortran
//! BMI adapters implement, allowing them to be used interchangeably.

use crate::error::{BmiError, BmiResult};
use std::collections::HashMap;

/// Information about a BMI variable's type.
#[derive(Debug, Clone, PartialEq)]
pub enum VarType {
    /// 32-bit floating point (float, real)
    Float,
    /// 64-bit floating point (double, double precision)
    Double,
    /// 32-bit integer
    Int,
    /// Unknown or unsupported type
    Unknown(String),
}

impl VarType {
    /// Parse a type string from BMI into a VarType.
    ///
    /// Handles common type names from C, Fortran, and other languages.
    pub fn from_bmi_type(type_name: &str, item_size: i32) -> Self {
        let type_lower = type_name.to_lowercase();

        match type_lower.as_str() {
            // Floating point types
            "float" | "real" | "float32" | "real*4" | "real4" => VarType::Float,
            "double" | "double precision" | "float64" | "real*8" | "real8" => VarType::Double,

            // Integer types
            "int" | "integer" | "int32" | "integer*4" | "integer4" | "i32" => VarType::Int,

            // Ambiguous types - use item_size to determine
            _ => {
                // Check if it contains hints
                if type_lower.contains("double") {
                    VarType::Double
                } else if type_lower.contains("float") || type_lower.contains("real") {
                    // Use item_size to distinguish float vs double
                    if item_size == 8 {
                        VarType::Double
                    } else {
                        VarType::Float
                    }
                } else if type_lower.contains("int") {
                    VarType::Int
                } else {
                    // Fall back to item_size
                    match item_size {
                        4 => VarType::Float, // Assume float for 4-byte unknowns
                        8 => VarType::Double,
                        _ => VarType::Unknown(type_name.to_string()),
                    }
                }
            }
        }
    }
}

/// A dynamically-typed value that can hold different BMI data types.
#[derive(Debug, Clone)]
pub enum BmiValue {
    Float(Vec<f32>),
    Double(Vec<f64>),
    Int(Vec<i32>),
}

impl BmiValue {
    /// Get as f64 values, converting if necessary.
    pub fn as_f64(&self) -> Vec<f64> {
        match self {
            BmiValue::Double(v) => v.clone(),
            BmiValue::Float(v) => v.iter().map(|&x| x as f64).collect(),
            BmiValue::Int(v) => v.iter().map(|&x| x as f64).collect(),
        }
    }

    /// Get as f32 values, converting if necessary.
    pub fn as_f32(&self) -> Vec<f32> {
        match self {
            BmiValue::Float(v) => v.clone(),
            BmiValue::Double(v) => v.iter().map(|&x| x as f32).collect(),
            BmiValue::Int(v) => v.iter().map(|&x| x as f32).collect(),
        }
    }

    /// Get as i32 values, converting if necessary.
    pub fn as_i32(&self) -> Vec<i32> {
        match self {
            BmiValue::Int(v) => v.clone(),
            BmiValue::Float(v) => v.iter().map(|&x| x as i32).collect(),
            BmiValue::Double(v) => v.iter().map(|&x| x as i32).collect(),
        }
    }

    /// Get a single scalar value as f64.
    pub fn scalar_f64(&self) -> Option<f64> {
        match self {
            BmiValue::Double(v) => v.first().copied(),
            BmiValue::Float(v) => v.first().map(|&x| x as f64),
            BmiValue::Int(v) => v.first().map(|&x| x as f64),
        }
    }

    /// Get a single scalar value as f32.
    pub fn scalar_f32(&self) -> Option<f32> {
        match self {
            BmiValue::Float(v) => v.first().copied(),
            BmiValue::Double(v) => v.first().map(|&x| x as f32),
            BmiValue::Int(v) => v.first().map(|&x| x as f32),
        }
    }

    /// Get the length of the value array.
    pub fn len(&self) -> usize {
        match self {
            BmiValue::Float(v) => v.len(),
            BmiValue::Double(v) => v.len(),
            BmiValue::Int(v) => v.len(),
        }
    }

    /// Check if the value array is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Common interface for all BMI model adapters.
///
/// This trait abstracts over the differences between C BMI (function pointer struct)
/// and Fortran BMI (free functions with opaque handle) implementations.
pub trait Bmi {
    // =========================================================================
    // Model Information
    // =========================================================================

    /// Get the name of this model instance.
    fn model_name(&self) -> &str;

    /// Check if the model has been initialized.
    fn is_initialized(&self) -> bool;

    // =========================================================================
    // Initialize, Run, Finalize (IRF)
    // =========================================================================

    /// Initialize the model with a configuration file.
    fn initialize(&mut self, config_file: &str) -> BmiResult<()>;

    /// Advance the model by one time step.
    fn update(&mut self) -> BmiResult<()>;

    /// Advance the model to a specific time.
    fn update_until(&mut self, time: f64) -> BmiResult<()>;

    /// Advance the model by a duration specified in seconds.
    fn update_for_duration_seconds(&mut self, duration_seconds: f64) -> BmiResult<()>;

    /// Finalize and clean up the model.
    fn finalize(&mut self) -> BmiResult<()>;

    // =========================================================================
    // Model Information
    // =========================================================================

    /// Get the name of the model component.
    fn get_component_name(&self) -> BmiResult<String>;

    /// Get the number of input variables.
    fn get_input_item_count(&self) -> BmiResult<i32>;

    /// Get the number of output variables.
    fn get_output_item_count(&self) -> BmiResult<i32>;

    /// Get the names of all input variables.
    fn get_input_var_names(&self) -> BmiResult<Vec<String>>;

    /// Get the names of all output variables.
    fn get_output_var_names(&self) -> BmiResult<Vec<String>>;

    // =========================================================================
    // Variable Information
    // =========================================================================

    /// Get the grid ID for a variable.
    fn get_var_grid(&self, name: &str) -> BmiResult<i32>;

    /// Get the data type of a variable.
    fn get_var_type(&self, name: &str) -> BmiResult<String>;

    /// Get the units of a variable.
    fn get_var_units(&self, name: &str) -> BmiResult<String>;

    /// Get the size in bytes of one element of a variable.
    fn get_var_itemsize(&self, name: &str) -> BmiResult<i32>;

    /// Get the total size in bytes of a variable.
    fn get_var_nbytes(&self, name: &str) -> BmiResult<i32>;

    /// Get the location of a variable on the grid.
    fn get_var_location(&self, name: &str) -> BmiResult<String>;

    // =========================================================================
    // Time Information
    // =========================================================================

    /// Get the current model time.
    fn get_current_time(&self) -> BmiResult<f64>;

    /// Get the start time of the model.
    fn get_start_time(&self) -> BmiResult<f64>;

    /// Get the end time of the model.
    fn get_end_time(&self) -> BmiResult<f64>;

    /// Get the time units used by the model.
    fn get_time_units(&self) -> BmiResult<String>;

    /// Get the time step size of the model.
    fn get_time_step(&self) -> BmiResult<f64>;

    /// Get the time conversion factor (model time units to seconds).
    fn get_time_convert_factor(&self) -> f64;

    /// Convert a model time value to seconds.
    fn convert_model_time_to_seconds(&self, model_time: f64) -> f64;

    /// Convert seconds to model time units.
    fn convert_seconds_to_model_time(&self, seconds: f64) -> f64;

    // =========================================================================
    // Getters - Type-specific
    // =========================================================================

    /// Get values of a variable as f64.
    fn get_value_f64(&self, name: &str) -> BmiResult<Vec<f64>>;

    /// Get values of a variable as f32.
    fn get_value_f32(&self, name: &str) -> BmiResult<Vec<f32>>;

    /// Get values of a variable as i32.
    fn get_value_i32(&self, name: &str) -> BmiResult<Vec<i32>>;

    /// Get values at specific indices as f64.
    fn get_value_at_indices_f64(&self, name: &str, indices: &[i32]) -> BmiResult<Vec<f64>>;

    // =========================================================================
    // Setters - Type-specific
    // =========================================================================

    /// Set values of a variable from f64 slice.
    fn set_value_f64(&mut self, name: &str, values: &[f64]) -> BmiResult<()>;

    /// Set values of a variable from f32 slice.
    fn set_value_f32(&mut self, name: &str, values: &[f32]) -> BmiResult<()>;

    /// Set values of a variable from i32 slice.
    fn set_value_i32(&mut self, name: &str, values: &[i32]) -> BmiResult<()>;

    /// Set values at specific indices from f64 slice.
    fn set_value_at_indices_f64(
        &mut self,
        name: &str,
        indices: &[i32],
        values: &[f64],
    ) -> BmiResult<()>;

    // =========================================================================
    // Grid Information
    // =========================================================================

    /// Get the rank (number of dimensions) of a grid.
    fn get_grid_rank(&self, grid: i32) -> BmiResult<i32>;

    /// Get the total number of elements in a grid.
    fn get_grid_size(&self, grid: i32) -> BmiResult<i32>;

    /// Get the type of a grid.
    fn get_grid_type(&self, grid: i32) -> BmiResult<String>;

    /// Get the shape of a grid.
    fn get_grid_shape(&self, grid: i32) -> BmiResult<Vec<i32>>;

    /// Get the spacing of a grid.
    fn get_grid_spacing(&self, grid: i32) -> BmiResult<Vec<f64>>;

    /// Get the origin of a grid.
    fn get_grid_origin(&self, grid: i32) -> BmiResult<Vec<f64>>;

    // =========================================================================
    // Variable Type Cache (for auto-typing)
    // =========================================================================

    /// Get the cached variable types. Returns None if not yet cached.
    fn get_var_type_cache(&self) -> Option<&HashMap<String, VarType>>;

    /// Get a mutable reference to the variable type cache.
    fn get_var_type_cache_mut(&mut self) -> &mut Option<HashMap<String, VarType>>;

    /// Build and cache the variable type information.
    /// Called automatically after initialize if not already cached.
    fn cache_var_types(&mut self) -> BmiResult<()> {
        if self.get_var_type_cache().is_some() {
            return Ok(());
        }

        let mut cache = HashMap::new();

        // Cache input variable types
        for name in self.get_input_var_names()? {
            let type_str = self.get_var_type(&name)?;
            let item_size = self.get_var_itemsize(&name)?;
            cache.insert(name, VarType::from_bmi_type(&type_str, item_size));
        }

        // Cache output variable types
        for name in self.get_output_var_names()? {
            let type_str = self.get_var_type(&name)?;
            let item_size = self.get_var_itemsize(&name)?;
            cache.insert(name, VarType::from_bmi_type(&type_str, item_size));
        }

        *self.get_var_type_cache_mut() = Some(cache);
        Ok(())
    }

    /// Get the cached type for a variable.
    fn get_cached_var_type(&self, name: &str) -> Option<VarType> {
        self.get_var_type_cache()
            .and_then(|cache| cache.get(name).cloned())
    }
}

/// Extension trait providing convenience methods on top of the base Bmi trait.
pub trait BmiExt: Bmi {
    /// Get variable value with automatic type handling.
    ///
    /// This method checks the variable's actual type and calls the appropriate
    /// typed getter, returning the result as a BmiValue.
    fn get_value(&self, name: &str) -> BmiResult<BmiValue> {
        // Try to use cached type first
        let var_type = if let Some(vt) = self.get_cached_var_type(name) {
            vt
        } else {
            // Fall back to querying the model
            let type_str = self.get_var_type(name)?;
            let item_size = self.get_var_itemsize(name)?;
            VarType::from_bmi_type(&type_str, item_size)
        };

        match var_type {
            VarType::Float => Ok(BmiValue::Float(self.get_value_f32(name)?)),
            VarType::Double => Ok(BmiValue::Double(self.get_value_f64(name)?)),
            VarType::Int => Ok(BmiValue::Int(self.get_value_i32(name)?)),
            VarType::Unknown(t) => Err(BmiError::BmiFunctionFailed {
                model: self.model_name().to_string(),
                func: format!("get_value: unknown type '{}'", t),
            }),
        }
    }

    /// Set variable value with automatic type handling from f64 input.
    ///
    /// This method checks the variable's actual type and converts the input
    /// values appropriately before calling the typed setter.
    fn set_value(&mut self, name: &str, values: &[f64]) -> BmiResult<()> {
        // Try to use cached type first
        let var_type = if let Some(vt) = self.get_cached_var_type(name) {
            vt
        } else {
            // Fall back to querying the model
            let type_str = self.get_var_type(name)?;
            let item_size = self.get_var_itemsize(name)?;
            VarType::from_bmi_type(&type_str, item_size)
        };

        match var_type {
            VarType::Float => {
                let f32_values: Vec<f32> = values.iter().map(|&x| x as f32).collect();
                self.set_value_f32(name, &f32_values)
            }
            VarType::Double => self.set_value_f64(name, values),
            VarType::Int => {
                let i32_values: Vec<i32> = values.iter().map(|&x| x as i32).collect();
                self.set_value_i32(name, &i32_values)
            }
            VarType::Unknown(t) => Err(BmiError::BmiFunctionFailed {
                model: self.model_name().to_string(),
                func: format!("set_value: unknown type '{}'", t),
            }),
        }
    }

    /// Get the value of a variable as a single f64 scalar.
    ///
    /// This is a convenience method for scalar variables. It uses automatic
    /// type detection and converts the result to f64.
    fn get_value_scalar(&self, name: &str) -> BmiResult<f64> {
        let value = self.get_value(name)?;
        value
            .scalar_f64()
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.model_name().to_string(),
                func: "get_value_scalar (empty result)".to_string(),
            })
    }

    /// Set a single scalar value with automatic type handling.
    fn set_value_scalar(&mut self, name: &str, value: f64) -> BmiResult<()> {
        self.set_value(name, &[value])
    }

    /// Get the value of a variable at index 0 as f64.
    ///
    /// This is a convenience method for scalar variables.
    fn get_value_scalar_f64(&self, name: &str) -> BmiResult<f64> {
        let values = self.get_value_f64(name)?;
        values
            .into_iter()
            .next()
            .ok_or_else(|| BmiError::BmiFunctionFailed {
                model: self.model_name().to_string(),
                func: "get_value (empty result)".to_string(),
            })
    }

    /// Check if a variable is an input variable.
    fn is_input_variable(&self, name: &str) -> BmiResult<bool> {
        let names = self.get_input_var_names()?;
        Ok(names.iter().any(|n| n == name))
    }

    /// Check if a variable is an output variable.
    fn is_output_variable(&self, name: &str) -> BmiResult<bool> {
        let names = self.get_output_var_names()?;
        Ok(names.iter().any(|n| n == name))
    }

    /// Get the number of items in a variable (nbytes / itemsize).
    fn get_var_item_count(&self, name: &str) -> BmiResult<i32> {
        let nbytes = self.get_var_nbytes(name)?;
        let itemsize = self.get_var_itemsize(name)?;
        Ok(nbytes / itemsize)
    }

    /// Get the parsed type of a variable.
    fn get_var_type_parsed(&self, name: &str) -> BmiResult<VarType> {
        if let Some(vt) = self.get_cached_var_type(name) {
            return Ok(vt);
        }
        let type_str = self.get_var_type(name)?;
        let item_size = self.get_var_itemsize(name)?;
        Ok(VarType::from_bmi_type(&type_str, item_size))
    }
}

// Blanket implementation of BmiExt for anything that implements Bmi
impl<T: Bmi + ?Sized> BmiExt for T {}
