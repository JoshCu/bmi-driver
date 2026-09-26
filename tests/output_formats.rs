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
    let loc_ids: Vec<String> = id_array
        .retrieve_array_subset::<Vec<String>>(&id_array.subset_all())
        .unwrap();

    let group = zarrs::group::Group::open(store.clone(), "/").unwrap();
    let var_names: Vec<String> =
        serde_json::from_value(group.attributes()["variables"].clone()).unwrap();

    let arrays: Vec<_> = var_names
        .iter()
        .map(|name| {
            let array = zarrs::array::Array::open(store.clone(), &format!("/{name}")).unwrap();
            let n_times = array.shape()[1] as usize;
            let vals: Vec<f32> = array.retrieve_array_subset::<Vec<f32>>(&array.subset_all()).unwrap();
            (name.clone(), n_times, vals)
        })
        .collect();

    loc_ids
        .iter()
        .enumerate()
        .map(|(loc_idx, loc_id)| {
            let columns: Vec<(String, Vec<f64>)> = arrays
                .iter()
                .map(|(name, n_times, vals)| {
                    let row = &vals[loc_idx * n_times..(loc_idx + 1) * n_times];
                    (name.clone(), row.iter().map(|&v| v as f64).collect())
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
        bmi_driver::output::zarr::chunk_locations(N_STEPS - 1, LOCATIONS.len()),
    )
    .unwrap();

    let mut store = bmi_driver::output::zarr::ZarrStore::new(zarr_path.clone(), 0).unwrap();
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

/// Several workers writing concurrently into one store, with a location count
/// that is a multiple of neither the chunk height nor the worker count.
#[cfg(feature = "zarr")]
#[test]
fn test_zarr_multi_worker_partial_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let zarr_path = dir.path().join("test.zarr");

    let n_locations: usize = 23;
    let n_workers = 4;
    let chunk_locations = 5;
    let var_names: Vec<String> = VAR_NAMES.iter().map(|s| s.to_string()).collect();
    let loc_ids: Vec<String> = (0..n_locations).map(|i| format!("cat-{i}")).collect();

    // Same alignment as run_parent: round each worker's block up to whole chunks.
    let per_worker = n_locations.div_ceil(n_workers);
    let block = per_worker.div_ceil(chunk_locations) * chunk_locations;
    assert_eq!(block, 10);

    bmi_driver::output::zarr::create_zarr_store(
        &zarr_path,
        START_TIME,
        INTERVAL,
        N_STEPS - 1,
        &loc_ids,
        &var_names,
        chunk_locations,
    )
    .unwrap();

    let value = |loc: usize, var: usize, step: usize| (loc * 1000 + var * 100 + step) as f64;
    // Location 7 has a short Q_OUT series; location 21 has no EVAPOTRANS at all.
    let columns_for = |loc: usize| -> Vec<(String, Vec<f64>)> {
        var_names
            .iter()
            .enumerate()
            .filter(|(v, _)| !(loc == 21 && *v == 1))
            .map(|(v, name)| {
                let len = if loc == 7 && v == 0 { N_STEPS - 5 } else { N_STEPS };
                (name.clone(), (0..len).map(|t| value(loc, v, t)).collect())
            })
            .collect()
    };

    let handles: Vec<_> = (0..n_locations)
        .step_by(block)
        .map(|start| {
            let end = (start + block).min(n_locations);
            let path = zarr_path.clone();
            let work: Vec<_> = (start..end).map(|l| (loc_ids[l].clone(), columns_for(l))).collect();
            std::thread::spawn(move || {
                let mut store = bmi_driver::output::zarr::ZarrStore::new(path, start).unwrap();
                write_all(&mut store, &work);
            })
        })
        .collect();
    assert_eq!(handles.len(), 3);
    for h in handles {
        h.join().unwrap();
    }

    let read_back = read_zarr_data(&zarr_path);
    assert_eq!(read_back.len(), n_locations);
    for (loc, (id, columns)) in read_back.iter().enumerate() {
        assert_eq!(id, &loc_ids[loc]);
        for (v, (name, vals)) in columns.iter().enumerate() {
            assert_eq!(name, &var_names[v]);
            assert_eq!(vals.len(), N_STEPS);
            for (t, &got) in vals.iter().enumerate() {
                let missing = (loc == 21 && v == 1) || (loc == 7 && v == 0 && t >= N_STEPS - 5);
                if missing {
                    assert!(got.is_nan(), "loc {loc} var {name} step {t}: expected NaN, got {got}");
                } else {
                    assert_eq!(got, value(loc, v, t), "loc {loc} var {name} step {t}");
                }
            }
        }
    }

    // One file per chunk of locations, not one per location.
    for name in &var_names {
        let chunk_dir = zarr_path.join(name).join("c");
        let n_files = walk_files(&chunk_dir);
        assert_eq!(n_files, n_locations.div_ceil(chunk_locations), "chunk files for {name}");
    }
}

#[cfg(feature = "zarr")]
fn walk_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let path = e.unwrap().path();
            if path.is_dir() { walk_files(&path) } else { 1 }
        })
        .sum()
}

// --- Parquet ---

#[cfg(feature = "parquet")]
fn read_parquet_data(path: &Path) -> Vec<(String, Vec<(String, Vec<f64>)>)> {
    use arrow_array::{Array, Float32Array, Int64Array, StringArray};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use std::collections::HashMap;
    use std::fs::File;

    let file = File::open(path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let var_names: Vec<String> = builder
        .schema()
        .fields()
        .iter()
        .skip(3) // id, time_step, time
        .map(|f| f.name().clone())
        .collect();
    let reader = builder.build().unwrap();

    // id -> (step -> [value per var])
    let mut rows: HashMap<String, Vec<(i64, Vec<f64>)>> = HashMap::new();
    for batch in reader {
        let batch = batch.unwrap();
        let ids = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
        let steps = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
        let var_cols: Vec<&Float32Array> = (0..var_names.len())
            .map(|i| batch.column(3 + i).as_any().downcast_ref::<Float32Array>().unwrap())
            .collect();
        for row in 0..batch.num_rows() {
            let vals: Vec<f64> = var_cols.iter().map(|c| c.value(row) as f64).collect();
            rows.entry(ids.value(row).to_string())
                .or_default()
                .push((steps.value(row), vals));
        }
    }

    LOCATIONS
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
        1,
    )
    .unwrap();
    let mut zarr_store = bmi_driver::output::zarr::ZarrStore::new(zarr_path.clone(), 0).unwrap();
    write_all(&mut zarr_store, &data);

    // Write Parquet
    let parquet_path = dir.path().join("test.parquet");
    let mut parquet_store = bmi_driver::output::parquet::ParquetWriter::new(
        parquet_path.clone(),
        start_epoch,
        INTERVAL,
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
