//! Example usage of the BMI Rust adapter.
//!
//! This demonstrates how to load, initialize, run, and interact with
//! BMI-compliant models using either C or Fortran interfaces.

use bmi::{preload_dependencies, Bmi, BmiC, BmiError, BmiExt, BmiFortran};
use std::env;
use std::path::PathBuf;

fn main() -> Result<(), BmiError> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 5 {
        eprintln!(
            "Usage: {} <model_type> <library_path> <registration_func> <config_file> [middleware_library]",
            args[0]
        );
        eprintln!();
        eprintln!("Model types: c, fortran");
        eprintln!();
        eprintln!("Examples:");
        eprintln!(
            "  {} c ./libheat.so register_bmi_heat ./config.yaml",
            args[0]
        );
        eprintln!(
            "  {} fortran ./libfortran_model.so create_bmi ./config.yaml ./libbmi_fortran.so",
            args[0]
        );
        eprintln!(
            "  {} fortran ./libfortran_model.so create_bmi ./config.yaml  # if middleware is in same library",
            args[0]
        );
        std::process::exit(1);
    }

    let model_type = &args[1];
    let library_path = PathBuf::from(&args[2]);
    let registration_func = &args[3];
    let config_file = PathBuf::from(&args[4]);
    let middleware_path = args.get(5).map(PathBuf::from);

    println!("Loading BMI model...");
    println!("  Type: {}", model_type);
    println!("  Library: {}", library_path.display());
    println!("  Registration function: {}", registration_func);
    println!("  Config file: {}", config_file.display());
    if let Some(ref mw) = middleware_path {
        println!("  Middleware library: {}", mw.display());
    }
    println!();

    // Preload common dependencies
    preload_dependencies()?;

    // Load the appropriate model type and use it via the trait
    let mut model: Box<dyn Bmi> = match model_type.as_str() {
        "c" => Box::new(BmiC::load("bmi_c_model", &library_path, registration_func)?),
        "fortran" => {
            if let Some(ref mw_path) = middleware_path {
                Box::new(BmiFortran::load(
                    "bmi_fortran_model",
                    &library_path,
                    mw_path,
                    registration_func,
                )?)
            } else {
                Box::new(BmiFortran::load_single_library(
                    "bmi_fortran_model",
                    &library_path,
                    registration_func,
                )?)
            }
        }
        _ => {
            eprintln!("Unknown model type: {}. Use 'c' or 'fortran'.", model_type);
            std::process::exit(1);
        }
    };

    println!("✓ Model loaded successfully");

    // Initialize
    model.initialize(config_file.to_str().unwrap())?;
    println!("✓ Model initialized");
    println!();

    // Print model information
    print_model_info(model.as_ref())?;

    // Run the model
    run_model(model.as_mut())?;

    // Finalize
    model.finalize()?;
    println!("✓ Model finalized");

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
    let mut step = 0;

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
        model.update()?;
        step += 1;

        let current_time = model.get_current_time()?;

        // Print progress every 10 steps or at the end
        if step % 10 == 0 || current_time >= end_time {
            print!("Step {}: time = {:.2}", step, current_time);

            // Try to print the first output variable's value
            if let Some(ref var_name) = first_output {
                // Try f64 first
                if let Ok(values) = model.get_value_f64(var_name) {
                    if !values.is_empty() {
                        if values.len() == 1 {
                            print!(", {} = {:.6}", var_name, values[0]);
                        } else {
                            print!(", {} = [{:.6}, ...]", var_name, values[0]);
                        }
                    }
                }
                // Could also try f32 or i32 if f64 fails
            }

            println!();
        }
    }

    println!();
    println!("✓ Completed {} steps", step);

    Ok(())
}
