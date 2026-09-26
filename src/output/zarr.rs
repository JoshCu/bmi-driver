use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use zarrs::array::codec::bytes_to_bytes::zstd::ZstdCodec;
use zarrs::array::{data_type, Array, ArrayBuilder, ArraySubset, BytesToBytesCodecTraits, ZARR_NAN_F32};
use zarrs::filesystem::FilesystemStore;
use zarrs::group::GroupBuilder;
use zarrs::storage::ReadableWritableListableStorage;

use super::LocationResult;
use crate::error::{function_failed, BmiError, BmiResult};

fn err(msg: String) -> BmiError {
    function_failed("zarr_writer", msg)
}

/// Uncompressed size a chunk of one variable should aim for.
const TARGET_CHUNK_BYTES: u64 = 8 * 1024 * 1024;
/// Upper bound on locations per chunk, so short runs don't produce huge chunks.
const MAX_CHUNK_LOCATIONS: u64 = 1024;
const ZSTD_LEVEL: i32 = 3;

/// Root group attribute listing the output variable arrays, so workers can
/// open them all up front without listing the store.
const VARIABLES_ATTR: &str = "variables";

/// Pick how many locations go in one chunk of each output variable.
///
/// Aims for roughly `TARGET_CHUNK_BYTES` of f32 per chunk, but never more
/// than `per_worker` locations, so every worker still gets at least one chunk.
pub fn chunk_locations(total_steps: usize, per_worker: usize) -> usize {
    let n_times = (total_steps + 1) as u64;
    let by_size = (TARGET_CHUNK_BYTES / (n_times * 4)).clamp(1, MAX_CHUNK_LOCATIONS);
    (by_size as usize).min(per_worker.max(1))
}

fn zstd() -> Vec<Arc<dyn BytesToBytesCodecTraits>> {
    vec![Arc::new(ZstdCodec::new(ZSTD_LEVEL, false))]
}

fn open_store(path: &Path) -> BmiResult<ReadableWritableListableStorage> {
    Ok(Arc::new(
        FilesystemStore::new(path).map_err(|e| err(format!("Failed to open store: {e}")))?,
    ))
}

/// Create a Zarr V3 store with pre-allocated arrays for all output variables.
///
/// Called by the parent process before spawning workers. The store contains:
/// - `time`: f64[n_times] — seconds since start_time
/// - `id`: string[n_locations] — location identifiers (coordinate variable)
/// - One f32[n_locations, n_times] array per output variable
///
/// Output variables are chunked as `[chunk_locations, n_times]` and zstd
/// compressed. The parent aligns each worker's block of locations to
/// `chunk_locations`, so every chunk is written by exactly one worker and
/// workers can write concurrently without locking.
pub fn create_zarr_store(
    path: &Path,
    start_time: &str,
    interval: i64,
    total_steps: usize,
    location_ids: &[String],
    var_names: &[String],
    chunk_locations: usize,
) -> BmiResult<()> {
    let n_times = (total_steps + 1) as u64;
    let n_locations = location_ids.len() as u64;
    let chunk_locations = chunk_locations.max(1) as u64;

    let store = open_store(path)?;

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
    group
        .attributes_mut()
        .insert(VARIABLES_ATTR.into(), serde_json::json!(var_names));
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

    // ID array: string[n_locations] in a single chunk, written once here.
    // Readers load coordinates eagerly, so one chunk is the cheapest to read.
    let id_array = ArrayBuilder::new(
        vec![n_locations],
        vec![n_locations.max(1)],
        data_type::string(),
        "",
    )
    .bytes_to_bytes_codecs(zstd())
    .dimension_names(["id"].into())
    .build(store.clone(), "/id")
    .map_err(|e| err(format!("Failed to create id array: {e}")))?;
    id_array.store_metadata().map_err(|e| err(format!("Failed to store id metadata: {e}")))?;

    if n_locations > 0 {
        let ids: Vec<&str> = location_ids.iter().map(String::as_str).collect();
        id_array
            .store_array_subset(&id_array.subset_all(), &ids)
            .map_err(|e| err(format!("Failed to write ids: {e}")))?;
    }

    // Output variable arrays: f32[n_locations, n_times], chunk=[chunk_locations, n_times]
    for name in var_names {
        let var_array = ArrayBuilder::new(
            vec![n_locations, n_times],
            vec![chunk_locations, n_times],
            data_type::float32(),
            ZARR_NAN_F32,
        )
        .bytes_to_bytes_codecs(zstd())
        .dimension_names(["id", "time"].into())
        .build(store.clone(), &format!("/{name}"))
        .map_err(|e| err(format!("Failed to create variable '{name}': {e}")))?;
        var_array
            .store_metadata()
            .map_err(|e| err(format!("Failed to store metadata for '{name}': {e}")))?;
    }

    Ok(())
}

type ZarrArray = Array<dyn zarrs::storage::ReadableWritableListableStorageTraits>;

/// The open arrays of a store, plus its layout.
struct Layout {
    names: Vec<String>,
    arrays: Vec<ZarrArray>,
    n_locations: u64,
    n_times: usize,
    chunk_locations: u64,
}

impl Layout {
    fn open(path: &Path) -> BmiResult<Self> {
        let store = open_store(path)?;
        let group = zarrs::group::Group::open(store.clone(), "/")
            .map_err(|e| err(format!("Failed to open root group: {e}")))?;
        let names: Vec<String> = group
            .attributes()
            .get(VARIABLES_ATTR)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .ok_or_else(|| err(format!("Root group has no '{VARIABLES_ATTR}' attribute")))?;

        let arrays = names
            .iter()
            .map(|name| {
                Array::open(store.clone(), &format!("/{name}"))
                    .map_err(|e| err(format!("Failed to open array '{name}': {e}")))
            })
            .collect::<BmiResult<Vec<_>>>()?;

        let (n_locations, n_times, chunk_locations) = match arrays.first() {
            Some(a) => {
                let chunk = a
                    .chunk_shape(&[0, 0])
                    .map_err(|e| err(format!("Failed to read chunk shape: {e}")))?;
                (a.shape()[0], a.shape()[1] as usize, chunk[0].get())
            }
            None => (0, 0, 1),
        };

        Ok(Self {
            names,
            arrays,
            n_locations,
            n_times,
            chunk_locations,
        })
    }

    /// Write `rows` consecutive locations starting at global index `start`.
    ///
    /// When the rows cover a whole chunk (or the in-bounds part of the last,
    /// partial chunk), each variable is written with one `store_chunk`.
    /// Otherwise it falls back to a subset write, which is only safe when no
    /// other worker writes the same chunk.
    fn write_block(&self, start: u64, rows: &[LocationResult]) -> BmiResult<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let n_rows = rows.len() as u64;
        let b = self.chunk_locations;
        let whole_chunk =
            start.is_multiple_of(b) && (n_rows == b || start + n_rows == self.n_locations);
        let buf_rows = if whole_chunk { b } else { n_rows } as usize;

        for (name, array) in self.names.iter().zip(&self.arrays) {
            // Rectangular buffer: missing variables, short series and the
            // padding past the end of the array stay NaN.
            let mut buf = vec![f32::NAN; buf_rows * self.n_times];
            for (r, result) in rows.iter().enumerate() {
                if let Some((_, vals)) = result.columns.iter().find(|(n, _)| n == name) {
                    let row = &mut buf[r * self.n_times..(r + 1) * self.n_times];
                    for (dst, &v) in row.iter_mut().zip(vals) {
                        *dst = v as f32;
                    }
                }
            }

            let res = if whole_chunk {
                array.store_chunk(&[start / b, 0], &buf)
            } else {
                let subset =
                    ArraySubset::new_with_ranges(&[start..start + n_rows, 0..self.n_times as u64]);
                array.store_array_subset(&subset, &buf)
            };
            res.map_err(|e| {
                err(format!(
                    "Failed to write '{name}' for locations {start}..{}: {e}",
                    start + n_rows
                ))
            })?;
        }
        Ok(())
    }
}

enum WriterMessage {
    Write(LocationResult),
    Shutdown,
}

/// Worker-side writer for a store created by [`create_zarr_store`].
///
/// Locations are buffered on a background thread and written a whole chunk
/// at a time, so the compute thread never waits on compression or I/O.
pub struct ZarrStore {
    sender: Sender<WriterMessage>,
    handle: Option<JoinHandle<BmiResult<()>>>,
    thread_error: Arc<Mutex<Option<BmiError>>>,
}

impl ZarrStore {
    /// Open the store at `path`. The first location written goes to global
    /// index `start_idx`, and each later one to the next index.
    pub fn new(path: PathBuf, start_idx: usize) -> BmiResult<Self> {
        let layout = Layout::open(&path)?;
        let (tx, rx) = mpsc::channel();
        let thread_error: Arc<Mutex<Option<BmiError>>> = Arc::new(Mutex::new(None));
        let thread_error_clone = Arc::clone(&thread_error);

        let handle = thread::spawn(move || {
            let result = writer_thread(rx, layout, start_idx as u64);
            if let Err(ref e) = result {
                if let Ok(mut guard) = thread_error_clone.lock() {
                    *guard = Some(err(format!("{e}")));
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
            if let Ok(guard) = self.thread_error.lock() {
                if let Some(ref e) = *guard {
                    return err(format!("Writer thread failed: {e}"));
                }
            }
            err("Writer thread exited unexpectedly".into())
        })
    }

    fn shutdown(&mut self) -> BmiResult<()> {
        let _ = self.sender.send(WriterMessage::Shutdown);
        match self.handle.take() {
            Some(handle) => handle
                .join()
                .map_err(|_| err("Writer thread panicked".into()))?,
            None => Ok(()),
        }
    }
}

impl Drop for ZarrStore {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

impl super::DivideDataStore for ZarrStore {
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

fn writer_thread(receiver: Receiver<WriterMessage>, layout: Layout, start: u64) -> BmiResult<()> {
    let b = layout.chunk_locations;
    // Global index of the first buffered location.
    let mut block_start = start;
    let mut buffer: Vec<LocationResult> = Vec::new();

    while let Ok(WriterMessage::Write(result)) = receiver.recv() {
        let idx = block_start + buffer.len() as u64;
        if idx >= layout.n_locations {
            return Err(err(format!(
                "Location index {idx} is past the end of the store ({} locations)",
                layout.n_locations
            )));
        }
        buffer.push(result);
        // Flush at each chunk boundary and at the end of the array.
        if (idx + 1).is_multiple_of(b) || idx + 1 == layout.n_locations {
            layout.write_block(block_start, &buffer)?;
            block_start += buffer.len() as u64;
            buffer.clear();
        }
    }

    layout.write_block(block_start, &buffer)
}
