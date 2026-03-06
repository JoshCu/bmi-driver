use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

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

pub struct LocationResult {
    pub location_id: String,
    pub columns: Vec<(String, Vec<f64>)>,
}

enum WriterMessage {
    Write(LocationResult),
    Shutdown,
}

pub struct NetCdfWriter {
    sender: Sender<WriterMessage>,
    handle: Option<JoinHandle<BmiResult<()>>>,
    /// Shared error state: if the writer thread dies, the actual error is stored here
    /// so write() can report it instead of just "sending on a closed channel".
    thread_error: Arc<Mutex<Option<BmiError>>>,
}

impl NetCdfWriter {
    pub fn new(
        path: PathBuf,
        start_time: &str,
        interval: i64,
        total_steps: usize,
    ) -> BmiResult<Self> {
        let (tx, rx) = mpsc::channel();
        let start_time = start_time.to_string();
        let thread_error: Arc<Mutex<Option<BmiError>>> = Arc::new(Mutex::new(None));
        let thread_error_clone = Arc::clone(&thread_error);

        let handle = thread::spawn(move || {
            let result = writer_thread(rx, path, &start_time, interval, total_steps);
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
        self.sender
            .send(WriterMessage::Write(result))
            .map_err(|_| {
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

    pub fn finish(mut self) -> BmiResult<()> {
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

fn writer_thread(
    receiver: Receiver<WriterMessage>,
    path: PathBuf,
    start_time: &str,
    interval: i64,
    total_steps: usize,
) -> BmiResult<()> {
    suppress_hdf5_errors();

    let mut file: Option<netcdf::FileMut> = None;
    let mut var_names: Vec<String> = Vec::new();
    let mut location_count: usize = 0;
    let mut batch: Vec<LocationResult> = Vec::new();
    let batch_size = 50;

    loop {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(WriterMessage::Write(result)) => {
                batch.push(result);
                if batch.len() >= batch_size {
                    if file.is_none() {
                        let (f, names) =
                            init_netcdf(&path, start_time, interval, total_steps, &batch[0])?;
                        file = Some(f);
                        var_names = names;
                    }
                    location_count =
                        write_batch(file.as_mut().unwrap(), &batch, &var_names, location_count)?;
                    batch.clear();
                }
            }
            Ok(WriterMessage::Shutdown) => {
                if !batch.is_empty() {
                    if file.is_none() {
                        let (f, names) =
                            init_netcdf(&path, start_time, interval, total_steps, &batch[0])?;
                        file = Some(f);
                        var_names = names;
                    }
                    write_batch(file.as_mut().unwrap(), &batch, &var_names, location_count)?;
                }
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !batch.is_empty() {
                    if file.is_none() {
                        let (f, names) =
                            init_netcdf(&path, start_time, interval, total_steps, &batch[0])?;
                        file = Some(f);
                        var_names = names;
                    }
                    location_count =
                        write_batch(file.as_mut().unwrap(), &batch, &var_names, location_count)?;
                    batch.clear();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if !batch.is_empty() {
                    if file.is_none() {
                        let (f, names) =
                            init_netcdf(&path, start_time, interval, total_steps, &batch[0])?;
                        file = Some(f);
                        var_names = names;
                    }
                    write_batch(file.as_mut().unwrap(), &batch, &var_names, location_count)?;
                }
                break;
            }
        }
    }
    Ok(())
}

fn init_netcdf(
    path: &Path,
    start_time: &str,
    interval: i64,
    total_steps: usize,
    first_result: &LocationResult,
) -> BmiResult<(netcdf::FileMut, Vec<String>)> {
    let n_times = total_steps + 1;

    let mut file = netcdf::create(path)
        .map_err(|e| err(format!("Failed to create {}: {}", path.display(), e)))?;

    file.add_dimension("time", n_times)
        .map_err(|e| err(format!("Failed to add time dimension: {}", e)))?;
    file.add_unlimited_dimension("id")
        .map_err(|e| err(format!("Failed to add id dimension: {}", e)))?;

    // Time variable
    let time_values: Vec<f64> = (0..n_times).map(|i| (i as f64) * interval as f64).collect();
    let mut time_var = file
        .add_variable::<f64>("time", &["time"])
        .map_err(|e| err(format!("Failed to add time variable: {}", e)))?;
    time_var
        .put_attribute("units", format!("seconds since {}", start_time))
        .map_err(|e| err(format!("Failed to set time units: {}", e)))?;
    time_var
        .put_attribute("long_name", "valid output time")
        .map_err(|e| err(format!("Failed to set time long_name: {}", e)))?;
    time_var
        .put_values(&time_values, ..)
        .map_err(|e| err(format!("Failed to write time values: {}", e)))?;

    // Location ID coordinate variable (same name as dimension for xarray .sel() support)
    file.add_string_variable("id", &["id"])
        .map_err(|e| err(format!("Failed to add id variable: {}", e)))?;

    // Output variables — discover from first result
    let var_names: Vec<String> = first_result
        .columns
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    for name in &var_names {
        let mut var = file
            .add_variable::<f32>(name, &["id", "time"])
            .map_err(|e| err(format!("Failed to add variable '{}': {}", name, e)))?;
        var.put_attribute("_FillValue", -9999.0f32)
            .map_err(|e| err(format!("Failed to set _FillValue on '{}': {}", name, e)))?;
    }

    Ok((file, var_names))
}

fn write_batch(
    file: &mut netcdf::FileMut,
    batch: &[LocationResult],
    var_names: &[String],
    start_idx: usize,
) -> BmiResult<usize> {
    // Build lookup for each result's columns
    let batch_maps: Vec<HashMap<&str, &Vec<f64>>> = batch
        .iter()
        .map(|r| {
            r.columns
                .iter()
                .map(|(name, vals)| (name.as_str(), vals))
                .collect()
        })
        .collect();

    // Write location IDs
    let mut ids_var = file
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

    // Write each output variable
    for var_name in var_names {
        let mut var = file
            .variable_mut(var_name)
            .ok_or_else(|| err(format!("Variable '{}' not found in output file", var_name)))?;
        for (i, batch_map) in batch_maps.iter().enumerate() {
            if let Some(vals) = batch_map.get(var_name.as_str()) {
                let f32_vals: Vec<f32> = vals.iter().map(|&v| v as f32).collect();
                var.put_values(&f32_vals, (start_idx + i, ..))
                    .map_err(|e| {
                        err(format!(
                            "Failed to write '{}' for batch item {}: {}",
                            var_name, i, e
                        ))
                    })?;
            }
        }
    }

    Ok(start_idx + batch.len())
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

    // Open first file to get structure
    let first = netcdf::open(&worker_files[0]).map_err(|e| {
        err(format!(
            "Failed to open {}: {}",
            worker_files[0].display(),
            e
        ))
    })?;

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
        .filter(|v| {
            let name = v.name();
            name != "time" && name != "id"
        })
        .map(|v| v.name())
        .collect();

    drop(first);

    // Create merged file
    let mut merged = netcdf::create(output_path)
        .map_err(|e| err(format!("Failed to create merged file: {}", e)))?;

    merged
        .add_dimension("time", n_times)
        .map_err(|e| err(format!("Failed to add time dim: {}", e)))?;
    merged
        .add_unlimited_dimension("id")
        .map_err(|e| err(format!("Failed to add id dim: {}", e)))?;

    let mut time_out = merged
        .add_variable::<f64>("time", &["time"])
        .map_err(|e| err(format!("Failed to add time var: {}", e)))?;
    if !time_units.is_empty() {
        time_out
            .put_attribute("units", time_units)
            .map_err(|e| err(format!("Failed to set time units: {}", e)))?;
    }
    time_out
        .put_attribute("long_name", "valid output time")
        .map_err(|e| err(format!("Failed to set time long_name: {}", e)))?;
    time_out
        .put_values(&time_values, ..)
        .map_err(|e| err(format!("Failed to write time: {}", e)))?;

    merged
        .add_string_variable("id", &["id"])
        .map_err(|e| err(format!("Failed to add id var: {}", e)))?;

    for name in &var_names {
        let mut var = merged
            .add_variable::<f32>(name, &["id", "time"])
            .map_err(|e| err(format!("Failed to add var '{}': {}", name, e)))?;
        var.put_attribute("_FillValue", -9999.0f32)
            .map_err(|e| err(format!("Failed to set _FillValue: {}", e)))?;
    }

    // Copy data from each worker file
    let mut out_idx = 0usize;
    for worker_path in worker_files {
        let src = netcdf::open(worker_path)
            .map_err(|e| err(format!("Failed to open {}: {}", worker_path.display(), e)))?;

        let n_locs = src.dimension("id").map(|d| d.len()).unwrap_or(0);
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

        // Copy each variable
        for name in &var_names {
            if let Some(src_var) = src.variable(name) {
                let mut dst_var = merged
                    .variable_mut(name)
                    .ok_or_else(|| err(format!("'{}' not found in merged file", name)))?;
                for i in 0..n_locs {
                    let row: Vec<f32> = src_var
                        .get_values((i, ..))
                        .map_err(|e| err(format!("Failed to read '{}' row {}: {}", name, i, e)))?;
                    dst_var.put_values(&row, (out_idx + i, ..)).map_err(|e| {
                        err(format!(
                            "Failed to write '{}' row {}: {}",
                            name,
                            out_idx + i,
                            e
                        ))
                    })?;
                }
            }
        }

        out_idx += n_locs;
        drop(src);

        // Remove temporary worker file
        let _ = std::fs::remove_file(worker_path);
    }

    Ok(())
}
