//! BMI adapters and the supporting value/unit machinery.
//!
//! This crate knows how to load a model that implements the Basic Model Interface -- from a C or
//! Fortran shared library, from a Python class, or the built-in SLOTH constant provider -- and
//! call it through one uniform [`Bmi`] trait. It has no opinion about forcing data, configuration
//! files or output formats; that belongs to `bmi-driver`.

mod adapters;
mod types;

pub mod aliases;
pub mod resample;
pub mod units;

pub use adapters::ffi;
pub use adapters::library;
pub use types::error;
pub use types::traits;

pub use adapters::BmiC;
#[cfg(feature = "fortran")]
pub use adapters::BmiFortran;
#[cfg(feature = "python")]
pub use adapters::BmiPython;
pub use adapters::BmiSloth;
pub use error::{BmiError, BmiResult};
pub use library::preload_dependencies;
pub use resample::{DownsampleMode, UpsampleMode};
pub use traits::{parse_time_units, Bmi, BmiExt, BmiValue, VarType};
