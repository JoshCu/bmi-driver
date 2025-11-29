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
//! ## Forcings
//!
//! The crate also provides a `Forcings` trait for reading forcing data, with a NetCDF
//! implementation (`NetCdfForcings`) for reading from NetCDF files.
//!
//! ## Example
//!
//! ```no_run
//! use bmi::{Bmi, BmiExt, BmiC, BmiFortran, preload_dependencies};
//! use bmi::{Forcings, ForcingsExt, NetCdfForcings};
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
//!     // Initialize
//!     model.initialize("/path/to/config.yml")?;
//!
//!     // Load forcing data from NetCDF
//!     let mut forcings = NetCdfForcings::new("my_forcings");
//!     forcings.initialize("/path/to/forcings.nc")?;
//!
//!     // Get forcing variable names
//!     for name in forcings.get_output_var_names()? {
//!         println!("Forcing var: {} [{}]", name, forcings.get_var_units(&name)?);
//!     }
//!
//!     // Run the model with forcing data
//!     let location = "cat-123";
//!     let mut step = 0;
//!     while model.get_current_time()? < model.get_end_time()? {
//!         // Get forcing value at current timestep (auto-typed)
//!         let temp = forcings.get_value_at_index("TMP_2maboveground", location, step)?;
//!         model.set_value("SFCTMP", &[temp])?;
//!
//!         model.update()?;
//!         step += 1;
//!     }
//!
//!     model.finalize()?;
//!     forcings.finalize()?;
//!     Ok(())
//! }
//! ```

mod adapter_c;
mod adapter_fortran;
mod bmi_ffi;
mod error;
mod forcings;
mod forcings_netcdf;
mod library;
mod traits;

// Re-export the main types
pub use adapter_c::BmiC;
pub use adapter_fortran::BmiFortran;
pub use error::{BmiError, BmiResult};
pub use forcings::{ForcingVarInfo, Forcings, ForcingsExt};
pub use forcings_netcdf::{NetCdfForcings, NetCdfForcingsConfig};
pub use library::preload_dependencies;
pub use traits::{Bmi, BmiExt, BmiValue, VarType};

// For backwards compatibility, also export BmiC as BmiModel
pub type BmiModel = BmiC;

// Re-export FFI types for advanced usage
pub mod ffi {
    pub use crate::bmi_ffi::*;
}
