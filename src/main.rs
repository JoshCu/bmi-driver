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
    let SPFH_2maboveground = vec![
        0.00380436, 0.00263561, 0.00171654, 0.00098395, 0.00088405, 0.0008066, 0.00070654,
        0.00070304, 0.0007, 0.0007, 0.0007, 0.0007, 0.0007, 0.00118412, 0.00181064, 0.00254062,
        0.00264073, 0.00273551, 0.00282243, 0.0029636, 0.00315558, 0.00334472, 0.00360349,
        0.0038095, 0.00399733,
    ];
    let precip_rate = vec![0.0002; 25];
    let TMP_2maboveground = vec![
        287.3306, 283.00186, 278.66342, 274.3335, 273.8761, 273.4151, 272.94516, 272.89264, 272.85,
        272.80078, 272.35684, 271.9021, 271.46396, 274.8715, 278.2935, 281.69907, 284.6559,
        287.57828, 290.5192, 291.2784, 292.05933, 292.82294, 291.04248, 289.25797, 287.46176,
    ];
    let UGRD_10maboveground = vec![
        1.2, 1.7000346, 2.1996388, 2.699006, 2.4929843, 2.305369, 2.0752988, 2.073942, 2.0625496,
        2.054329, 1.906676, 1.7300266, 1.5876702, 1.2235489, 0.85219723, 0.4969373, 0.5730691,
        0.64374685, 0.70793855, 0.97199005, 1.200095, 1.4499584, 1.1, 0.79057455, 0.40678793,
    ];
    let VGRD_10maboveground = vec![
        0., -0.7200795, -1.4191973, -2.123727, -1.9748795, -1.839585, -1.6936963, -1.5441712,
        -1.3863225, -1.240015, -1.4458387, -1.6690408, -1.8802783, -1.8317194, -1.7996365,
        -1.7561393, -1.6363872, -1.5447624, -1.4536467, -1.8375547, -2.2013934, -2.5808604,
        -2.2097566, -1.8863903, -1.4983606,
    ];
    let DLWRF_surface = vec![
        203.23927, 197.01161, 190.80948, 177.25778, 176.66985, 176.14465, 176.21483, 176.17572,
        176.16026, 177.60666, 177.11806, 176.63808, 169.04381, 173.26738, 177.51637, 198.97624,
        202.90031, 206.90909, 232.38126, 233.5114, 234.692, 240.30154, 237.64606, 234.98735,
        213.07878,
    ];
    let DSWRF_surface = vec![
        137.2865, 7.663944, 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 90.378716, 274.69037,
        421.05798, 558.2634, 641.3965, 675.7488, 662.2209, 606.29626, 491.62857, 352.87973,
        129.08543,
    ];
    let PRES_surface = vec![
        70373.52, 70437.02, 70507.17, 70571.516, 70547.695, 70524.74, 70501.25, 70475.98, 70448.41,
        70423.734, 70440.586, 70458.734, 70475.62, 70532.75, 70586.555, 70646.06, 70638.984,
        70633.125, 70629.734, 70705.05, 70779.984, 70850.914, 70824.336, 70797.86, 70769.664,
    ];

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
        model.set_value("PRCPNONC", &[precip_rate[step]])?;
        model.set_value("Q2", &[SPFH_2maboveground[step]])?;
        model.set_value("SFCTMP", &[TMP_2maboveground[step]])?;
        model.set_value("UU", &[UGRD_10maboveground[step]])?;
        model.set_value("VV", &[VGRD_10maboveground[step]])?;
        model.set_value("LWDN", &[DLWRF_surface[step]])?;
        model.set_value("SOLDN", &[DSWRF_surface[step]])?;
        model.set_value("SFCPRS", &[PRES_surface[step]])?;
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
