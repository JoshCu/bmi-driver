//! The embedding API: running a model chain from another Rust program and getting results back
//! in memory, with no chdir and no output files.

mod common;

use std::path::PathBuf;

use bmi_driver::{run_location, Simulation};
use common::*;

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

#[test]
fn runs_a_location_and_returns_values() {
    let Some(dir) = data_dir() else { return };

    let mut sim = Simulation::open(dir).unwrap();
    let out = sim.run("cat-1").unwrap();

    assert_eq!(out.location, "cat-1");
    assert_eq!(out.start_epoch, START_EPOCH);
    assert_eq!(out.interval, 3600);
    // 24 hourly intervals, inclusive of both ends
    assert_eq!(out.len(), 25);

    let q = out.get("Q_OUT").expect("Q_OUT missing");
    let expected = expected_q_out(0, q.len());
    for (i, (got, want)) in q.iter().zip(&expected).enumerate() {
        assert!(
            approx_eq(*got, *want),
            "step {i}: got {got}, expected {want}"
        );
    }

    // The config asked for three variables, including a non-double one.
    let mut names: Vec<&str> = out.variable_names().collect();
    names.sort();
    assert_eq!(names, ["Q_OUT", "STEP_COUNT", "STORAGE"]);

    let steps = out.get("STEP_COUNT").unwrap();
    assert!(approx_eq(steps[0], 1.0), "int variable came back as {steps:?}");

    let times: Vec<i64> = out.times().collect();
    assert_eq!(times[0], START_EPOCH);
    assert_eq!(times[1], START_EPOCH + 3600);
    assert_eq!(times.len(), out.len());
}

#[test]
fn run_variable_returns_one_column() {
    let Some(dir) = data_dir() else { return };

    let mut sim = Simulation::open(dir).unwrap();
    let q = sim.run_variable("cat-2", "Q_OUT").unwrap();
    assert_eq!(q, expected_q_out(1, q.len()));
}

#[test]
fn one_simulation_runs_many_locations() {
    let Some(dir) = data_dir() else { return };

    // Reusing a Simulation across locations is the whole reason it exists: the config parse and
    // the forcing file open happen once, the models are reloaded per location.
    let mut sim = Simulation::open(dir).unwrap();
    for (idx, loc) in LOCATIONS.iter().enumerate() {
        let q = sim.run_variable(loc, "Q_OUT").unwrap();
        assert_eq!(q, expected_q_out(idx, q.len()), "location {loc}");
    }
}

#[test]
fn relative_paths_resolve_against_the_data_dir_not_the_cwd() {
    let Some(dir) = data_dir() else { return };

    // The config's library_file and init_config are relative. Nothing in the library may chdir,
    // so running from an unrelated working directory must still work.
    let cwd_before = std::env::current_dir().unwrap();
    let q = run_location(dir, "cat-1", "Q_OUT").unwrap();
    assert_eq!(q, expected_q_out(0, q.len()));
    assert_eq!(std::env::current_dir().unwrap(), cwd_before);
}

#[test]
fn two_simulations_coexist_in_one_process() {
    let Some((dir_a, dir_b)) = shared_data_dirs() else {
        return;
    };

    // With a process-global working directory this was impossible.
    let mut sim_a = Simulation::open(dir_a).unwrap();
    let mut sim_b = Simulation::open(dir_b).unwrap();

    let qa = sim_a.run_variable("cat-1", "Q_OUT").unwrap();
    let qb = sim_b.run_variable("cat-1", "Q_OUT").unwrap();
    assert_eq!(qa, qb);
    assert_ne!(sim_a.data_dir(), sim_b.data_dir());
}

#[test]
fn reports_variable_units() {
    let Some(dir) = data_dir() else { return };

    let mut sim = Simulation::open(dir).unwrap();
    // Consumers converting the output (e.g. a depth per hour into a discharge) need this
    // rather than a hardcoded assumption about the model's units.
    assert_eq!(sim.var_units("cat-1", "Q_OUT").unwrap(), "m h-1");
    assert_eq!(sim.var_units("cat-1", "RAINRATE").unwrap(), "mm h-1");
}

#[test]
fn output_variable_selection_can_be_overridden() {
    let Some(dir) = data_dir() else { return };

    let mut sim = Simulation::open(dir).unwrap();
    sim.set_output_variables(["STORAGE".to_string()]);
    let out = sim.run("cat-1").unwrap();
    assert_eq!(out.variable_names().collect::<Vec<_>>(), ["STORAGE"]);
}

#[test]
fn missing_location_is_an_error_not_a_panic() {
    let Some(dir) = data_dir() else { return };

    let mut sim = Simulation::open(dir).unwrap();
    assert!(sim.run("cat-does-not-exist").is_err());
}

#[test]
fn missing_data_dir_is_an_error() {
    let missing = PathBuf::from("/nonexistent/data/dir");
    assert!(Simulation::open(&missing).is_err());
}

#[test]
fn simulations_run_concurrently_on_one_forcing_file() {
    let Some(dir) = data_dir() else { return };

    // This is the shape rs_route uses: worker threads each building their own Simulation over
    // the same data directory. It only works because the forcing file is opened once for the
    // process -- two HDF5 handles on one file tear each other down on close.
    let handles: Vec<_> = LOCATIONS
        .iter()
        .enumerate()
        .map(|(idx, loc)| {
            let loc = loc.to_string();
            std::thread::spawn(move || {
                let mut sim = Simulation::open(dir).unwrap();
                let q = sim.run_variable(&loc, "Q_OUT").unwrap();
                (idx, q)
            })
        })
        .collect();

    for handle in handles {
        let (idx, q) = handle.join().expect("worker thread panicked");
        assert_eq!(q, expected_q_out(idx, q.len()), "location index {idx}");
    }
}

#[test]
fn forcings_are_read_through_the_shared_handle() {
    use bmi_driver::forcings::Forcings;

    let Some(dir) = data_dir() else { return };

    let mut a = bmi_driver::NetCdfForcings::new("a");
    let mut b = bmi_driver::NetCdfForcings::new("b");
    let path = dir.join("forcings").join("forcings.nc");
    let path = path.to_str().unwrap();

    a.initialize(path).unwrap();
    b.initialize(path).unwrap();

    // Two readers, two independent location caches, one underlying file.
    let series_a = a.series("RAINRATE", "cat-1").unwrap();
    let series_b = b.series("RAINRATE", "cat-3").unwrap();
    assert_eq!(series_a.len(), N_FORCING_STEPS);
    assert_ne!(series_a, series_b);
    assert_eq!(a.var_units("RAINRATE").unwrap(), "mm h-1");

    // Dropping one reader must not disturb the other.
    a.finalize().unwrap();
    assert_eq!(b.series("RAINRATE", "cat-3").unwrap(), series_b);
}

#[test]
fn total_steps_is_known_before_the_first_run() {
    let Some(dir) = data_dir() else { return };

    let mut sim = Simulation::open(dir).unwrap();
    // Callers size their own buffers from this, so it cannot wait for a run to populate it.
    assert_eq!(sim.total_steps(), 24);
    assert_eq!(sim.run("cat-1").unwrap().len(), sim.total_steps() + 1);
}

#[test]
fn absolute_config_paths_are_left_alone() {
    use bmi_driver::runner::resolve_path;
    use std::path::Path;

    // Real ngen realizations point library_file at /dmod/shared_libs/..., which must not be
    // rewritten relative to the data directory.
    let base = Path::new("/data/my_run");
    assert_eq!(
        resolve_path(base, "/dmod/shared_libs/libcfebmi.so.1.0.0"),
        Path::new("/dmod/shared_libs/libcfebmi.so.1.0.0")
    );
    assert_eq!(
        resolve_path(base, "./forcings/forcings.nc"),
        Path::new("/data/my_run/./forcings/forcings.nc")
    );
    assert_eq!(
        resolve_path(base, "libs/x.so"),
        Path::new("/data/my_run/libs/x.so")
    );
    // An unset field stays unset, so callers can still test for "not provided".
    assert_eq!(resolve_path(base, ""), Path::new(""));
}
