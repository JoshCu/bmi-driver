//! # BMI-RS: Rust bindings for the Basic Model Interface
//!
//! This crate provides safe Rust bindings for loading and interacting with
//! BMI (Basic Model Interface) compliant models compiled as shared libraries.
//!
//! ## Supported Model Types
//!
//! - **C BMI** (`BmiC`): Models using the standard C BMI interface with function pointer structs
//! - **Fortran BMI** (`BmiFortran`): Models using Fortran iso_c_binding with free functions
//!
//! Both adapters implement the common `Bmi` trait, allowing them to be used interchangeably.
//!
//! ## Example
//!
//! ```no_run
//! use bmi::{Bmi, BmiExt, BmiC, BmiFortran, preload_dependencies};
//!
//! fn main() -> Result<(), bmi::BmiError> {
//!     // Preload dependencies (libm, etc.)
//!     preload_dependencies()?;
//!
//!     // Load a C BMI model
//!     let mut model = BmiC::load(
//!         "my_model",
//!         "/path/to/libmodel.so",
//!         "register_bmi",
//!     )?;
//!
//!     // Or load a Fortran BMI model (requires separate middleware library)
//!     // let mut model = BmiFortran::load(
//!     //     "my_fortran_model",
//!     //     "/path/to/libfortran_model.so",
//!     //     "/path/to/libbmi_fortran.so",  // middleware library
//!     //     "create_bmi_model",
//!     // )?;
//!     //
//!     // Or if everything is in one library:
//!     // let mut model = BmiFortran::load_single_library(
//!     //     "my_fortran_model",
//!     //     "/path/to/libfortran_model.so",
//!     //     "create_bmi_model",
//!     // )?;
//!
//!     // Initialize
//!     model.initialize("/path/to/config.yml")?;
//!
//!     // Use the common Bmi trait methods
//!     println!("Component: {}", model.get_component_name()?);
//!     println!("Time units: {}", model.get_time_units()?);
//!
//!     // Get values (type-specific)
//!     let values = model.get_value_f64("temperature")?;
//!
//!     // Or use the BmiExt convenience methods
//!     let scalar = model.get_value_scalar_f64("discharge")?;
//!
//!     // Run the model
//!     while model.get_current_time()? < model.get_end_time()? {
//!         model.update()?;
//!     }
//!
//!     model.finalize()?;
//!     Ok(())
//! }
//! ```
//!
//! ## Using with Dynamic Dispatch (Box<dyn Bmi>)
//!
//! ```no_run
//! use bmi::{Bmi, BmiC, BmiFortran};
//!
//! fn load_model(model_type: &str, lib_path: &str, middleware_path: Option<&str>, reg_func: &str)
//!     -> Result<Box<dyn Bmi>, bmi::BmiError>
//! {
//!     match model_type {
//!         "c" => Ok(Box::new(BmiC::load("model", lib_path, reg_func)?)),
//!         "fortran" => {
//!             if let Some(mw) = middleware_path {
//!                 Ok(Box::new(BmiFortran::load("model", lib_path, mw, reg_func)?))
//!             } else {
//!                 Ok(Box::new(BmiFortran::load_single_library("model", lib_path, reg_func)?))
//!             }
//!         }
//!         _ => panic!("Unknown model type"),
//!     }
//! }
//! ```

mod adapter_c;
mod adapter_fortran;
mod bmi_ffi;
mod error;
mod library;
mod traits;

// Re-export the main types
pub use adapter_c::BmiC;
pub use adapter_fortran::BmiFortran;
pub use error::{BmiError, BmiResult};
pub use library::preload_dependencies;
pub use traits::{Bmi, BmiExt, BmiValue, VarType};

// For backwards compatibility, also export BmiC as BmiModel
pub type BmiModel = BmiC;

// Re-export FFI types for advanced usage
pub mod ffi {
    pub use crate::bmi_ffi::*;
}
