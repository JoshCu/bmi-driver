use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use arrow_array::{
    ArrayRef, Float32Array, Int64Array, RecordBatch, StringArray, TimestampMillisecondArray,
};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::arrow_writer::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;

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
pub struct ParquetWriter {
    sender: Sender<WriterMessage>,
    handle: Option<JoinHandle<BmiResult<()>>>,
    thread_error: Arc<Mutex<Option<BmiError>>>,
}

impl ParquetWriter {
    pub fn new(path: PathBuf, start_epoch: i64, interval: i64) -> BmiResult<Self> {
        let (tx, rx) = mpsc::channel();
        let thread_error: Arc<Mutex<Option<BmiError>>> = Arc::new(Mutex::new(None));
        let thread_error_clone = Arc::clone(&thread_error);

        let handle = thread::spawn(move || {
            let result = writer_thread(rx, path, start_epoch, interval);
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

fn writer_properties() -> WriterProperties {
    WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::default()))
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
        let n_steps = result.columns.first().map(|(_, v)| v.len()).unwrap_or(0);
        for step in 0..n_steps {
            ids.push(result.location_id.clone());
            time_steps.push(step as i64);
            times.push((start_epoch + (step as i64) * interval) * 1000);
            for (i, name) in var_names.iter().enumerate() {
                let val = result
                    .columns
                    .iter()
                    .find(|(col_name, _)| col_name == name)
                    .map(|(_, vals)| vals[step] as f32)
                    .unwrap_or(f32::NAN);
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
) -> BmiResult<()> {
    let mut writer: Option<ArrowWriter<File>> = None;
    let mut schema: Option<Arc<Schema>> = None;
    let mut var_names: Vec<String> = Vec::new();
    let mut batch: Vec<LocationResult> = Vec::new();
    // Larger batches produce fewer, bigger row groups, which compress better.
    let batch_size = 500;

    let flush_batch = |writer: &mut Option<ArrowWriter<File>>,
                       schema: &mut Option<Arc<Schema>>,
                       var_names: &mut Vec<String>,
                       batch: &mut Vec<LocationResult>|
     -> BmiResult<()> {
        if batch.is_empty() {
            return Ok(());
        }
        if writer.is_none() {
            let names: Vec<String> = batch[0].columns.iter().map(|(n, _)| n.clone()).collect();
            let s = build_schema(&names);
            let file = File::create(&path)
                .map_err(|e| err(format!("Failed to create {}: {}", path.display(), e)))?;
            let w = ArrowWriter::try_new(file, s.clone(), Some(writer_properties()))
                .map_err(|e| err(format!("Failed to create writer: {}", e)))?;
            *var_names = names;
            *schema = Some(s);
            *writer = Some(w);
        }
        let record_batch = build_batch(
            schema.as_ref().unwrap(),
            var_names,
            batch,
            start_epoch,
            interval,
        )?;
        writer
            .as_mut()
            .unwrap()
            .write(&record_batch)
            .map_err(|e| err(format!("Failed to write batch: {}", e)))?;
        batch.clear();
        Ok(())
    };

    loop {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(WriterMessage::Write(result)) => {
                batch.push(result);
                if batch.len() >= batch_size {
                    flush_batch(&mut writer, &mut schema, &mut var_names, &mut batch)?;
                }
            }
            Ok(WriterMessage::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                flush_batch(&mut writer, &mut schema, &mut var_names, &mut batch)?;
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                flush_batch(&mut writer, &mut schema, &mut var_names, &mut batch)?;
            }
        }
    }

    if let Some(w) = writer {
        w.close()
            .map_err(|e| err(format!("Failed to close writer: {}", e)))?;
    }
    Ok(())
}

/// Merge multiple per-worker Parquet files into a single output file.
pub fn merge_parquet_files(worker_files: &[PathBuf], output_path: &Path) -> BmiResult<()> {
    if worker_files.is_empty() {
        return Ok(());
    }

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

    let first_file = File::open(&worker_files[0]).map_err(|e| {
        err(format!(
            "Failed to open {}: {}",
            worker_files[0].display(),
            e
        ))
    })?;
    let schema = ParquetRecordBatchReaderBuilder::try_new(first_file)
        .map_err(|e| err(format!("Failed to read schema: {}", e)))?
        .schema()
        .clone();

    let out_file = File::create(output_path)
        .map_err(|e| err(format!("Failed to create {}: {}", output_path.display(), e)))?;
    let mut writer = ArrowWriter::try_new(out_file, schema, Some(writer_properties()))
        .map_err(|e| err(format!("Failed to create merged writer: {}", e)))?;

    for worker_path in worker_files {
        let file = File::open(worker_path)
            .map_err(|e| err(format!("Failed to open {}: {}", worker_path.display(), e)))?;
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .map_err(|e| {
                err(format!(
                    "Failed to open reader for {}: {}",
                    worker_path.display(),
                    e
                ))
            })?
            .build()
            .map_err(|e| {
                err(format!(
                    "Failed to build reader for {}: {}",
                    worker_path.display(),
                    e
                ))
            })?;
        for batch in reader {
            let batch = batch.map_err(|e| err(format!("Failed to read batch: {}", e)))?;
            writer
                .write(&batch)
                .map_err(|e| err(format!("Failed to write merged batch: {}", e)))?;
        }
        let _ = std::fs::remove_file(worker_path);
    }

    writer
        .close()
        .map_err(|e| err(format!("Failed to close merged writer: {}", e)))?;
    Ok(())
}
