use bmi_driver::{preload_dependencies, Bmi, BmiError, ModelRunner};
use clap::Parser;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

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

    /// Print all unit conversion info and exit without running
    #[arg(long)]
    units: bool,
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

    if args.units {
        return print_units(&realization, &locations);
    }

    if let (Some(start), Some(end)) = (args.worker_start, args.worker_end) {
        let output_path = data_dir.join("outputs").join("bmi-driver");
        run_worker(&realization, &locations[start..end], &output_path)
    } else {
        run_parent(&data_dir, &realization, &locations, args.jobs)
    }
}

fn print_units(realization: &PathBuf, locations: &[String]) -> Result<(), BmiError> {
    let mut runner = ModelRunner::from_config(realization)?;
    if let Some(loc) = locations.first() {
        runner.initialize(loc)?;
        runner.print_all_unit_info();
        runner.finalize()?;
    } else {
        eprintln!("No locations found.");
    }
    Ok(())
}

fn run_parent(
    data_dir: &PathBuf,
    realization: &PathBuf,
    locations: &[String],
    jobs: Option<usize>,
) -> Result<(), BmiError> {
    // Check for missing mappings and print active unit conversions
    {
        let mut runner = ModelRunner::from_config(realization)?;
        if let Some(loc) = locations.first() {
            runner.initialize(loc)?;

            let suggestions = runner.find_missing_mappings();
            if !suggestions.is_empty() {
                eprintln!("Found unmapped model inputs that match available variables:");
                for (i, s) in suggestions.iter().enumerate() {
                    eprintln!(
                        "  [{}] {}: \"{}\" ← \"{}\"",
                        i + 1,
                        s.model_name,
                        s.model_input,
                        s.suggested_source
                    );
                }
                eprintln!();
                eprint!("Add these mappings to realization.json? [y/N] ");
                io::stderr().flush().ok();

                let mut answer = String::new();
                if io::stdin().read_line(&mut answer).is_ok()
                    && answer.trim().eq_ignore_ascii_case("y")
                {
                    apply_suggestions(realization, &runner, &suggestions)?;
                    eprintln!("Updated {}. Restarting...", realization.display());
                    eprintln!();
                    runner.finalize()?;

                    // Re-initialize with updated config to show new conversions
                    let mut runner2 = ModelRunner::from_config(realization)?;
                    runner2.initialize(loc)?;
                    runner2.print_active_conversions();
                    runner2.finalize()?;
                } else {
                    eprintln!("Skipping. Running with current config.");
                    runner.print_active_conversions();
                    runner.finalize()?;
                }
            } else {
                runner.print_active_conversions();
                runner.finalize()?;
            }
        }
    }

    let n_workers = jobs.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1)
    });
    let n_locations = locations.len();
    let chunk_size = (n_locations + n_workers - 1) / n_workers;

    let exe = env::current_exe().unwrap();

    let output_path = data_dir.join("outputs").join("bmi-driver");
    fs::create_dir_all(&output_path).unwrap();

    let mp = MultiProgress::new();

    // Overall progress bar at the top
    let overall_pb = mp.add(ProgressBar::new(n_locations as u64));
    overall_pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} ({per_sec}) {msg}",
        )
        .unwrap()
        .progress_chars("━╸─"),
    );
    overall_pb.enable_steady_tick(std::time::Duration::from_millis(100));
    overall_pb.set_message("total");

    // Per-worker style
    let worker_style =
        ProgressStyle::with_template("  worker {msg}: {bar:30.white/black} {pos}/{len}")
            .unwrap()
            .progress_chars("━╸─");

    let mut handles = Vec::new();

    for i in 0..n_workers {
        let start = i * chunk_size;
        if start >= n_locations {
            break;
        }
        let end = ((i + 1) * chunk_size).min(n_locations);
        let worker_count = (end - start) as u64;

        let mut child = Command::new(&exe)
            .arg(data_dir)
            .arg("--worker-start")
            .arg(start.to_string())
            .arg("--worker-end")
            .arg(end.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| BmiError::FunctionFailed {
                model: "runner".into(),
                func: format!("Failed to spawn worker: {}", e),
            })?;

        let worker_pb = mp.add(ProgressBar::new(worker_count));
        worker_pb.set_style(worker_style.clone());
        worker_pb.set_message(format!("{}", i));

        let stdout = child.stdout.take().unwrap();
        let overall_pb = overall_pb.clone();

        let handle = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            let mut prev = 0u64;
            for line in reader.lines().flatten() {
                if let Ok(count) = line.trim().parse::<u64>() {
                    let delta = count - prev;
                    worker_pb.inc(delta);
                    overall_pb.inc(delta);
                    prev = count;
                }
            }
            worker_pb.finish_and_clear();
            child.wait()
        });
        handles.push(handle);
    }

    let mut failed = false;
    for handle in handles {
        match handle.join() {
            Ok(Ok(status)) if status.success() => {}
            Ok(Ok(status)) => {
                eprintln!("Worker exited with: {}", status);
                failed = true;
            }
            Ok(Err(e)) => {
                eprintln!("Worker error: {}", e);
                failed = true;
            }
            Err(_) => {
                eprintln!("Worker thread panicked");
                failed = true;
            }
        }
    }

    if failed {
        overall_pb.finish_with_message("failed");
        return Err(BmiError::FunctionFailed {
            model: "runner".into(),
            func: "One or more workers failed".into(),
        });
    }

    overall_pb.finish_with_message("done ✓");
    Ok(())
}

fn apply_suggestions(
    realization: &PathBuf,
    runner: &ModelRunner,
    suggestions: &[bmi_driver::runner::SuggestedMapping],
) -> Result<(), BmiError> {
    let content = fs::read_to_string(realization).map_err(|e| BmiError::FunctionFailed {
        model: "config".into(),
        func: format!("Failed to read {}: {}", realization.display(), e),
    })?;
    let mut root: serde_json::Value =
        serde_json::from_str(&content).map_err(|e| BmiError::FunctionFailed {
            model: "config".into(),
            func: format!("Failed to parse {}: {}", realization.display(), e),
        })?;

    // Group suggestions by model_idx
    let modules = root["global"]["formulations"]
        .as_array()
        .and_then(|f| f.first())
        .and_then(|f| f["params"]["modules"].as_array().map(|a| a.len()))
        .unwrap_or(0);

    // Build a map: model_name → module index in the JSON array.
    // The runner loads modules in dependency order which may differ from config order,
    // so match by model_type_name.
    let config_modules = runner.config.modules();

    for s in suggestions {
        // Find the module in the config that matches this model
        let module_json_idx = config_modules
            .iter()
            .position(|m| m.params.model_type_name == s.model_name);

        if let Some(idx) = module_json_idx {
            if idx < modules {
                // Navigate safely with get_mut to avoid creating null entries
                let params = root
                    .get_mut("global")
                    .and_then(|g| g.get_mut("formulations"))
                    .and_then(|f| f.get_mut(0))
                    .and_then(|f| f.get_mut("params"))
                    .and_then(|p| p.get_mut("modules"))
                    .and_then(|m| m.get_mut(idx))
                    .and_then(|m| m.get_mut("params"));

                if let Some(params) = params {
                    // Create variables_names_map if it doesn't exist
                    if !params.get("variables_names_map").is_some_and(|v| v.is_object()) {
                        params.as_object_mut().unwrap().insert(
                            "variables_names_map".to_string(),
                            serde_json::Value::Object(serde_json::Map::new()),
                        );
                    }
                    if let Some(map) = params
                        .get_mut("variables_names_map")
                        .and_then(|v| v.as_object_mut())
                    {
                        map.insert(
                            s.model_input.clone(),
                            serde_json::Value::String(s.suggested_source.clone()),
                        );
                    }
                }
            }
        }
    }

    let updated =
        serde_json::to_string_pretty(&root).map_err(|e| BmiError::FunctionFailed {
            model: "config".into(),
            func: format!("Failed to serialize: {}", e),
        })?;
    fs::write(realization, updated).map_err(|e| BmiError::FunctionFailed {
        model: "config".into(),
        func: format!("Failed to write {}: {}", realization.display(), e),
    })?;

    Ok(())
}

fn run_worker(
    realization: &PathBuf,
    locations: &[String],
    output_path: &PathBuf,
) -> Result<(), BmiError> {
    let mut runner = ModelRunner::from_config(realization)?;
    runner.suppress_warnings = true;
    let start_epoch = bmi_driver::parse_datetime(&runner.config.time.start_time)?;
    let interval = runner.config.time.output_interval;
    let output_vars: Vec<String> = runner
        .config
        .global
        .formulations
        .first()
        .map(|f| f.params.output_variables.clone())
        .unwrap_or_default();

    let report_interval = ((locations.len() as f64) * 0.01).ceil().max(1.0) as usize;

    for (i, location) in locations.iter().enumerate() {
        runner.initialize(location)?;
        runner.run()?;

        let columns: Vec<(&str, &Vec<f64>)> = if output_vars.is_empty() {
            runner
                .outputs
                .iter()
                .map(|(name, vals)| (name.as_str(), vals))
                .collect()
        } else {
            output_vars
                .iter()
                .filter_map(|name| runner.outputs(name).ok().map(|vals| (name.as_str(), vals)))
                .collect()
        };

        let csv_path = output_path.join(format!("{}.csv", location));
        let mut csv = String::from("Time Step,Time");
        for (name, _) in &columns {
            csv.push(',');
            csv.push_str(name);
        }
        csv.push('\n');

        let n_steps = columns.first().map(|(_, v)| v.len()).unwrap_or(0);
        for step in 0..n_steps {
            let ts = start_epoch + (step as i64) * interval;
            csv.push_str(&format!("{},{}", step, format_epoch(ts)));
            for (_, vals) in &columns {
                csv.push_str(&format!(",{:.9}", vals[step]));
            }
            csv.push('\n');
        }

        fs::write(&csv_path, csv).map_err(|e| BmiError::FunctionFailed {
            model: "runner".into(),
            func: format!("Failed to write {}: {}", csv_path.display(), e),
        })?;

        runner.finalize()?;

        if (i + 1) % report_interval == 0 || i + 1 == locations.len() {
            println!("{}", i + 1);
        }
    }
    Ok(())
}

fn format_epoch(epoch: i64) -> String {
    let secs_per_day: i64 = 86400;
    let mut remaining = epoch;
    let sec = remaining % 60;
    remaining /= 60;
    let min = remaining % 60;
    remaining /= 60;
    let hour = remaining % 24;
    let mut days = epoch / secs_per_day;

    let leap = |y: i32| (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days_in_month = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    let mut year = 1970i32;
    loop {
        let yd = if leap(year) { 366 } else { 365 };
        if days < yd {
            break;
        }
        days -= yd;
        year += 1;
    }

    let mut month = 0u32;
    for m in 0..12 {
        let md = days_in_month[m] as i64 + if m == 1 && leap(year) { 1 } else { 0 };
        if days < md {
            month = m as u32 + 1;
            break;
        }
        days -= md;
    }
    let day = days + 1;

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hour, min, sec
    )
}
