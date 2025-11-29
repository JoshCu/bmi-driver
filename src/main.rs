//! Example usage of the BMI Rust adapter.
//!
//! This demonstrates how to load, initialize, run, and interact with
//! a BMI-compliant model.

use bmi::{preload_dependencies, BmiError, BmiModel};
use std::env;
use std::path::PathBuf;

fn main() -> Result<(), BmiError> {
    // Get command line arguments
    let args: Vec<String> = env::args().collect();

    if args.len() < 4 {
        eprintln!(
            "Usage: {} <library_path> <registration_func> <config_file>",
            args[0]
        );
        eprintln!();
        eprintln!("Example:");
        eprintln!(
            "  {} ./libheat.so register_bmi_heat ./heat_config.yaml",
            args[0]
        );
        std::process::exit(1);
    }

    let library_path = PathBuf::from(&args[1]);
    let registration_func = &args[2];
    let config_file = PathBuf::from(&args[3]);

    println!("Loading BMI model...");
    println!("  Library: {}", library_path.display());
    println!("  Registration function: {}", registration_func);
    println!("  Config file: {}", config_file.display());
    println!();

    // Preload common dependencies (libm, etc.) to avoid symbol lookup errors
    preload_dependencies()?;

    // Load the model from the shared library
    let mut model = BmiModel::load("example_model", &library_path, registration_func)?;
    println!("✓ Model loaded successfully");

    // Initialize the model
    model.initialize(&config_file)?;
    println!("✓ Model initialized");
    println!();

    // Print model information
    print_model_info(&model)?;

    // Run the model
    run_model(&mut model)?;

    // Finalize
    model.finalize()?;
    println!("✓ Model finalized");

    Ok(())
}

fn print_model_info(model: &BmiModel) -> Result<(), BmiError> {
    println!("=== Model Information ===");
    println!("Component name: {}", model.get_component_name()?);
    println!();

    println!("=== Time Information ===");
    println!("Time units: {}", model.get_time_units()?);
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

fn print_var_info(model: &BmiModel, name: &str) -> Result<(), BmiError> {
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

fn run_model(model: &mut BmiModel) -> Result<(), BmiError> {
    // let end_time = model.get_end_time()?;
    let end_time = 18000.0;
    let time_step = model.get_time_step()?;
    let mut step = 0;

    println!("=== Running Model ===");
    println!(
        "Running from {} to {} (step size: {})",
        model.get_current_time()?,
        end_time,
        time_step
    );
    println!();

    let atmosphere_water__liquid_equivalent_precipitation_rate = (0.0002 * 1000.0) / 3600.0;
    let EVAPOTRANS = vec![
        0.000000039,
        0.000000034,
        0.000000030,
        0.000000027,
        0.000000024,
        0.000000022,
    ];
    // Get output variable names to print values
    let output_vars = model.get_output_var_names()?;
    let first_output = Some("Q_OUT"); //= output_vars.first().map(|s| s.as_str());

    // model.update_for_duration_seconds(3600.0)?;

    // Get all the inputs and print them and their values
    let input_vars = model.get_input_var_names()?;
    for var_name in input_vars {
        if let Ok(values) = model.get_value::<f64>(var_name.as_str()) {
            println!("{} = {:.10}", var_name, values[0]);
        } else if let Ok(values) = model.get_value::<f32>(var_name.as_str()) {
            println!("{} = {:.10}", var_name, values[0]);
        }
    }

    while model.get_current_time()? < end_time {
        // set the model inputs
        let evap = EVAPOTRANS[step];
        model.set_value(
            "atmosphere_water__liquid_equivalent_precipitation_rate",
            &[atmosphere_water__liquid_equivalent_precipitation_rate],
        )?;
        model.set_value("water_potential_evaporation_flux", &[evap])?;
        // model.set_value(
        //     "atmosphere_water__liquid_equivalent_precipitation_rate",
        //     &[100.0],
        // )?;
        // model.set_value("water_potential_evaporation_flux", &[0.0])?;

        model.update()?;
        step += 1;

        let current_time = model.get_current_time()?;

        // Print progress every 10 steps or at the end
        if step % 1 == 0 || current_time >= end_time {
            print!("Step {}: time = {:.2}", step, current_time);

            // Try to print the first output variable's value
            if let Some(var_name) = first_output {
                // Try to get as f64 first, then try f32
                if let Ok(values) = model.get_value::<f64>(var_name) {
                    if !values.is_empty() {
                        if values.len() == 1 {
                            print!(", {} = {:.10}", var_name, values[0]);
                        } else {
                            print!(", {} = [{:.10}, ...]", var_name, values[0]);
                        }
                    }
                } else if let Ok(values) = model.get_value::<f32>(var_name) {
                    if !values.is_empty() {
                        if values.len() == 1 {
                            print!(", {} = {:.10}", var_name, values[0]);
                        } else {
                            print!(", {} = [{:.10}, ...]", var_name, values[0]);
                        }
                    }
                }
            }

            println!();
        }
    }

    println!();
    println!("✓ Completed {} steps", step);

    Ok(())
}
