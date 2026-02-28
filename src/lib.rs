pub mod aliases;
mod adapters;
pub mod config;
mod error;
pub mod ffi;
mod forcings;
mod library;
pub mod runner;
mod traits;
pub mod units;

pub use adapters::{BmiC, BmiFortran, BmiSloth};
pub use config::{parse_datetime, BmiAdapterType, ModuleConfig, RealizationConfig};
pub use error::{BmiError, BmiResult};
pub use forcings::{Forcings, NetCdfForcings};
pub use library::preload_dependencies;
pub use runner::{ModelInstance, ModelRunner, VarSource};
pub use traits::{Bmi, BmiExt, BmiValue, VarType};

pub type BmiModel = BmiC;
