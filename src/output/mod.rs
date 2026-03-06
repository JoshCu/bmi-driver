use crate::error::BmiResult;

pub mod csv;
pub mod netcdf;
#[cfg(feature = "zarr")]
pub mod zarr;

/// Unified write interface for per-location timeseries data.
///
/// Each output format (CSV, NetCDF, Zarr) implements this trait so that
/// the worker loop can write results without knowing the format.
pub trait DivideDataStore {
    /// Write one location's output data. Each column is (variable_name, values_per_timestep).
    fn write_location(&mut self, loc_id: &str, columns: &[(String, Vec<f64>)]) -> BmiResult<()>;

    /// Finalize the store (flush buffers, close files, etc).
    fn finish(&mut self) -> BmiResult<()> {
        Ok(())
    }
}
