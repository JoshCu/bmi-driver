pub mod aliases;
mod adapters;
pub mod config;
mod error;
pub mod ffi;
mod forcings;
mod library;
mod resample;
pub mod runner;
mod traits;
pub mod units;

pub use adapters::BmiC;
#[cfg(feature = "fortran")]
pub use adapters::BmiFortran;
#[cfg(feature = "python")]
pub use adapters::BmiPython;
pub use adapters::BmiSloth;
pub use config::{parse_datetime, BmiAdapterType, DownsampleMode, ModuleConfig, RealizationConfig, UpsampleMode};
pub use error::{BmiError, BmiResult};
pub use forcings::{Forcings, NetCdfForcings};
pub use library::preload_dependencies;
pub use runner::{ModelInstance, ModelRunner, TimestepInfo, VarSource};
pub use traits::{Bmi, BmiExt, BmiValue, VarType};

pub type BmiModel = BmiC;
