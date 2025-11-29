//! # BMI-RS: Rust bindings for the Basic Model Interface
//!
//! This crate provides safe Rust bindings for loading and interacting with
//! BMI (Basic Model Interface) compliant models compiled as shared libraries.
//!
//! ## Overview
//!
//! The [Basic Model Interface](https://bmi.readthedocs.io/) is a standardized
//! set of functions for controlling and coupling numerical models. This crate
//! allows you to:
//!
//! - Load BMI C libraries dynamically at runtime
//! - Initialize models with configuration files
//! - Run model time steps
//! - Get and set model variables
//! - Query model metadata (time, grid info, variable info)
//!
//! ## Example
//!
//! ```no_run
//! use bmi::BmiModel;
//!
//! fn main() -> Result<(), bmi::BmiError> {
//!     // Load a BMI model from a shared library
//!     let mut model = BmiModel::load(
//!         "my_hydrology_model",
//!         "/path/to/libmodel.so",
//!         "register_bmi",
//!     )?;
//!
//!     // Initialize with a config file
//!     model.initialize("/path/to/config.yml")?;
//!
//!     // Print model info
//!     println!("Component: {}", model.get_component_name()?);
//!     println!("Time units: {}", model.get_time_units()?);
//!     println!("Start time: {}", model.get_start_time()?);
//!     println!("End time: {}", model.get_end_time()?);
//!
//!     // Print input/output variables
//!     println!("\nInput variables:");
//!     for name in model.get_input_var_names()? {
//!         println!("  - {} ({})", name, model.get_var_units(&name)?);
//!     }
//!
//!     println!("\nOutput variables:");
//!     for name in model.get_output_var_names()? {
//!         println!("  - {} ({})", name, model.get_var_units(&name)?);
//!     }
//!
//!     // Run the model
//!     let end_time = model.get_end_time()?;
//!     while model.get_current_time()? < end_time {
//!         model.update()?;
//!
//!         // Read an output variable
//!         let values: Vec<f64> = model.get_value("discharge")?;
//!         println!("Time {}: discharge = {:?}", model.get_current_time()?, values);
//!     }
//!
//!     // Clean up (also happens automatically on drop)
//!     model.finalize()?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Setting Variable Values
//!
//! ```no_run
//! # use bmi::BmiModel;
//! # fn example(mut model: BmiModel) -> Result<(), bmi::BmiError> {
//! // Set values for an input variable
//! let precipitation = vec![0.5, 0.3, 0.8, 0.0];
//! model.set_value("precipitation", &precipitation)?;
//!
//! // Set values at specific indices
//! let indices = vec![0, 2];
//! let values = vec![1.0, 1.5];
//! model.set_value_at_indices("temperature", &indices, &values)?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Grid Information
//!
//! ```no_run
//! # use bmi::BmiModel;
//! # fn example(model: BmiModel) -> Result<(), bmi::BmiError> {
//! // Get the grid for a variable
//! let grid_id = model.get_var_grid("elevation")?;
//!
//! // Query grid properties
//! let grid_type = model.get_grid_type(grid_id)?;
//! let grid_rank = model.get_grid_rank(grid_id)?;
//! let grid_size = model.get_grid_size(grid_id)?;
//!
//! println!("Grid {}: type={}, rank={}, size={}", grid_id, grid_type, grid_rank, grid_size);
//!
//! // For uniform rectilinear grids
//! let shape = model.get_grid_shape(grid_id)?;
//! let spacing = model.get_grid_spacing(grid_id)?;
//! let origin = model.get_grid_origin(grid_id)?;
//! # Ok(())
//! # }
//! ```

mod adapter;
mod bmi_ffi;
mod error;

pub use adapter::{preload_dependencies, BmiModel};
pub use error::{BmiError, BmiResult};

// Re-export FFI types for advanced usage
pub mod ffi {
    pub use crate::bmi_ffi::*;
}
