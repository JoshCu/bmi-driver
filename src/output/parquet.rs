use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use arrow_array::{
    ArrayRef, Float32Array, Int64Array, RecordBatch, StringArray, TimestampMillisecondArray,
};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use parquet::arrow::arrow_writer::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;
use parquet::schema::types::ColumnPath;

use super::LocationResult;
use crate::error::{function_failed, BmiError, BmiResult};

fn err(msg: String) -> BmiError {
    function_failed("parquet_writer", msg)
}

enum WriterMessage {
    Write(LocationResult),
    Shutdown,
}

/// Long-format Parquet writer: one row per (location, timestep), columns are
/// `id`, `time_step`, `time`, followed by one column per output variable.
///
/// Each worker writes its own file into a Parquet dataset directory
/// (`results.parquet/part-<worker>.parquet`), which pandas, polars, pyarrow,
/// DuckDB and Spark all read as one table, so no merge step is needed.
pub struct ParquetWriter {
    sender: Sender<WriterMessage>,
    handle: Option<JoinHandle<BmiResult<()>>>,
    thread_error: Arc<Mutex<Option<BmiError>>>,
}

impl ParquetWriter {
    /// `var_names` fixes the variable columns and their order, so every
    /// worker's file has the same schema. If it is empty, the columns of the
    /// first location written are used instead.
    pub fn new(
        path: PathBuf,
        start_epoch: i64,
        interval: i64,
        var_names: Vec<String>,
    ) -> BmiResult<Self> {
        let (tx, rx) = mpsc::channel();
        let thread_error: Arc<Mutex<Option<BmiError>>> = Arc::new(Mutex::new(None));
        let thread_error_clone = Arc::clone(&thread_error);

        let handle = thread::spawn(move || {
            let result = writer_thread(rx, path, start_epoch, interval, var_names);
            if let Err(ref e) = result {
                if let Ok(mut guard) = thread_error_clone.lock() {
                    *guard = Some(BmiError::FunctionFailed {
                        model: "parquet_writer".into(),
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
            if let Ok(guard) = self.thread_error.lock() {
                if let Some(ref e) = *guard {
                    return BmiError::FunctionFailed {
                        model: "parquet_writer".into(),
                        func: format!("Writer thread failed: {}", e),
                    };
                }
            }
            err("Writer thread exited unexpectedly".into())
        })
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

impl super::DivideDataStore for ParquetWriter {
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

fn build_schema(var_names: &[String]) -> Arc<Schema> {
    let mut fields = vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("time_step", DataType::Int64, false),
        Field::new(
            "time",
            DataType::Timestamp(TimeUnit::Millisecond, None),
            false,
        ),
    ];
    for name in var_names {
        fields.push(Field::new(name, DataType::Float32, true));
    }
    Arc::new(Schema::new(fields))
}

/// Rows per row group. Each location's rows are contiguous, so row-group
/// statistics on `id` let readers skip data when filtering by catchment.
const MAX_ROW_GROUP_SIZE: usize = 1024 * 1024;

fn writer_properties() -> WriterProperties {
    WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::default()))
        .set_max_row_group_row_count(Some(MAX_ROW_GROUP_SIZE))
        .set_column_dictionary_enabled(ColumnPath::from("id"), true)
        .build()
}

fn build_batch(
    schema: &Arc<Schema>,
    var_names: &[String],
    batch: &[LocationResult],
    start_epoch: i64,
    interval: i64,
) -> BmiResult<RecordBatch> {
    let mut ids: Vec<String> = Vec::new();
    let mut time_steps: Vec<i64> = Vec::new();
    let mut times: Vec<i64> = Vec::new();
    let mut var_values: Vec<Vec<f32>> = vec![Vec::new(); var_names.len()];

    for result in batch {
        // Look each variable up once per location, not once per row.
        let series: Vec<Option<&Vec<f64>>> = var_names
            .iter()
            .map(|name| {
                result
                    .columns
                    .iter()
                    .find(|(col_name, _)| col_name == name)
                    .map(|(_, vals)| vals)
            })
            .collect();
        // Series can differ in length; emit rows up to the longest one and
        // fill the gaps in shorter or missing series with NaN.
        let n_steps = series.iter().flatten().map(|v| v.len()).max().unwrap_or(0);
        for step in 0..n_steps {
            ids.push(result.location_id.clone());
            time_steps.push(step as i64);
            times.push((start_epoch + (step as i64) * interval) * 1000);
            for (i, vals) in series.iter().enumerate() {
                let val = vals
                    .and_then(|v| v.get(step))
                    .map_or(f32::NAN, |&v| v as f32);
                var_values[i].push(val);
            }
        }
    }

    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from_iter_values(ids.iter())),
        Arc::new(Int64Array::from(time_steps)),
        Arc::new(TimestampMillisecondArray::from(times)),
    ];
    for vals in var_values {
        columns.push(Arc::new(Float32Array::from(vals)));
    }

    RecordBatch::try_new(schema.clone(), columns)
        .map_err(|e| err(format!("Failed to build record batch: {}", e)))
}

fn writer_thread(
    receiver: Receiver<WriterMessage>,
    path: PathBuf,
    start_epoch: i64,
    interval: i64,
    mut var_names: Vec<String>,
) -> BmiResult<()> {
    let mut writer: Option<(ArrowWriter<File>, Arc<Schema>)> = None;
    let mut batch: Vec<LocationResult> = Vec::new();
    // Larger batches mean fewer, bigger writes; ArrowWriter buffers them
    // into row groups of MAX_ROW_GROUP_SIZE.
    let batch_size = 500;

    let mut flush_batch = |batch: &mut Vec<LocationResult>| -> BmiResult<()> {
        if batch.is_empty() {
            return Ok(());
        }
        if writer.is_none() {
            if var_names.is_empty() {
                var_names = batch[0].columns.iter().map(|(n, _)| n.clone()).collect();
            }
            let schema = build_schema(&var_names);
            let file = File::create(&path)
                .map_err(|e| err(format!("Failed to create {}: {}", path.display(), e)))?;
            let w = ArrowWriter::try_new(file, schema.clone(), Some(writer_properties()))
                .map_err(|e| err(format!("Failed to create writer: {}", e)))?;
            writer = Some((w, schema));
        }
        let (w, schema) = writer.as_mut().unwrap();
        let record_batch = build_batch(schema, &var_names, batch, start_epoch, interval)?;
        w.write(&record_batch)
            .map_err(|e| err(format!("Failed to write batch: {}", e)))?;
        batch.clear();
        Ok(())
    };

    // Block until the next result arrives; flush only when a batch is full
    // or at shutdown.
    while let Ok(WriterMessage::Write(result)) = receiver.recv() {
        batch.push(result);
        if batch.len() >= batch_size {
            flush_batch(&mut batch)?;
        }
    }
    flush_batch(&mut batch)?;

    if let Some((w, _)) = writer {
        w.close()
            .map_err(|e| err(format!("Failed to close writer: {}", e)))?;
    }
    Ok(())
}
