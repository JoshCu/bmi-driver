use std::ffi::CString;
use std::fs::File;
use std::os::raw::{c_char, c_int};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::error::{function_failed, BmiError, BmiResult};

fn err(msg: String) -> BmiError {
    function_failed("netcdf_writer", msg)
}

/// Suppress HDF5's default error handler which prints verbose diagnostics
/// to stderr (e.g. "No such file or directory" when checking if a file exists
/// before creating it). These are not real errors — just internal HDF5 checks.
fn suppress_hdf5_errors() {
    extern "C" {
        fn H5Eset_auto2(
            estack_id: i64,
            func: *const std::ffi::c_void,
            client_data: *mut std::ffi::c_void,
        ) -> i32;
    }
    unsafe {
        H5Eset_auto2(0, std::ptr::null(), std::ptr::null_mut());
    }
}

use super::LocationResult;

// Minimal HDF5 API for locating each variable's raw data in the file.
// hid_t is int64_t and haddr_t is uint64_t for HDF5 >= 1.10.
extern "C" {
    fn H5Fopen(name: *const c_char, flags: u32, fapl_id: i64) -> i64;
    fn H5Fclose(file_id: i64) -> i32;
    fn H5Dopen2(loc_id: i64, name: *const c_char, dapl_id: i64) -> i64;
    fn H5Dclose(dset_id: i64) -> i32;
    fn H5Dget_offset(dset_id: i64) -> u64;
    fn H5Dget_create_plist(dset_id: i64) -> i64;
    fn H5Dget_type(dset_id: i64) -> i64;
    fn H5Pget_layout(plist_id: i64) -> c_int;
    fn H5Pget_nfilters(plist_id: i64) -> c_int;
    fn H5Pclose(plist_id: i64) -> i32;
    fn H5Tget_order(type_id: i64) -> c_int;
    fn H5Tget_size(type_id: i64) -> usize;
    fn H5Tclose(type_id: i64) -> i32;
}
const H5F_ACC_RDONLY: u32 = 0;
const H5P_DEFAULT: i64 = 0;
const H5D_CONTIGUOUS: c_int = 1;
const H5T_ORDER_BE: c_int = 1;
const HADDR_UNDEF: u64 = u64::MAX;

enum WriterMessage {
    Write(LocationResult),
    Shutdown,
}

/// Number of locations buffered before each write to disk.
const BATCH_SIZE: usize = 100;

const FILL_VALUE: f32 = -9999.0;

/// Where one output variable's (id, time) f32 data lives in the file.
struct VarLayout {
    name: String,
    offset: u64,
    big_endian: bool,
}

/// Byte layout of a file made by [`create_netcdf_store`].
struct DataLayout {
    n_times: usize,
    n_locations: usize,
    vars: Vec<VarLayout>,
}

/// Create the shared output file before any worker starts.
///
/// Every dimension is fixed and every location ID is written here, so the HDF5
/// metadata is final once this returns. Each output variable is stored contiguously
/// and uncompressed with its space allocated up front, which lets workers write
/// their rows straight to known byte offsets (see [`NetCdfWriter`]) with no merge.
pub fn create_netcdf_store(
    path: &Path,
    start_time: &str,
    interval: i64,
    total_steps: usize,
    location_ids: &[String],
    var_names: &[String],
) -> BmiResult<()> {
    suppress_hdf5_errors();

    let n_times = total_steps + 1;
    let n_locations = location_ids.len();
    let time_values: Vec<f64> = (0..n_times).map(|i| (i as f64) * interval as f64).collect();

    let mut file = create_file(
        path,
        n_times,
        n_locations,
        &format!("seconds since {}", start_time),
        &time_values,
        var_names,
    )?;

    // Workers write every row, so skip HDF5 pre-filling the data with _FillValue:
    // allocation then just reserves (sparse) space instead of writing the whole file.
    for name in var_names {
        let mut var = file
            .variable_mut(name)
            .ok_or_else(|| err(format!("Variable '{}' not found", name)))?;
        unsafe { var.set_nofill() }
            .map_err(|e| err(format!("Failed to set nofill on '{}': {}", name, e)))?;
        // set_nofill drops the attribute, but readers still need it to mask missing data
        var.put_attribute("_FillValue", FILL_VALUE)
            .map_err(|e| err(format!("Failed to set _FillValue on '{}': {}", name, e)))?;
    }

    // Contiguous datasets are allocated in full on their first write
    if n_locations > 0 {
        for name in var_names {
            let mut var = file
                .variable_mut(name)
                .ok_or_else(|| err(format!("Variable '{}' not found", name)))?;
            var.put_values(&[FILL_VALUE], (n_locations - 1, n_times - 1))
                .map_err(|e| err(format!("Failed to allocate '{}': {}", name, e)))?;
        }
    }

    drop(file);

    put_all_strings(path, "id", location_ids)?;

    // Fail here, once, rather than in every worker if the layout is unusable
    data_layout(path).map(|_| ())
}

/// Write a whole 1-D string variable in one call. The netcdf crate only writes one
/// string per call, which is far too slow for hundreds of thousands of IDs.
fn put_all_strings(path: &Path, var_name: &str, values: &[String]) -> BmiResult<()> {
    use netcdf_sys::{nc_close, nc_inq_varid, nc_open, nc_put_var_string, NC_NOERR, NC_WRITE};

    if values.is_empty() {
        return Ok(());
    }
    let c_path = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| err(format!("Invalid path {}", path.display())))?;
    let c_var = CString::new(var_name).map_err(|_| err(format!("Invalid name '{}'", var_name)))?;
    let c_values: Vec<CString> = values
        .iter()
        .map(|v| {
            CString::new(v.as_bytes()).map_err(|_| err(format!("Invalid {} '{}'", var_name, v)))
        })
        .collect::<BmiResult<_>>()?;
    let mut ptrs: Vec<*const c_char> = c_values.iter().map(|v| v.as_ptr()).collect();

    let _lock = netcdf_sys::libnetcdf_lock.lock();
    unsafe {
        let mut ncid = 0;
        if nc_open(c_path.as_ptr(), NC_WRITE, &mut ncid) != NC_NOERR {
            return Err(err(format!("Failed to reopen {}", path.display())));
        }
        let mut varid = 0;
        let mut status = nc_inq_varid(ncid, c_var.as_ptr(), &mut varid);
        if status == NC_NOERR {
            status = nc_put_var_string(ncid, varid, ptrs.as_mut_ptr());
        }
        let close_status = nc_close(ncid);
        if status != NC_NOERR || close_status != NC_NOERR {
            return Err(err(format!(
                "Failed to write '{}' (netcdf status {}, close status {})",
                var_name, status, close_status
            )));
        }
    }
    Ok(())
}

/// Read dimension sizes and each output variable's data offset.
///
/// Calls HDF5 directly, outside the netcdf crate's lock, so it must not run
/// concurrently with other netcdf/HDF5 use in the same process.
fn data_layout(path: &Path) -> BmiResult<DataLayout> {
    let file =
        netcdf::open(path).map_err(|e| err(format!("Failed to open {}: {}", path.display(), e)))?;
    let dim_len = |name: &str| {
        file.dimension(name).map(|d| d.len()).ok_or_else(|| {
            err(format!(
                "{} dimension not found in {}",
                name,
                path.display()
            ))
        })
    };
    let n_times = dim_len("time")?;
    let n_locations = dim_len("id")?;
    let var_names: Vec<String> = file
        .variables()
        .map(|v| v.name())
        .filter(|name| name != "time" && name != "id")
        .collect();
    drop(file);

    let c_path = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| err(format!("Invalid path {}", path.display())))?;
    let file_id = unsafe { H5Fopen(c_path.as_ptr(), H5F_ACC_RDONLY, H5P_DEFAULT) };
    if file_id < 0 {
        return Err(err(format!("HDF5 failed to open {}", path.display())));
    }
    let vars: BmiResult<Vec<VarLayout>> = var_names
        .into_iter()
        .map(|name| var_layout(file_id, name))
        .collect();
    unsafe { H5Fclose(file_id) };

    Ok(DataLayout {
        n_times,
        n_locations,
        vars: vars?,
    })
}

fn var_layout(file_id: i64, name: String) -> BmiResult<VarLayout> {
    let c_name = CString::new(name.as_bytes())
        .map_err(|_| err(format!("Invalid variable name '{}'", name)))?;
    unsafe {
        let dset = H5Dopen2(file_id, c_name.as_ptr(), H5P_DEFAULT);
        if dset < 0 {
            return Err(err(format!("HDF5 dataset '{}' not found", name)));
        }
        let plist = H5Dget_create_plist(dset);
        let dtype = H5Dget_type(dset);
        let layout = H5Pget_layout(plist);
        let nfilters = H5Pget_nfilters(plist);
        let type_size = H5Tget_size(dtype);
        let big_endian = H5Tget_order(dtype) == H5T_ORDER_BE;
        let offset = H5Dget_offset(dset);
        H5Tclose(dtype);
        H5Pclose(plist);
        H5Dclose(dset);

        if layout != H5D_CONTIGUOUS || nfilters != 0 || type_size != 4 {
            return Err(err(format!(
                "'{}' must be contiguous, unfiltered f32 for direct writes \
                 (layout {}, {} filters, {}-byte type)",
                name, layout, nfilters, type_size
            )));
        }
        if offset == HADDR_UNDEF {
            return Err(err(format!("'{}' has no allocated storage", name)));
        }
        Ok(VarLayout {
            name,
            offset,
            big_endian,
        })
    }
}

/// Writes one worker's locations into a file made by [`create_netcdf_store`].
///
/// Each worker owns a contiguous range of `id` rows, and with (id, time) row-major
/// contiguous storage that range is one contiguous byte range per variable. Batches
/// are written there with positioned writes, bypassing HDF5 entirely, so any number
/// of worker processes can write the same file at once.
pub struct NetCdfWriter {
    sender: Sender<WriterMessage>,
    handle: Option<JoinHandle<BmiResult<()>>>,
    /// Shared error state: if the writer thread dies, the actual error is stored here
    /// so write() can report it instead of just "sending on a closed channel".
    thread_error: Arc<Mutex<Option<BmiError>>>,
}

impl NetCdfWriter {
    /// `start_idx` is the global `id` index of this writer's first location.
    pub fn new(path: PathBuf, start_idx: usize) -> BmiResult<Self> {
        suppress_hdf5_errors();
        // Resolved here, on the caller's thread, so HDF5 isn't used concurrently
        let layout = data_layout(&path)?;
        let file = File::options()
            .write(true)
            .open(&path)
            .map_err(|e| err(format!("Failed to open {}: {}", path.display(), e)))?;

        let (tx, rx) = mpsc::channel();
        let thread_error: Arc<Mutex<Option<BmiError>>> = Arc::new(Mutex::new(None));
        let thread_error_clone = Arc::clone(&thread_error);

        let handle = thread::spawn(move || {
            let result = writer_thread(rx, file, layout, start_idx);
            if let Err(ref e) = result {
                if let Ok(mut guard) = thread_error_clone.lock() {
                    *guard = Some(BmiError::FunctionFailed {
                        model: "netcdf_writer".into(),
                        func: format!("{}", e),
                    });
                }
            }
            result
        });

        Ok(Self {
            sender: tx,
            handle: Some(handle),
            thread_error,
        })
    }

    pub fn write(&self, result: LocationResult) -> BmiResult<()> {
        self.sender.send(WriterMessage::Write(result)).map_err(|_| {
            // Channel closed — the writer thread died. Report the actual error.
            if let Ok(guard) = self.thread_error.lock() {
                if let Some(ref e) = *guard {
                    return BmiError::FunctionFailed {
                        model: "netcdf_writer".into(),
                        func: format!("Writer thread failed: {}", e),
                    };
                }
            }
            err("Writer thread exited unexpectedly".into())
        })
    }

    pub fn finish_owned(mut self) -> BmiResult<()> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> BmiResult<()> {
        let _ = self.sender.send(WriterMessage::Shutdown);
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .map_err(|_| err("Writer thread panicked".into()))?
        } else {
            Ok(())
        }
    }
}

impl super::DivideDataStore for NetCdfWriter {
    fn write_location(&mut self, loc_id: &str, columns: &[(String, Vec<f64>)]) -> BmiResult<()> {
        self.write(LocationResult {
            location_id: loc_id.to_string(),
            columns: columns.to_vec(),
        })
    }

    fn finish(&mut self) -> BmiResult<()> {
        self.shutdown()
    }
}

fn writer_thread(
    receiver: Receiver<WriterMessage>,
    file: File,
    layout: DataLayout,
    start_idx: usize,
) -> BmiResult<()> {
    let mut next_idx = start_idx;
    let mut batch: Vec<LocationResult> = Vec::with_capacity(BATCH_SIZE);

    // Block until each result arrives; only touch the file once a full batch is ready.
    while let Ok(WriterMessage::Write(result)) = receiver.recv() {
        batch.push(result);
        if batch.len() >= BATCH_SIZE {
            next_idx = write_batch(&file, &layout, &batch, next_idx)?;
            batch.clear();
        }
    }
    write_batch(&file, &layout, &batch, next_idx)?;
    file.sync_all()
        .map_err(|e| err(format!("Failed to sync output file: {}", e)))
}

fn write_batch(
    file: &File,
    layout: &DataLayout,
    batch: &[LocationResult],
    start_idx: usize,
) -> BmiResult<usize> {
    let end_idx = start_idx + batch.len();
    if batch.is_empty() {
        return Ok(end_idx);
    }
    if end_idx > layout.n_locations {
        return Err(err(format!(
            "Location index {} is past the {} locations the file was created for",
            end_idx - 1,
            layout.n_locations
        )));
    }

    // Concatenating per-location rows gives row-major (id, time) order, matching the
    // variable layout, so each variable is written for the whole batch in one call.
    // Rows are padded/truncated to n_times; missing variables are written as fill.
    let n_times = layout.n_times;
    let row_bytes = n_times * 4;
    let mut data: Vec<u8> = Vec::with_capacity(batch.len() * row_bytes);
    for var in &layout.vars {
        let encode = |v: f32| {
            if var.big_endian {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        data.clear();
        for result in batch {
            let row_start = data.len();
            if let Some((_, vals)) = result.columns.iter().find(|(n, _)| *n == var.name) {
                for &v in vals.iter().take(n_times) {
                    data.extend_from_slice(&encode(v as f32));
                }
            }
            while data.len() < row_start + row_bytes {
                data.extend_from_slice(&encode(FILL_VALUE));
            }
        }
        let pos = var.offset + (start_idx * row_bytes) as u64;
        file.write_all_at(&data, pos)
            .map_err(|e| err(format!("Failed to write '{}': {}", var.name, e)))?;
    }

    Ok(end_idx)
}

/// Create a file with fixed-size `time` and `id` dimensions and the given variables.
fn create_file(
    path: &Path,
    n_times: usize,
    n_locations: usize,
    time_units: &str,
    time_values: &[f64],
    var_names: &[String],
) -> BmiResult<netcdf::FileMut> {
    let mut file = netcdf::create(path)
        .map_err(|e| err(format!("Failed to create {}: {}", path.display(), e)))?;

    file.add_dimension("time", n_times)
        .map_err(|e| err(format!("Failed to add time dimension: {}", e)))?;
    file.add_dimension("id", n_locations)
        .map_err(|e| err(format!("Failed to add id dimension: {}", e)))?;

    let mut time_var = file
        .add_variable::<f64>("time", &["time"])
        .map_err(|e| err(format!("Failed to add time variable: {}", e)))?;
    if !time_units.is_empty() {
        time_var
            .put_attribute("units", time_units)
            .map_err(|e| err(format!("Failed to set time units: {}", e)))?;
    }
    time_var
        .put_attribute("long_name", "valid output time")
        .map_err(|e| err(format!("Failed to set time long_name: {}", e)))?;
    time_var
        .put_values(time_values, ..)
        .map_err(|e| err(format!("Failed to write time values: {}", e)))?;

    // Location ID coordinate variable (same name as dimension for xarray .sel() support)
    file.add_string_variable("id", &["id"])
        .map_err(|e| err(format!("Failed to add id variable: {}", e)))?;

    for name in var_names {
        let mut var = file
            .add_variable::<f32>(name, &["id", "time"])
            .map_err(|e| err(format!("Failed to add variable '{}': {}", name, e)))?;
        var.put_attribute("_FillValue", FILL_VALUE)
            .map_err(|e| err(format!("Failed to set _FillValue on '{}': {}", name, e)))?;
    }

    Ok(file)
}
