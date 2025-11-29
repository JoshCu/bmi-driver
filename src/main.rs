use bmi::{preload_dependencies, Bmi, BmiError, BmiExt, ModelRunner};
use std::path::PathBuf;
use std::process::{Child, Command};

fn main() -> Result<(), BmiError> {
    let args: Vec<String> = std::env::args().collect();

    // Child process: run locations passed as args
    if args.len() >= 2 {
        preload_dependencies();
        for location in &args[1..] {
            if let Err(e) = run_single_location(location) {
                eprintln!("Error processing {}: {:?}", location, e);
            }
        }
        return Ok(());
    }

    // Parent process: read locations from gpkg and spawn workers
    preload_dependencies();

    let db_path = PathBuf::from(
        "/home/josh/code/JoshCu/hf_resample/output/cost_test/config/cost_test_subset.gpkg",
    );
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let mut stmt = conn.prepare("SELECT divide_id FROM 'divides'").unwrap();
    let rows = stmt
        .query_map([], |row| Ok(row.get::<_, String>(0)?))
        .unwrap();
    let locations: Vec<String> = rows.flatten().collect();

    let num_processes = 32;
    let chunk_size = (locations.len() + num_processes - 1) / num_processes;

    let mut handles: Vec<Child> = Vec::new();
    for chunk in locations.chunks(chunk_size) {
        let child = Command::new(&args[0])
            .args(chunk)
            .spawn()
            .expect("Failed to spawn");
        handles.push(child);
    }

    for mut h in handles {
        h.wait().ok();
    }

    Ok(())
}

fn run_single_location(location: &str) -> Result<(), BmiError> {
    let mut runner = ModelRunner::from_config(
        "/home/josh/code/JoshCu/hf_resample/output/cost_test/config/realization.json",
    )?;

    runner.initialize(location)?;
    runner.run()?;

    let _outputs = runner.main_outputs()?;
    runner.finalize()?;
    Ok(())
}

fn print_model_info(model: &dyn Bmi) -> Result<(), BmiError> {
    println!("=== Model Information ===");
    println!("Component name: {}", model.get_component_name()?);
    println!();

    println!("=== Time Information ===");
    println!("Time units: {}", model.get_time_units()?);
    println!("Time factor: {} (to seconds)", model.time_factor());
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
