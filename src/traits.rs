//! Common trait definition for BMI models.
//!
//! This trait provides a unified interface that both C and Fortran
//! BMI adapters implement, allowing them to be used interchangeably.

use crate::error::BmiResult;

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
    fn set_value_at_indices_f64(&mut self, name: &str, indices: &[i32], values: &[f64]) -> BmiResult<()>;

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
}

/// Extension trait providing convenience methods on top of the base Bmi trait.
pub trait BmiExt: Bmi {
    /// Get the value of a variable at index 0 as f64.
    /// 
    /// This is a convenience method for scalar variables.
    fn get_value_scalar_f64(&self, name: &str) -> BmiResult<f64> {
        let values = self.get_value_f64(name)?;
        values.into_iter().next().ok_or_else(|| {
            crate::error::BmiError::BmiFunctionFailed {
                model: self.model_name().to_string(),
                func: "get_value (empty result)".to_string(),
            }
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
}

// Blanket implementation of BmiExt for anything that implements Bmi
impl<T: Bmi + ?Sized> BmiExt for T {}
