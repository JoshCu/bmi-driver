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

enum WriterMessage {
    Write(LocationResult),
    Shutdown,
}

/// Number of locations buffered before each write to disk.
const BATCH_SIZE: usize = 100;

/// Upper bound on values held in memory per variable while merging worker files.
const MERGE_BLOCK_VALUES: usize = 16 * 1024 * 1024;

const FILL_VALUE: f32 = -9999.0;

pub struct NetCdfWriter {
    sender: Sender<WriterMessage>,
    handle: Option<JoinHandle<BmiResult<()>>>,
    /// Shared error state: if the writer thread dies, the actual error is stored here
    /// so write() can report it instead of just "sending on a closed channel".
    thread_error: Arc<Mutex<Option<BmiError>>>,
}

impl NetCdfWriter {
    /// `n_locations` is the number of locations this writer will receive. The `id`
    /// dimension is sized to it up front, since unlimited dimensions are much slower.
    pub fn new(
        path: PathBuf,
        start_time: &str,
        interval: i64,
        total_steps: usize,
        n_locations: usize,
    ) -> BmiResult<Self> {
        let (tx, rx) = mpsc::channel();
        let start_time = start_time.to_string();
        let thread_error: Arc<Mutex<Option<BmiError>>> = Arc::new(Mutex::new(None));
        let thread_error_clone = Arc::clone(&thread_error);

        let handle = thread::spawn(move || {
            let result = writer_thread(rx, path, &start_time, interval, total_steps, n_locations);
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

/// Output file plus its append position, created lazily from the first batch
/// (the variable names are only known once a result arrives).
struct OpenFile {
    file: netcdf::FileMut,
    var_names: Vec<String>,
    n_times: usize,
    n_locations: usize,
    next_idx: usize,
}

fn writer_thread(
    receiver: Receiver<WriterMessage>,
    path: PathBuf,
    start_time: &str,
    interval: i64,
    total_steps: usize,
    n_locations: usize,
) -> BmiResult<()> {
    suppress_hdf5_errors();

    let mut out: Option<OpenFile> = None;
    let mut batch: Vec<LocationResult> = Vec::with_capacity(BATCH_SIZE);

    let flush = |out: &mut Option<OpenFile>, batch: &mut Vec<LocationResult>| -> BmiResult<()> {
        if batch.is_empty() {
            return Ok(());
        }
        if out.is_none() {
            *out = Some(init_netcdf(
                &path,
                start_time,
                interval,
                total_steps,
                n_locations,
                &batch[0],
            )?);
        }
        write_batch(out.as_mut().unwrap(), batch)?;
        batch.clear();
        Ok(())
    };

    // Block until each result arrives; only touch the file once a full batch is ready.
    while let Ok(WriterMessage::Write(result)) = receiver.recv() {
        batch.push(result);
        if batch.len() >= BATCH_SIZE {
            flush(&mut out, &mut batch)?;
        }
    }
    flush(&mut out, &mut batch)
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

fn init_netcdf(
    path: &Path,
    start_time: &str,
    interval: i64,
    total_steps: usize,
    n_locations: usize,
    first_result: &LocationResult,
) -> BmiResult<OpenFile> {
    let n_times = total_steps + 1;
    let time_values: Vec<f64> = (0..n_times).map(|i| (i as f64) * interval as f64).collect();
    // Output variables — discover from first result
    let var_names: Vec<String> = first_result
        .columns
        .iter()
        .map(|(name, _)| name.clone())
        .collect();

    let file = create_file(
        path,
        n_times,
        n_locations,
        &format!("seconds since {}", start_time),
        &time_values,
        &var_names,
    )?;

    Ok(OpenFile {
        file,
        var_names,
        n_times,
        n_locations,
        next_idx: 0,
    })
}

fn write_batch(out: &mut OpenFile, batch: &[LocationResult]) -> BmiResult<()> {
    let start_idx = out.next_idx;
    let end_idx = start_idx + batch.len();
    if end_idx > out.n_locations {
        return Err(err(format!(
            "Received {} locations but the file was sized for {}",
            end_idx, out.n_locations
        )));
    }

    // Write location IDs (the netcdf crate has no bulk string write)
    let mut ids_var = out
        .file
        .variable_mut("id")
        .ok_or_else(|| err("id variable not found".into()))?;
    for (i, result) in batch.iter().enumerate() {
        ids_var
            .put_string(&result.location_id, start_idx + i)
            .map_err(|e| {
                err(format!(
                    "Failed to write id '{}': {}",
                    result.location_id, e
                ))
            })?;
    }

    // Concatenating per-location rows gives row-major (id, time) order, matching the
    // variable layout, so each variable is written for the whole batch in one call.
    // Rows are padded/truncated to n_times; missing variables are left as fill.
    let n_times = out.n_times;
    let mut data: Vec<f32> = Vec::with_capacity(batch.len() * n_times);
    for var_name in &out.var_names {
        data.clear();
        for result in batch {
            let row_start = data.len();
            if let Some((_, vals)) = result.columns.iter().find(|(n, _)| n == var_name) {
                data.extend(vals.iter().take(n_times).map(|&v| v as f32));
            }
            data.resize(row_start + n_times, FILL_VALUE);
        }
        let mut var = out
            .file
            .variable_mut(var_name)
            .ok_or_else(|| err(format!("Variable '{}' not found in output file", var_name)))?;
        var.put_values(&data, (start_idx..end_idx, ..))
            .map_err(|e| err(format!("Failed to write '{}': {}", var_name, e)))?;
    }

    out.next_idx = end_idx;
    Ok(())
}

/// Merge multiple per-worker NetCDF files into a single output file.
pub fn merge_netcdf_files(worker_files: &[PathBuf], output_path: &Path) -> BmiResult<()> {
    suppress_hdf5_errors();

    if worker_files.is_empty() {
        return Ok(());
    }

    // If only one worker file, just rename it
    if worker_files.len() == 1 {
        std::fs::rename(&worker_files[0], output_path).map_err(|e| {
            err(format!(
                "Failed to rename {} to {}: {}",
                worker_files[0].display(),
                output_path.display(),
                e
            ))
        })?;
        return Ok(());
    }

    let sources: Vec<netcdf::File> = worker_files
        .iter()
        .map(|p| netcdf::open(p).map_err(|e| err(format!("Failed to open {}: {}", p.display(), e))))
        .collect::<BmiResult<_>>()?;
    let first = &sources[0];

    let n_times = first
        .dimension("time")
        .ok_or_else(|| err("time dimension not found in worker file".into()))?
        .len();

    // Read time values and attributes
    let time_var = first
        .variable("time")
        .ok_or_else(|| err("time variable not found".into()))?;
    let time_values: Vec<f64> = time_var
        .get_values(..)
        .map_err(|e| err(format!("Failed to read time values: {}", e)))?;
    let time_units: String = time_var
        .attribute("units")
        .and_then(|a| a.value().ok())
        .map(|v| match v {
            netcdf::AttributeValue::Str(s) => s,
            _ => String::new(),
        })
        .unwrap_or_default();

    // Discover output variable names (everything except time and id)
    let var_names: Vec<String> = first
        .variables()
        .map(|v| v.name())
        .filter(|name| name != "time" && name != "id")
        .collect();

    // Total location count is known up front, so the id dimension can be fixed-size.
    let loc_counts: Vec<usize> = sources
        .iter()
        .map(|f| f.dimension("id").map(|d| d.len()).unwrap_or(0))
        .collect();
    let total_locs: usize = loc_counts.iter().sum();

    let mut merged = create_file(
        output_path,
        n_times,
        total_locs,
        &time_units,
        &time_values,
        &var_names,
    )?;

    let rows_per_block = (MERGE_BLOCK_VALUES / n_times.max(1)).max(1);

    // Copy data from each worker file
    let mut out_idx = 0usize;
    for (src, &n_locs) in sources.iter().zip(&loc_counts) {
        if n_locs == 0 {
            continue;
        }

        // Copy location IDs
        let ids_src = src
            .variable("id")
            .ok_or_else(|| err("id variable not found in worker file".into()))?;
        let mut ids_dst = merged
            .variable_mut("id")
            .ok_or_else(|| err("id variable not found in merged file".into()))?;
        for i in 0..n_locs {
            let id = ids_src
                .get_string(i)
                .map_err(|e| err(format!("Failed to read id at {}: {}", i, e)))?;
            ids_dst
                .put_string(&id, out_idx + i)
                .map_err(|e| err(format!("Failed to write id '{}': {}", id, e)))?;
        }

        // Copy each variable in large contiguous row blocks
        for name in &var_names {
            let Some(src_var) = src.variable(name) else {
                continue;
            };
            let mut dst_var = merged
                .variable_mut(name)
                .ok_or_else(|| err(format!("'{}' not found in merged file", name)))?;
            for block_start in (0..n_locs).step_by(rows_per_block) {
                let block_end = (block_start + rows_per_block).min(n_locs);
                let block: Vec<f32> = src_var
                    .get_values((block_start..block_end, ..))
                    .map_err(|e| err(format!("Failed to read '{}': {}", name, e)))?;
                dst_var
                    .put_values(&block, (out_idx + block_start..out_idx + block_end, ..))
                    .map_err(|e| err(format!("Failed to write '{}': {}", name, e)))?;
            }
        }

        out_idx += n_locs;
    }

    drop(sources);
    // Remove temporary worker files
    for worker_path in worker_files {
        let _ = std::fs::remove_file(worker_path);
    }

    Ok(())
}
