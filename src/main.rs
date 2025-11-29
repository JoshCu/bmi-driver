//! Example usage of the BMI Rust adapter.
//!
//! This demonstrates how to load, initialize, run, and interact with
//! BMI-compliant models using either C or Fortran interfaces.

use bmi::{
    preload_dependencies, Bmi, BmiC, BmiError, BmiExt, BmiFortran, Forcings, ForcingsExt,
    ModelRunner, NetCdfForcings,
};
use std::path::PathBuf;
use std::process::{Child, Command};

fn main() -> Result<(), BmiError> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() >= 2 {
        return run_single_location(&args[1]);
    }

    preload_dependencies()?;

    let db_path = PathBuf::from(
        "/home/josh/code/JoshCu/hf_resample/output/speed_test/config/speed_test_subset.gpkg",
    );
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let mut stmt = conn.prepare("SELECT divide_id FROM 'divides'").unwrap();
    let rows = stmt
        .query_map([], |row| Ok(row.get::<_, String>(0)?))
        .unwrap();
    let locations: Vec<String> = rows.flatten().collect();

    let max_parallel = 8;
    let mut handles: Vec<Child> = Vec::new();

    for location in locations {
        // Wait if we have too many running
        while handles.len() >= max_parallel {
            // Find and remove one finished process
            let mut finished_idx = None;
            for (i, h) in handles.iter_mut().enumerate() {
                if h.try_wait().ok().flatten().is_some() {
                    finished_idx = Some(i);
                    break;
                }
            }

            if let Some(idx) = finished_idx {
                handles.remove(idx);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }

        let child = Command::new(&args[0])
            .arg(&location)
            .spawn()
            .expect("Failed to spawn");
        handles.push(child);
    }

    // Wait for remaining
    for mut h in handles {
        h.wait().ok();
    }

    Ok(())
}

fn run_single_location(location: &str) -> Result<(), BmiError> {
    preload_dependencies()?;

    let mut runner = ModelRunner::from_config(
        "/home/josh/code/JoshCu/hf_resample/output/speed_test/config/realization.json",
    )?;

    runner.initialize(location)?;

    let mut outputs: Vec<f64> = Vec::new();
    while runner.has_more_steps() {
        runner.update()?;
        outputs.push(runner.get_main_output()?);
    }

    println!("Completed {} timesteps for {}", outputs.len(), location);
    runner.finalize()?;
    Ok(())
}

fn print_model_info(model: &dyn Bmi) -> Result<(), BmiError> {
    println!("=== Model Information ===");
    println!("Component name: {}", model.get_component_name()?);
    println!();

    println!("=== Time Information ===");
    println!("Time units: {}", model.get_time_units()?);
    println!(
        "Time conversion factor: {} (to seconds)",
        model.get_time_convert_factor()
    );
    println!("Start time: {}", model.get_start_time()?);
    println!("End time: {}", model.get_end_time()?);
    println!("Time step: {}", model.get_time_step()?);
    println!("Current time: {}", model.get_current_time()?);
    println!();

    println!(
        "=== Input Variables ({}) ===",
        model.get_input_item_count()?
    );
    for name in model.get_input_var_names()? {
        print_var_info(model, &name)?;
    }
    println!();

    println!(
        "=== Output Variables ({}) ===",
        model.get_output_item_count()?
    );
    for name in model.get_output_var_names()? {
        print_var_info(model, &name)?;
    }
    println!();

    Ok(())
}

fn print_var_info(model: &dyn Bmi, name: &str) -> Result<(), BmiError> {
    let var_type = model.get_var_type(name)?;
    let units = model.get_var_units(name)?;
    let itemsize = model.get_var_itemsize(name)?;
    let nbytes = model.get_var_nbytes(name)?;
    let grid = model.get_var_grid(name)?;
    let location = model.get_var_location(name)?;

    println!(
        "  {} [{}]",
        name,
        if units.is_empty() { "-" } else { &units }
    );
    println!(
        "    type: {}, itemsize: {}, nbytes: {}, grid: {}, location: {}",
        var_type, itemsize, nbytes, grid, location
    );
    Ok(())
}

fn run_model(model: &mut dyn Bmi) -> Result<(), BmiError> {
    let end_time = model.get_end_time()?;
    let time_step = model.get_time_step()?;
    let location = "cat-2863621";
    let mut step = 0;
    let mut forcings = NetCdfForcings::new("noaa_forcings");
    forcings
        .initialize("/home/josh/code/JoshCu/hf_resample/output/time_test/forcings/forcings.nc")?;

    // Print forcing info
    println!(
        "=== Forcing Variables ({}) ===",
        forcings.get_output_item_count()?
    );
    for name in forcings.get_output_var_names()? {
        println!("  {} [{}]", name, forcings.get_var_units(&name)?);
    }
    println!();
    println!("Locations: {:?}", forcings.get_location_ids()?);
    println!("Timesteps: {}", forcings.get_timestep_count()?);
    println!(
        "Time: {} to {} (step: {})",
        forcings.get_start_time()?,
        forcings.get_end_time()?,
        forcings.get_time_step()?
    );

    println!("=== Running Model ===");
    println!(
        "Running from {} to {} (step size: {})",
        model.get_current_time()?,
        end_time,
        time_step
    );
    println!();

    // Get first output variable name to print values
    let output_vars = model.get_output_var_names()?;
    let first_output = output_vars.first().cloned();

    while model.get_current_time()? < end_time {
        // Get forcing values at current timestep (auto-typed)
        // let precip = forcings.get_value_at_index("precip_rate", location, step)?;
        let precip = 0.0002;

        let temp = forcings.get_value_at_index("TMP_2maboveground", location, step)?;
        let spfh = forcings.get_value_at_index("SPFH_2maboveground", location, step)?;
        let pres = forcings.get_value_at_index("PRES_surface", location, step)?;
        let dlwrf = forcings.get_value_at_index("DLWRF_surface", location, step)?;
        let dswrf = forcings.get_value_at_index("DSWRF_surface", location, step)?;
        let ugrd = forcings.get_value_at_index("UGRD_10maboveground", location, step)?;
        let vgrd = forcings.get_value_at_index("VGRD_10maboveground", location, step)?;
        // print out the values for debugging
        // println!(
        //     "precip: {}, temp: {}, spfh: {}, pres: {}, dlwrf: {}, dswrf: {}, ugrd: {}, vgrd: {}",
        //     precip, temp, spfh, pres, dlwrf, dswrf, ugrd, vgrd
        // );

        // Set model inputs (auto-converts f64 to model's type)
        model.set_value("PRCPNONC", &[precip])?;
        model.set_value("SFCTMP", &[temp])?;
        model.set_value("Q2", &[spfh])?;
        model.set_value("SFCPRS", &[pres])?;
        model.set_value("LWDN", &[dlwrf])?;
        model.set_value("SOLDN", &[dswrf])?;
        model.set_value("UU", &[ugrd])?;
        model.set_value("VV", &[vgrd])?;

        model.update()?;
        step += 1;

        let current_time = model.get_current_time()?;

        // Print progress every 10 steps or at the end
        if step % 1 == 0 || current_time >= end_time {
            println!("Step {}: time = {:.2}", step, current_time);

            // Try to print the first output variable's value
            if let Some(ref var_name) = first_output {
                // Try f64 first
                if let Ok(values) = model.get_value_scalar(var_name) {
                    println!("QINSUR = {:9}", values);
                }
            }
        }
    }

    println!();
    println!("✓ Completed {} steps", step);

    Ok(())
}
