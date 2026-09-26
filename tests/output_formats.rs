use bmi_driver::output::DivideDataStore;
use std::path::Path;

const LOCATIONS: &[&str] = &["cat-100", "cat-200"];
const VAR_NAMES: &[&str] = &["Q_OUT", "EVAPOTRANS", "RAIN_RATE"];
const N_STEPS: usize = 25;
const START_TIME: &str = "2010-01-01 00:00:00";
const INTERVAL: i64 = 3600;

/// Generate deterministic test data: value = loc_idx * 1000 + var_idx * 100 + step + 0.123456789
fn generate_data() -> Vec<(String, Vec<(String, Vec<f64>)>)> {
    LOCATIONS
        .iter()
        .enumerate()
        .map(|(loc_idx, loc_id)| {
            let columns: Vec<(String, Vec<f64>)> = VAR_NAMES
                .iter()
                .enumerate()
                .map(|(var_idx, var_name)| {
                    let values: Vec<f64> = (0..N_STEPS)
                        .map(|step| loc_idx as f64 * 1000.0 + var_idx as f64 * 100.0 + step as f64 + 0.123456789)
                        .collect();
                    (var_name.to_string(), values)
                })
                .collect();
            (loc_id.to_string(), columns)
        })
        .collect()
}

fn write_all(store: &mut dyn DivideDataStore, data: &[(String, Vec<(String, Vec<f64>)>)]) {
    for (loc_id, columns) in data {
        store.write_location(loc_id, columns).unwrap();
    }
    store.finish().unwrap();
}

// --- CSV ---

fn read_csv_data(dir: &Path) -> Vec<(String, Vec<(String, Vec<f64>)>)> {
    LOCATIONS
        .iter()
        .map(|loc_id| {
            let csv_path = dir.join(format!("{}.csv", loc_id));
            let content = std::fs::read_to_string(&csv_path)
                .unwrap_or_else(|e| panic!("Failed to read {}: {}", csv_path.display(), e));
            let mut lines = content.lines();
            let header = lines.next().unwrap();
            // Header: "Time Step,Time,VAR1,VAR2,..."
            let var_names: Vec<String> = header.split(',').skip(2).map(|s| s.to_string()).collect();
            let mut columns: Vec<Vec<f64>> = vec![Vec::new(); var_names.len()];

            for line in lines {
                let fields: Vec<&str> = line.split(',').collect();
                for (i, val_str) in fields.iter().skip(2).enumerate() {
                    columns[i].push(val_str.parse::<f64>().unwrap());
                }
            }

            let result: Vec<(String, Vec<f64>)> = var_names
                .into_iter()
                .zip(columns)
                .collect();
            (loc_id.to_string(), result)
        })
        .collect()
}

#[test]
fn test_csv_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let data = generate_data();

    let start_epoch = 1262304000i64; // 2010-01-01 00:00:00 UTC
    let mut store = bmi_driver::output::csv::CsvStore::new(dir.path().to_path_buf(), start_epoch, INTERVAL);
    write_all(&mut store, &data);

    let read_back = read_csv_data(dir.path());
    assert_eq!(read_back.len(), data.len());

    for (orig, read) in data.iter().zip(read_back.iter()) {
        assert_eq!(orig.0, read.0, "location ID mismatch");
        assert_eq!(orig.1.len(), read.1.len(), "variable count mismatch");
        for (orig_col, read_col) in orig.1.iter().zip(read.1.iter()) {
            assert_eq!(orig_col.0, read_col.0, "variable name mismatch");
            assert_eq!(orig_col.1.len(), read_col.1.len(), "value count mismatch");
            for (i, (o, r)) in orig_col.1.iter().zip(read_col.1.iter()).enumerate() {
                // CSV writes 9 decimal places
                assert!(
                    (o - r).abs() < 1e-8,
                    "CSV value mismatch at step {} for {}: {} vs {}",
                    i, orig_col.0, o, r
                );
            }
        }
    }
}

// --- NetCDF ---

fn read_netcdf_data(path: &Path) -> Vec<(String, Vec<(String, Vec<f64>)>)> {
    let file = netcdf::open(path).unwrap();

    let n_locs = file.dimension("id").unwrap().len();
    let ids_var = file.variable("id").unwrap();

    // Discover variable names (everything except time and id)
    let var_names: Vec<String> = file
        .variables()
        .filter(|v| {
            let name = v.name();
            name != "time" && name != "id"
        })
        .map(|v| v.name())
        .collect();

    (0..n_locs)
        .map(|loc_idx| {
            let loc_id = ids_var.get_string(loc_idx).unwrap();
            let columns: Vec<(String, Vec<f64>)> = var_names
                .iter()
                .map(|name| {
                    let var = file.variable(name).unwrap();
                    let f32_vals: Vec<f32> = var.get_values((loc_idx, ..)).unwrap();
                    let f64_vals: Vec<f64> = f32_vals.iter().map(|&v| v as f64).collect();
                    (name.clone(), f64_vals)
                })
                .collect();
            (loc_id, columns)
        })
        .collect()
}

#[test]
fn test_netcdf_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let data = generate_data();

    let nc_path = dir.path().join("test.nc");
    let mut store = bmi_driver::output::netcdf::NetCdfWriter::new(
        nc_path.clone(),
        START_TIME,
        INTERVAL,
        N_STEPS - 1, // total_steps = N_STEPS - 1 because n_times = total_steps + 1
    )
    .unwrap();
    write_all(&mut store, &data);

    let read_back = read_netcdf_data(&nc_path);
    assert_eq!(read_back.len(), data.len());

    for (orig, read) in data.iter().zip(read_back.iter()) {
        assert_eq!(orig.0, read.0, "location ID mismatch");
        assert_eq!(orig.1.len(), read.1.len(), "variable count mismatch");
        for (orig_col, read_col) in orig.1.iter().zip(read.1.iter()) {
            assert_eq!(orig_col.0, read_col.0, "variable name mismatch");
            assert_eq!(orig_col.1.len(), read_col.1.len(), "value count mismatch for {}", orig_col.0);
            for (i, (o, r)) in orig_col.1.iter().zip(read_col.1.iter()).enumerate() {
                // NetCDF stores f32, so tolerance must account for f64→f32→f64 roundtrip
                assert!(
                    (o - r).abs() < 1e-2,
                    "NetCDF value mismatch at step {} for {}: {} vs {}",
                    i, orig_col.0, o, r
                );
            }
        }
    }
}

// --- Zarr ---

#[cfg(feature = "zarr")]
fn read_zarr_data(path: &Path) -> Vec<(String, Vec<(String, Vec<f64>)>)> {
    use std::sync::Arc;
    use zarrs::filesystem::FilesystemStore;
    use zarrs::storage::ReadableWritableListableStorage;

    let store: ReadableWritableListableStorage =
        Arc::new(FilesystemStore::new(path).unwrap());

    let id_array = zarrs::array::Array::open(store.clone(), "/id").unwrap();
    let n_locs = id_array.shape()[0] as usize;

    // Read location IDs
    let loc_ids: Vec<String> = (0..n_locs)
        .map(|i| {
            let chunk: Vec<String> = id_array
                .retrieve_chunk::<Vec<String>>(&[i as u64])
                .unwrap();
            chunk[0].clone()
        })
        .collect();

    let var_names: Vec<String> = VAR_NAMES.iter().map(|s| s.to_string()).collect();

    loc_ids
        .iter()
        .enumerate()
        .map(|(loc_idx, loc_id): (usize, &String)| {
            let columns: Vec<(String, Vec<f64>)> = var_names
                .iter()
                .map(|name| {
                    let array = zarrs::array::Array::open(store.clone(), &format!("/{name}")).unwrap();
                    let f32_vals: Vec<f32> = array
                        .retrieve_chunk::<Vec<f32>>(&[loc_idx as u64, 0])
                        .unwrap();
                    let f64_vals: Vec<f64> = f32_vals.iter().map(|&v| v as f64).collect();
                    (name.clone(), f64_vals)
                })
                .collect();
            (loc_id.clone(), columns)
        })
        .collect()
}

#[cfg(feature = "zarr")]
#[test]
fn test_zarr_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let data = generate_data();

    let zarr_path = dir.path().join("test.zarr");
    let loc_ids: Vec<String> = LOCATIONS.iter().map(|s| s.to_string()).collect();
    let var_names_owned: Vec<String> = VAR_NAMES.iter().map(|s| s.to_string()).collect();

    bmi_driver::output::zarr::create_zarr_store(
        &zarr_path,
        START_TIME,
        INTERVAL,
        N_STEPS - 1,
        &loc_ids,
        &var_names_owned,
    )
    .unwrap();

    let mut store = bmi_driver::output::zarr::ZarrStore::new(zarr_path.clone(), 0);
    write_all(&mut store, &data);

    let read_back = read_zarr_data(&zarr_path);
    assert_eq!(read_back.len(), data.len());

    for (orig, read) in data.iter().zip(read_back.iter()) {
        assert_eq!(orig.0, read.0, "location ID mismatch");
        assert_eq!(orig.1.len(), read.1.len(), "variable count mismatch");
        for (orig_col, read_col) in orig.1.iter().zip(read.1.iter()) {
            assert_eq!(orig_col.0, read_col.0, "variable name mismatch");
            assert_eq!(orig_col.1.len(), read_col.1.len(), "value count mismatch for {}", orig_col.0);
            for (i, (o, r)) in orig_col.1.iter().zip(read_col.1.iter()).enumerate() {
                // Zarr stores f32
                assert!(
                    (o - r).abs() < 1e-2,
                    "Zarr value mismatch at step {} for {}: {} vs {}",
                    i, orig_col.0, o, r
                );
            }
        }
    }
}

// --- Parquet ---

/// Read a Parquet file, or a dataset directory of part files, in `LOCATIONS` order.
#[cfg(feature = "parquet")]
fn read_parquet_data(path: &Path) -> Vec<(String, Vec<(String, Vec<f64>)>)> {
    read_parquet_locations(path, LOCATIONS)
}

#[cfg(feature = "parquet")]
fn read_parquet_locations(path: &Path, locations: &[&str]) -> Vec<(String, Vec<(String, Vec<f64>)>)> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use std::collections::HashMap;
    use std::fs::File;

    let files: Vec<std::path::PathBuf> = if path.is_dir() {
        let mut parts: Vec<_> = std::fs::read_dir(path)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "parquet"))
            .collect();
        parts.sort();
        parts
    } else {
        vec![path.to_path_buf()]
    };

    let mut var_names: Option<Vec<String>> = None;
    // id -> (step -> [value per var])
    let mut rows: HashMap<String, Vec<(i64, Vec<f64>)>> = HashMap::new();
    for file in files {
        let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(file).unwrap()).unwrap();
        let names: Vec<String> = builder
            .schema()
            .fields()
            .iter()
            .skip(3) // id, time_step, time
            .map(|f| f.name().clone())
            .collect();
        // Every part of a dataset must share one schema
        assert_eq!(var_names.get_or_insert_with(|| names.clone()), &names, "part schemas differ");
        read_parquet_rows(builder.build().unwrap(), names.len(), &mut rows);
    }
    let var_names = var_names.unwrap();

    locations
        .iter()
        .map(|loc_id| {
            let mut loc_rows = rows.remove(*loc_id).unwrap();
            loc_rows.sort_by_key(|(step, _)| *step);
            let columns: Vec<(String, Vec<f64>)> = var_names
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let vals: Vec<f64> = loc_rows.iter().map(|(_, row)| row[i]).collect();
                    (name.clone(), vals)
                })
                .collect();
            (loc_id.to_string(), columns)
        })
        .collect()
}

#[cfg(feature = "parquet")]
fn read_parquet_rows(
    reader: parquet::arrow::arrow_reader::ParquetRecordBatchReader,
    n_vars: usize,
    rows: &mut std::collections::HashMap<String, Vec<(i64, Vec<f64>)>>,
) {
    use arrow_array::{Array, Float32Array, Int64Array, StringArray};
    for batch in reader {
        let batch = batch.unwrap();
        let ids = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
        let steps = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
        let var_cols: Vec<&Float32Array> = (0..n_vars)
            .map(|i| batch.column(3 + i).as_any().downcast_ref::<Float32Array>().unwrap())
            .collect();
        for row in 0..batch.num_rows() {
            let vals: Vec<f64> = var_cols.iter().map(|c| c.value(row) as f64).collect();
            rows.entry(ids.value(row).to_string())
                .or_default()
                .push((steps.value(row), vals));
        }
    }
}

#[cfg(feature = "parquet")]
#[test]
fn test_parquet_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let data = generate_data();

    let start_epoch = 1262304000i64;
    let parquet_path = dir.path().join("test.parquet");
    let mut store = bmi_driver::output::parquet::ParquetWriter::new(
        parquet_path.clone(),
        start_epoch,
        INTERVAL,
        VAR_NAMES.iter().map(|s| s.to_string()).collect(),
    )
    .unwrap();
    write_all(&mut store, &data);

    let read_back = read_parquet_data(&parquet_path);
    assert_eq!(read_back.len(), data.len());

    for (orig, read) in data.iter().zip(read_back.iter()) {
        assert_eq!(orig.0, read.0, "location ID mismatch");
        assert_eq!(orig.1.len(), read.1.len(), "variable count mismatch");
        for (orig_col, read_col) in orig.1.iter().zip(read.1.iter()) {
            assert_eq!(orig_col.0, read_col.0, "variable name mismatch");
            assert_eq!(orig_col.1.len(), read_col.1.len(), "value count mismatch for {}", orig_col.0);
            for (i, (o, r)) in orig_col.1.iter().zip(read_col.1.iter()).enumerate() {
                // Parquet stores f32, so tolerance must account for f64->f32->f64 roundtrip
                assert!(
                    (o - r).abs() < 1e-2,
                    "Parquet value mismatch at step {} for {}: {} vs {}",
                    i, orig_col.0, o, r
                );
            }
        }
    }
}

/// Variables shorter than the first column, or missing, must be NaN-filled
/// rather than panic, and a location whose columns come in a different order
/// must still land in the schema given up front.
#[cfg(feature = "parquet")]
#[test]
fn test_parquet_short_and_missing_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.parquet");
    let var_names: Vec<String> = VAR_NAMES.iter().map(|s| s.to_string()).collect();

    let mut store =
        bmi_driver::output::parquet::ParquetWriter::new(path.clone(), 1262304000, INTERVAL, var_names.clone())
            .unwrap();
    // cat-100: EVAPOTRANS is shorter than Q_OUT (the first column); RAIN_RATE is missing.
    // cat-200: columns in a different order, with RAIN_RATE longest.
    let data = vec![
        (
            "cat-100".to_string(),
            vec![
                ("Q_OUT".to_string(), vec![1.0, 2.0, 3.0]),
                ("EVAPOTRANS".to_string(), vec![10.0]),
            ],
        ),
        (
            "cat-200".to_string(),
            vec![
                ("RAIN_RATE".to_string(), vec![7.0, 8.0, 9.0, 10.0]),
                ("Q_OUT".to_string(), vec![4.0, 5.0]),
                ("EVAPOTRANS".to_string(), vec![6.0, 6.5]),
            ],
        ),
    ];
    write_all(&mut store, &data);

    let read = read_parquet_data(&path);
    let nan = f64::NAN;
    let expected: [[&[f64]; 3]; 2] = [
        [&[1.0, 2.0, 3.0], &[10.0, nan, nan], &[nan, nan, nan]],
        [&[4.0, 5.0, nan, nan], &[6.0, 6.5, nan, nan], &[7.0, 8.0, 9.0, 10.0]],
    ];
    for (loc, (_, columns)) in read.iter().enumerate() {
        for (v, (name, vals)) in columns.iter().enumerate() {
            assert_eq!(name, &var_names[v]);
            let want = expected[loc][v];
            assert_eq!(vals.len(), want.len(), "loc {loc} {name}");
            for (got, want) in vals.iter().zip(want) {
                assert!(
                    (got.is_nan() && want.is_nan()) || got == want,
                    "loc {loc} {name}: {vals:?} vs {want:?}"
                );
            }
        }
    }
}

/// Workers write parts of one dataset directory, with no merge step.
#[cfg(feature = "parquet")]
#[test]
fn test_parquet_dataset_parts() {
    let dir = tempfile::tempdir().unwrap();
    let dataset = dir.path().join("results.parquet");
    std::fs::create_dir_all(&dataset).unwrap();
    let data = generate_data();
    let var_names: Vec<String> = VAR_NAMES.iter().map(|s| s.to_string()).collect();

    let handles: Vec<_> = data
        .iter()
        .enumerate()
        .map(|(worker, loc)| {
            let path = dataset.join(format!("part-{worker:05}.parquet"));
            // Reverse the column order so the schema can't come from the first result.
            let mut columns = loc.1.clone();
            columns.reverse();
            let work = vec![(loc.0.clone(), columns)];
            let var_names = var_names.clone();
            std::thread::spawn(move || {
                let mut store =
                    bmi_driver::output::parquet::ParquetWriter::new(path, 1262304000, INTERVAL, var_names)
                        .unwrap();
                write_all(&mut store, &work);
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    let read = read_parquet_data(&dataset);
    assert_eq!(read.len(), data.len());
    for (orig, read) in data.iter().zip(&read) {
        assert_eq!(orig.0, read.0);
        for (o, r) in orig.1.iter().zip(&read.1) {
            assert_eq!(o.0, r.0);
            assert_eq!(o.1.len(), r.1.len());
            for (a, b) in o.1.iter().zip(&r.1) {
                assert!((a - b).abs() < 1e-2);
            }
        }
    }
}

// --- Cross-format equivalence ---

#[cfg(all(feature = "zarr", feature = "parquet"))]
#[test]
fn test_all_formats_equivalent() {
    let dir = tempfile::tempdir().unwrap();
    let data = generate_data();
    let start_epoch = 1262304000i64;

    // Write CSV
    let csv_dir = dir.path().join("csv");
    std::fs::create_dir_all(&csv_dir).unwrap();
    let mut csv_store = bmi_driver::output::csv::CsvStore::new(csv_dir.clone(), start_epoch, INTERVAL);
    write_all(&mut csv_store, &data);

    // Write NetCDF
    let nc_path = dir.path().join("test.nc");
    let mut nc_store = bmi_driver::output::netcdf::NetCdfWriter::new(
        nc_path.clone(),
        START_TIME,
        INTERVAL,
        N_STEPS - 1,
    )
    .unwrap();
    write_all(&mut nc_store, &data);

    // Write Zarr
    let zarr_path = dir.path().join("test.zarr");
    let loc_ids: Vec<String> = LOCATIONS.iter().map(|s| s.to_string()).collect();
    let var_names_owned: Vec<String> = VAR_NAMES.iter().map(|s| s.to_string()).collect();
    bmi_driver::output::zarr::create_zarr_store(
        &zarr_path,
        START_TIME,
        INTERVAL,
        N_STEPS - 1,
        &loc_ids,
        &var_names_owned,
    )
    .unwrap();
    let mut zarr_store = bmi_driver::output::zarr::ZarrStore::new(zarr_path.clone(), 0);
    write_all(&mut zarr_store, &data);

    // Write Parquet
    let parquet_path = dir.path().join("test.parquet");
    let mut parquet_store = bmi_driver::output::parquet::ParquetWriter::new(
        parquet_path.clone(),
        start_epoch,
        INTERVAL,
        VAR_NAMES.iter().map(|s| s.to_string()).collect(),
    )
    .unwrap();
    write_all(&mut parquet_store, &data);

    // Read back
    let csv_data = read_csv_data(&csv_dir);
    let nc_data = read_netcdf_data(&nc_path);
    let zarr_data = read_zarr_data(&zarr_path);
    let parquet_data = read_parquet_data(&parquet_path);

    // Compare location IDs
    assert_eq!(csv_data.len(), nc_data.len());
    assert_eq!(csv_data.len(), zarr_data.len());
    assert_eq!(csv_data.len(), parquet_data.len());

    for loc_idx in 0..csv_data.len() {
        assert_eq!(csv_data[loc_idx].0, nc_data[loc_idx].0, "CSV vs NetCDF loc ID");
        assert_eq!(csv_data[loc_idx].0, zarr_data[loc_idx].0, "CSV vs Zarr loc ID");
        assert_eq!(csv_data[loc_idx].0, parquet_data[loc_idx].0, "CSV vs Parquet loc ID");

        let csv_cols = &csv_data[loc_idx].1;
        let nc_cols = &nc_data[loc_idx].1;
        let zarr_cols = &zarr_data[loc_idx].1;
        let parquet_cols = &parquet_data[loc_idx].1;

        assert_eq!(csv_cols.len(), nc_cols.len(), "CSV vs NetCDF var count");
        assert_eq!(csv_cols.len(), zarr_cols.len(), "CSV vs Zarr var count");
        assert_eq!(csv_cols.len(), parquet_cols.len(), "CSV vs Parquet var count");

        for var_idx in 0..csv_cols.len() {
            let csv_vals = &csv_cols[var_idx].1;
            let nc_vals = &nc_cols[var_idx].1;
            let zarr_vals = &zarr_cols[var_idx].1;
            let parquet_vals = &parquet_cols[var_idx].1;

            assert_eq!(csv_vals.len(), nc_vals.len(), "value count mismatch");
            assert_eq!(csv_vals.len(), zarr_vals.len(), "value count mismatch");
            assert_eq!(csv_vals.len(), parquet_vals.len(), "value count mismatch");

            for step in 0..csv_vals.len() {
                // NetCDF, Zarr, and Parquet store f32; CSV stores full precision
                // Compare NetCDF ↔ Zarr (both f32, should be identical)
                assert!(
                    (nc_vals[step] - zarr_vals[step]).abs() < 1e-6,
                    "NetCDF vs Zarr mismatch at loc={}, var={}, step={}: {} vs {}",
                    loc_idx, csv_cols[var_idx].0, step, nc_vals[step], zarr_vals[step]
                );
                // Compare CSV ↔ NetCDF (CSV has more precision than f32 roundtrip)
                assert!(
                    (csv_vals[step] - nc_vals[step]).abs() < 1e-2,
                    "CSV vs NetCDF mismatch at loc={}, var={}, step={}: {} vs {}",
                    loc_idx, csv_cols[var_idx].0, step, csv_vals[step], nc_vals[step]
                );
                // Compare NetCDF ↔ Parquet (both f32, should be identical)
                assert!(
                    (nc_vals[step] - parquet_vals[step]).abs() < 1e-6,
                    "NetCDF vs Parquet mismatch at loc={}, var={}, step={}: {} vs {}",
                    loc_idx, csv_cols[var_idx].0, step, nc_vals[step], parquet_vals[step]
                );
            }
        }
    }
}
