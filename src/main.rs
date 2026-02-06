use anyhow::{Context, Result};
use bmi_driver::{preload_dependencies, Bmi, BmiError, BmiExt, ModelRunner};
use clap::{command, Parser};
use std::path::PathBuf;
use std::process::{Child, Command};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    data_dir: PathBuf,
    // info: bool,
}

fn main() -> Result<(), BmiError> {
    let args = Args::parse();
    let data_dir = args.data_dir;
    let config_dir = data_dir.join("config");

    // Parent process: read locations from gpkg and spawn workers
    preload_dependencies();

    let db_path = config_dir
        .read_dir()
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().map_or(false, |ext| ext == "gpkg"))
        .unwrap()
        .path();

    let realization = config_dir.join("realization.json");

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let mut stmt = conn.prepare("SELECT divide_id FROM 'divides'").unwrap();
    let rows = stmt
        .query_map([], |row| Ok(row.get::<_, String>(0)?))
        .unwrap();
    let locations: Vec<String> = rows.flatten().collect();
    let mut runner = ModelRunner::from_config(realization)?;
    // if args.info {
    //     for instance in runner.models {
    //         let model = instance.model;
    //         print_model_info(model)
    //     }
    // }
    for location in locations {
        runner.initialize(&location)?;
        runner.run()?;
        let _outputs = runner.main_outputs()?;
        // for val in _outputs {
        //     println!("{:.9}", val);
        // }
        runner.finalize()?;
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
    for val in _outputs {
        println!("{}", val);
    }
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
