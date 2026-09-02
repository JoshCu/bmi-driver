//! Orchestration for BMI model chains: configuration, NetCDF forcings, dependency-ordered
//! execution, unit conversion, resampling, and output writing.
//!
//! Two entry points, depending on how much control you want:
//!
//! - [`Simulation`] -- open a data directory, run one location, get the results back as vectors.
//!   This is what you want when embedding the driver in another program.
//! - [`ModelRunner`] -- the underlying stateful runner, if you need access to the individual
//!   model instances, their variable maps or unit conversions.
//!
//! ```no_run
//! # fn main() -> bmi_driver::BmiResult<()> {
//! let mut sim = bmi_driver::Simulation::open("/data/my_run")?;
//! let flows = sim.run_variable("cat-123", "Q_OUT")?;
//! # Ok(()) }
//! ```
//!
//! ## Paths
//!
//! Every relative path in a realization config -- the forcing file, `init_config`, `library_file`
//! -- is resolved against the *data directory* passed in when the runner is built, never against
//! the process working directory. Nothing in this crate calls `chdir`, so several runners for
//! different data directories can coexist in one process.
//!
//! ## Threads
//!
//! A [`Simulation`] is not `Sync` and deliberately not `Send`: the models behind it are shared
//! libraries that frequently keep global state (Fortran `save` variables, C file-scope statics)
//! and Python models hold the GIL. Build one `Simulation` per thread, or run separate processes
//! the way the CLI does.

pub mod config;
pub mod forcings;
#[cfg(feature = "gpkg")]
pub mod locations;
pub mod model_info;
pub mod nc_lock;
pub mod output;
pub mod runner;
pub mod simulation;

// Re-export the core BMI layer so downstream users need only one dependency.
pub use bmi_core::{
    aliases, error, ffi, library, parse_time_units, preload_dependencies, resample, traits, units,
    Bmi, BmiC, BmiError, BmiExt, BmiResult, BmiSloth, BmiValue, DownsampleMode, UpsampleMode,
    VarType,
};
#[cfg(feature = "fortran")]
pub use bmi_core::BmiFortran;
#[cfg(feature = "python")]
pub use bmi_core::BmiPython;

pub use config::{
    parse_datetime, BmiAdapterType, ModuleConfig, OutputFormat, RealizationConfig,
};
pub use forcings::{Forcings, NetCdfForcings};
#[cfg(feature = "gpkg")]
pub use locations::read_gpkg_locations;
pub use output::DivideDataStore;
pub use runner::{ModelInstance, ModelRunner, TimestepInfo, VarSource};
pub use simulation::{run_location, RunOutput, Simulation};
