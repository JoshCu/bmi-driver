mod adapters;
mod types;
mod variables;

pub mod config;
pub mod output;
pub mod runner;

// Re-export submodules at crate root so internal imports (e.g. crate::error) still resolve.
pub use adapters::ffi;
pub use adapters::library;
pub use types::error;
pub use types::traits;
pub use variables::aliases;
pub use variables::forcings;
pub use variables::resample;
pub use variables::units;

// Public API re-exports
pub use adapters::BmiC;
#[cfg(feature = "fortran")]
pub use adapters::BmiFortran;
#[cfg(feature = "python")]
pub use adapters::BmiPython;
pub use adapters::BmiSloth;
pub use config::{
    parse_datetime, BmiAdapterType, DownsampleMode, ModuleConfig, OutputFormat, RealizationConfig,
    UpsampleMode,
};
pub use error::{BmiError, BmiResult};
pub use forcings::{Forcings, NetCdfForcings};
pub use library::preload_dependencies;
pub use runner::{ModelInstance, ModelRunner, TimestepInfo, VarSource};
pub use traits::{Bmi, BmiExt, BmiValue, VarType};

pub type BmiModel = BmiC;
