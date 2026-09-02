//! Builds a self-contained data directory -- forcings, config, model library -- so the
//! integration tests can run a real BMI model without any external model libraries installed.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

pub const LOCATIONS: &[&str] = &["cat-1", "cat-2", "cat-3"];
pub const N_FORCING_STEPS: usize = 49;
/// 2010-01-01 00:00:00 UTC
pub const START_EPOCH: i64 = 1_262_304_000;
pub const DRAIN_FRACTION: f64 = 0.15;
pub const INITIAL_STORAGE: f64 = 0.01;

/// Deterministic precipitation in mm h-1, so tests can predict the model's output exactly.
pub fn precip_mm_per_hour(loc_idx: usize, step: usize) -> f32 {
    (1.0 + loc_idx as f32) * 0.5 + (step % 7) as f32 * 0.25
}

pub fn temperature_kelvin(loc_idx: usize, step: usize) -> f32 {
    278.15 + loc_idx as f32 + (step % 5) as f32
}

/// Compile `bucket_bmi.c` into a shared library. Returns None if no C compiler is available,
/// which lets the tests skip rather than fail on a machine without one.
pub fn build_bucket_library(out_dir: &Path) -> Option<PathBuf> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("bucket_bmi.c");
    let lib = out_dir.join("libbucketbmi.so");

    for cc in ["cc", "gcc", "clang"] {
        let status = Command::new(cc)
            .args(["-shared", "-fPIC", "-O1", "-o"])
            .arg(&lib)
            .arg(&src)
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return Some(lib);
        }
    }
    eprintln!("skipping: no working C compiler found");
    None
}

/// Two data directories, built once and shared by every test in the binary.
///
/// Deliberately not one per test: cargo runs these tests as threads in a single process, and
/// repeatedly creating then deleting NetCDF files makes HDF5's process-wide open-file registry
/// trip over recycled inodes. Sharing the fixtures also matches how the driver is really used --
/// one data directory, many locations.
pub fn shared_data_dirs() -> Option<(&'static Path, &'static Path)> {
    struct Fixture {
        _dirs: (tempfile::TempDir, tempfile::TempDir),
        paths: Option<(PathBuf, PathBuf)>,
    }
    static FIXTURE: OnceLock<Fixture> = OnceLock::new();

    let fixture = FIXTURE.get_or_init(|| {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let paths = match (make_data_dir(a.path()), make_data_dir(b.path())) {
            (Some(_), Some(_)) => Some((a.path().to_path_buf(), b.path().to_path_buf())),
            _ => None,
        };
        Fixture {
            _dirs: (a, b),
            paths,
        }
    });

    fixture
        .paths
        .as_ref()
        .map(|(a, b)| (a.as_path(), b.as_path()))
}

/// The primary shared data directory. `None` means the fixtures could not be built (no C
/// compiler), and the test should skip.
pub fn data_dir() -> Option<&'static Path> {
    shared_data_dirs().map(|(a, _)| a)
}

/// Write a complete data directory. Returns the path to the library it built, or None to skip.
pub fn make_data_dir(root: &Path) -> Option<PathBuf> {
    std::fs::create_dir_all(root.join("config").join("cat_config").join("Bucket")).unwrap();
    std::fs::create_dir_all(root.join("forcings")).unwrap();
    std::fs::create_dir_all(root.join("libs")).unwrap();

    let lib = build_bucket_library(&root.join("libs"))?;
    write_forcings(&root.join("forcings").join("forcings.nc"));

    for loc in LOCATIONS {
        std::fs::write(
            root.join("config")
                .join("cat_config")
                .join("Bucket")
                .join(format!("{loc}.ini")),
            format!("drain_fraction={DRAIN_FRACTION}\ninitial_storage={INITIAL_STORAGE}\n"),
        )
        .unwrap();
    }

    std::fs::write(
        root.join("config").join("realization.json"),
        realization_json(),
    )
    .unwrap();

    Some(lib)
}

fn write_forcings(path: &Path) {
    // Same reason bmi_driver::nc_lock exists: libnetcdf is not thread-safe and cargo runs
    // integration tests in parallel threads.
    bmi_driver::nc_lock::with_netcdf(|| write_forcings_locked(path));
}

fn write_forcings_locked(path: &Path) {
    let mut file = netcdf::create(path).unwrap();
    // `file` is dropped at the end of this function, still under the lock -- nc_close is as
    // global as nc_open.
    file.add_dimension("catchment-id", LOCATIONS.len()).unwrap();
    file.add_dimension("time", N_FORCING_STEPS).unwrap();

    file.add_string_variable("ids", &["catchment-id"]).unwrap();
    for (i, loc) in LOCATIONS.iter().enumerate() {
        file.variable_mut("ids").unwrap().put_string(loc, i).unwrap();
    }

    file.add_variable::<i64>("Time", &["catchment-id", "time"])
        .unwrap();
    let times: Vec<i64> = (0..N_FORCING_STEPS)
        .map(|s| START_EPOCH + s as i64 * 3600)
        .collect();
    for i in 0..LOCATIONS.len() {
        file.variable_mut("Time")
            .unwrap()
            .put_values(&times, (i, ..))
            .unwrap();
    }

    type Generator = fn(usize, usize) -> f32;
    let vars: [(&str, &str, Generator); 2] = [
        ("RAINRATE", "mm h-1", precip_mm_per_hour),
        ("T2D", "K", temperature_kelvin),
    ];
    for (name, units, f) in vars {
        file.add_variable::<f32>(name, &["catchment-id", "time"])
            .unwrap();
        let mut var = file.variable_mut(name).unwrap();
        var.put_attribute("units", units).unwrap();
        for loc_idx in 0..LOCATIONS.len() {
            let row: Vec<f32> = (0..N_FORCING_STEPS).map(|s| f(loc_idx, s)).collect();
            var.put_values(&row, (loc_idx, ..)).unwrap();
        }
    }
}

fn realization_json() -> String {
    // library_file and init_config are deliberately relative: the point of the base_dir
    // refactor is that they resolve against the data dir, not the process working directory.
    r#"{
  "global": {
    "formulations": [{
      "name": "bmi_multi",
      "params": {
        "main_output_variable": "Q_OUT",
        "output_variables": ["Q_OUT", "STORAGE", "STEP_COUNT"],
        "modules": [{
          "name": "bmi_c",
          "params": {
            "model_type_name": "Bucket",
            "library_file": "libs/libbucketbmi.so",
            "init_config": "./config/cat_config/Bucket/{{id}}.ini",
            "registration_function": "register_bmi_bucket",
            "main_output_variable": "Q_OUT",
            "variables_names_map": {
              "precip_rate": "RAINRATE",
              "temperature": "T2D"
            }
          }
        }]
      }
    }],
    "forcing": { "path": "./forcings/forcings.nc" }
  },
  "time": {
    "start_time": "2010-01-01 00:00:00",
    "end_time": "2010-01-02 00:00:00",
    "output_interval": 3600
  }
}
"#
    .to_string()
}

/// What the bucket model should produce for a location, computed independently of the driver.
pub fn expected_q_out(loc_idx: usize, steps: usize) -> Vec<f64> {
    let mut storage = INITIAL_STORAGE;
    let mut out = Vec::with_capacity(steps);
    for step in 0..steps {
        // mm h-1 -> m h-1, the conversion the driver is expected to apply
        let precip = precip_mm_per_hour(loc_idx, step) as f64 * 0.001;
        storage += precip;
        let q = storage * DRAIN_FRACTION;
        storage -= q;
        out.push(q);
    }
    out
}
