use bmi_driver::{preload_dependencies, Bmi, BmiError, ModelRunner};
use clap::Parser;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::process::Command;
use std::env;
use std::fs;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    data_dir: PathBuf,

    /// Number of parallel worker processes (default: number of CPUs)
    #[arg(short = 'j', long)]
    jobs: Option<usize>,

    /// Internal: start index for worker mode (inclusive)
    #[arg(long, hide = true)]
    worker_start: Option<usize>,

    /// Internal: end index for worker mode (exclusive)
    #[arg(long, hide = true)]
    worker_end: Option<usize>,
}

fn main() -> Result<(), BmiError> {
    let args = Args::parse();
    let data_dir = fs::canonicalize(&args.data_dir).unwrap();
    let config_dir = data_dir.join("config");
    let _ = env::set_current_dir(&data_dir);

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

    if let (Some(start), Some(end)) = (args.worker_start, args.worker_end) {
        // Worker mode: process assigned slice of locations
        run_worker(&realization, &locations[start..end])
    } else {
        // Parent mode: spawn worker processes
        run_parent(&data_dir, &locations, args.jobs)
    }
}

fn run_parent(
    data_dir: &PathBuf,
    locations: &[String],
    jobs: Option<usize>,
) -> Result<(), BmiError> {
    let n_workers = jobs.unwrap_or_else(|| std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1));
    let n_locations = locations.len();
    let chunk_size = (n_locations + n_workers - 1) / n_workers;

    let exe = env::current_exe().unwrap();

    let output_path = data_dir.join("outputs").join("bmi-driver");
    fs::create_dir_all(output_path).unwrap();

    let pb = ProgressBar::new(n_locations as u64);
    pb.set_style(
        ProgressStyle::with_template("{bar:40} {pos}/{len} {msg}")
            .unwrap()
            .progress_chars("##-"),
    );

    let mut children = Vec::new();
    for i in 0..n_workers {
        let start = i * chunk_size;
        if start >= n_locations {
            break;
        }
        let end = ((i + 1) * chunk_size).min(n_locations);

        let child = Command::new(&exe)
            .arg(data_dir)
            .arg("--worker-start")
            .arg(start.to_string())
            .arg("--worker-end")
            .arg(end.to_string())
            .spawn()
            .map_err(|e| BmiError::FunctionFailed {
                model: "runner".into(),
                func: format!("Failed to spawn worker: {}", e),
            })?;
        children.push((child, end - start));
    }

    pb.set_message(format!("{} workers", children.len()));

    let mut failed = false;
    for (mut child, count) in children {
        let status = child.wait().map_err(|e| BmiError::FunctionFailed {
            model: "runner".into(),
            func: format!("Worker error: {}", e),
        })?;
        if !status.success() {
            eprintln!("Worker exited with: {}", status);
            failed = true;
        }
        pb.inc(count as u64);
    }

    if failed {
        pb.finish_with_message("failed");
        return Err(BmiError::FunctionFailed {
            model: "runner".into(),
            func: "One or more workers failed".into(),
        });
    }

    pb.finish_with_message("done");
    Ok(())
}

fn run_worker(realization: &PathBuf, locations: &[String]) -> Result<(), BmiError> {
    let mut runner = ModelRunner::from_config(realization)?;
    let output_path = data_dir.join("outputs").join("bmi-driver");
    for location in locations {
        runner.initialize(location)?;
        runner.run()?;
        let _outputs = runner.main_outputs()?;
        let cat_out = output_path.join(format!("{}.csv",location))
        // output needs to match
        // Time Step,Time,Q_OUT
        // 0,2010-01-01 00:00:00,0.000809171
        // 1,2010-01-01 01:00:00,0.000736643
        // for val in _outputs{println!("{} {:.9}",location,val);}
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
