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
//! ## Model Runner
//!
//! The `ModelRunner` provides a high-level interface for running multiple BMI models
//! together based on a realization configuration file.
//!
//! ## Example
//!
//! ```no_run
//! use bmi::ModelRunner;
//!
//! fn main() -> Result<(), bmi::BmiError> {
//!     // Load configuration and create runner
//!     let mut runner = ModelRunner::from_config("realization.json")?;
//!
//!     // Initialize for a specific catchment
//!     runner.initialize("cat-123")?;
//!
//!     // Run all timesteps
//!     while runner.has_more_steps() {
//!         runner.update()?;
//!
//!         // Get outputs
//!         let q_out = runner.get_main_output()?;
//!         println!("Step {}: Q_OUT = {:.6}", runner.current_step(), q_out);
//!     }
//!
//!     runner.finalize()?;
//!     Ok(())
//! }
//! ```

mod adapter_c;
mod adapter_fortran;
mod adapter_sloth;
mod bmi_ffi;
pub mod config;
mod error;
mod forcings;
mod forcings_netcdf;
mod library;
mod runner;
mod traits;

// Re-export the main types
pub use adapter_c::BmiC;
pub use adapter_fortran::BmiFortran;
pub use adapter_sloth::BmiSloth;
pub use config::{BmiAdapterType, ModuleConfig, RealizationConfig};
pub use error::{BmiError, BmiResult};
pub use forcings::{ForcingVarInfo, Forcings, ForcingsExt};
pub use forcings_netcdf::{NetCdfForcings, NetCdfForcingsConfig};
pub use library::preload_dependencies;
pub use runner::{ModelInstance, ModelRunner, VarSource};
pub use traits::{Bmi, BmiExt, BmiValue, VarType};

// For backwards compatibility, also export BmiC as BmiModel
pub type BmiModel = BmiC;

// Re-export FFI types for advanced usage
pub mod ffi {
    pub use crate::bmi_ffi::*;
}
