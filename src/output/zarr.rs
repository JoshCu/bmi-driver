use std::path::Path;
use std::sync::Arc;

use zarrs::array::{data_type, ArrayBuilder, ZARR_NAN_F32};
use zarrs::filesystem::FilesystemStore;
use zarrs::group::GroupBuilder;
use zarrs::storage::ReadableWritableListableStorage;

use crate::error::{function_failed, BmiError, BmiResult};

fn err(msg: String) -> BmiError {
    function_failed("zarr_writer", msg)
}

/// Create a Zarr V3 store with pre-allocated arrays for all output variables.
///
/// Called by the parent process before spawning workers. The store contains:
/// - `time`: f64[n_times] — seconds since start_time
/// - `id`: string[n_locations] — location identifiers (coordinate variable)
/// - One f32[n_locations, n_times] array per output variable
///
/// Chunk layout: [1, n_times] so each location is an independent chunk,
/// enabling safe concurrent writes from multiple worker processes.
pub fn create_zarr_store(
    path: &Path,
    start_time: &str,
    interval: i64,
    total_steps: usize,
    location_ids: &[String],
    var_names: &[String],
) -> BmiResult<()> {
    let n_times = (total_steps + 1) as u64;
    let n_locations = location_ids.len() as u64;

    let store: ReadableWritableListableStorage =
        Arc::new(FilesystemStore::new(path).map_err(|e| err(format!("Failed to create store: {e}")))?);

    // Root group
    let mut group = GroupBuilder::new()
        .build(store.clone(), "/")
        .map_err(|e| err(format!("Failed to create root group: {e}")))?;
    group
        .attributes_mut()
        .insert("start_time".into(), serde_json::Value::String(start_time.to_string()));
    group
        .attributes_mut()
        .insert("interval_seconds".into(), serde_json::json!(interval));
    group.store_metadata().map_err(|e| err(format!("Failed to store root metadata: {e}")))?;

    // Time array: f64[n_times]
    let time_array = ArrayBuilder::new(
        vec![n_times],
        vec![n_times], // single chunk for time
        data_type::float64(),
        f64::NAN,
    )
    .dimension_names(["time"].into())
    .build(store.clone(), "/time")
    .map_err(|e| err(format!("Failed to create time array: {e}")))?;
    time_array.store_metadata().map_err(|e| err(format!("Failed to store time metadata: {e}")))?;

    let time_values: Vec<f64> = (0..n_times).map(|i| (i as f64) * interval as f64).collect();
    time_array
        .store_chunk(&[0], &time_values)
        .map_err(|e| err(format!("Failed to write time values: {e}")))?;

    // ID array: string[n_locations], chunk=[1] for parallel writes
    let id_array = ArrayBuilder::new(
        vec![n_locations],
        vec![1],
        data_type::string(),
        "",
    )
    .dimension_names(["id"].into())
    .build(store.clone(), "/id")
    .map_err(|e| err(format!("Failed to create id array: {e}")))?;
    id_array.store_metadata().map_err(|e| err(format!("Failed to store id metadata: {e}")))?;

    // Write location IDs one chunk (one id) at a time
    for (i, id) in location_ids.iter().enumerate() {
        id_array
            .store_chunk(&[i as u64], &[id.as_str()])
            .map_err(|e| err(format!("Failed to write id '{}': {e}", id)))?;
    }

    // Output variable arrays: f32[n_locations, n_times], chunk=[1, n_times]
    for name in var_names {
        let var_array = ArrayBuilder::new(
            vec![n_locations, n_times],
            vec![1, n_times],
            data_type::float32(),
            ZARR_NAN_F32,
        )
        .dimension_names(["id", "time"].into())
        .build(store.clone(), &format!("/{name}"))
        .map_err(|e| err(format!("Failed to create variable '{name}': {e}")))?;
        var_array
            .store_metadata()
            .map_err(|e| err(format!("Failed to store metadata for '{name}': {e}")))?;
    }

    Ok(())
}

/// Write one location's output data to a pre-created Zarr store.
///
/// Called by worker processes. Each call writes to chunk [location_idx, 0]
/// which maps to a unique file, so concurrent calls with different indices are safe.
pub fn write_location(
    path: &Path,
    location_idx: usize,
    columns: &[(String, Vec<f64>)],
) -> BmiResult<()> {
    let store: ReadableWritableListableStorage =
        Arc::new(FilesystemStore::new(path).map_err(|e| err(format!("Failed to open store: {e}")))?);

    for (name, values) in columns {
        let array = zarrs::array::Array::open(store.clone(), &format!("/{name}"))
            .map_err(|e| err(format!("Failed to open array '{name}': {e}")))?;

        let f32_vals: Vec<f32> = values.iter().map(|&v| v as f32).collect();
        array
            .store_chunk(&[location_idx as u64, 0], &f32_vals)
            .map_err(|e| err(format!("Failed to write '{name}' for location {location_idx}: {e}")))?;
    }

    Ok(())
}
