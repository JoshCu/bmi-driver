//! A process-wide lock around libnetcdf.
//!
//! libnetcdf is only thread-safe when built with `--enable-threadsafe`, which most distribution
//! packages are not, and the `netcdf` crate does no locking of its own. Opening two files
//! concurrently from different threads corrupts HDF5's global state and surfaces as a spurious
//! "NetCDF: HDF error".
//!
//! Everything that touches the C library goes through [`with_netcdf`]. That is coarse, but it
//! only covers file open/create and the bulk read of one location's forcings -- the model run
//! itself, which is where the time goes, stays fully parallel.

use std::sync::{Mutex, MutexGuard, OnceLock};

fn lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Run `f` with exclusive access to libnetcdf.
///
/// Not reentrant: never call this from inside another `with_netcdf` closure.
pub fn with_netcdf<T>(f: impl FnOnce() -> T) -> T {
    let _guard: MutexGuard<'_, ()> = lock().lock().unwrap_or_else(|e| e.into_inner());
    f()
}

/// `netcdf::open` under the lock. A convenience for the common case.
pub fn with_netcdf_open(
    path: impl AsRef<std::path::Path>,
) -> Result<netcdf::File, netcdf::Error> {
    with_netcdf(|| netcdf::open(path))
}
