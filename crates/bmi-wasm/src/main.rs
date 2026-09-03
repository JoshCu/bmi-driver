//! Emscripten build of the bmi-driver adapters.
//!
//! Depends on `bmi-core` (with default features off, so no Fortran/Python adapter) for `BmiC`
//! and `BmiSloth`, and layers a C ABI for JavaScript on top (`api.rs`).
//!
//! The heavy driver dependencies (netcdf, rusqlite, ipc-channel, ...) are deliberately not
//! pulled in -- this binary is just the BMI adapters plus that C ABI.

mod api;

fn main() {}
