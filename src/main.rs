use bmi_driver::{preload_dependencies, BmiError, DivideDataStore, ModelRunner, OutputFormat};
use clap::Parser;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

#[derive(Debug, Clone, Copy, clap::ValueEnum, serde::Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ProgressMode {
    /// One progress bar per worker plus overall total
    Full,
    /// Single overall progress bar only
    Summary,
    /// No progress output
    None,
}

#[derive(serde::Deserialize, Default)]
struct TomlConfig {
    jobs: Option<usize>,
    progress: Option<ProgressMode>,
}

fn load_toml_config() -> TomlConfig {
    let config_path = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("bmi-driver")
        .join("config.toml");

    if let Ok(contents) = fs::read_to_string(&config_path) {
        toml::from_str(&contents).unwrap_or_else(|e| {
            eprintln!("Warning: failed to parse {}: {}", config_path.display(), e);
            TomlConfig::default()
        })
    } else {
        TomlConfig::default()
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    data_dir: PathBuf,

    /// Number of parallel worker processes (default: number of CPUs)
    #[arg(short = 'j', long)]
    jobs: Option<usize>,

    /// Progress bar display mode
    #[arg(long, value_enum)]
    progress: Option<ProgressMode>,

    /// Internal: start index for worker mode (inclusive)
    #[arg(long, hide = true)]
    worker_start: Option<usize>,

    /// Internal: end index for worker mode (exclusive)
    #[arg(long, hide = true)]
    worker_end: Option<usize>,

    /// Print all unit conversion info and exit without running
    #[arg(long)]
    units: bool,

    /// Minify realization.json, removing fields bmi-driver doesn't use
    #[arg(long)]
    minify: bool,

    /// Index of the first location to process on this node (for SLURM/multi-node use)
    #[arg(long, default_value_t = 0)]
    node_start: usize,

    /// Number of locations to process on this node (0 = all remaining, for SLURM/multi-node use)
    #[arg(long, default_value_t = 0)]
    node_count: usize,

    /// Internal: output variable names for zarr workers (comma-separated)
    #[arg(long, hide = true)]
    output_vars: Option<String>,
}

fn main() -> Result<(), BmiError> {
    let args = Args::parse();
    let data_dir = fs::canonicalize(&args.data_dir).unwrap();
    let config_dir = data_dir.join("config");
    let _ = env::set_current_dir(&data_dir);

    let realization = config_dir.join("realization.json");

    if args.minify {
        bmi_driver::config::minify_file(&realization)?;
        eprintln!("Minified {}", realization.display());
        return Ok(());
    }

    preload_dependencies();

    let db_path = config_dir
        .read_dir()
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().map_or(false, |ext| ext == "gpkg"))
        .unwrap()
        .path();

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let mut stmt = conn.prepare("SELECT divide_id FROM 'divides'").unwrap();
    let rows = stmt
        .query_map([], |row| Ok(row.get::<_, String>(0)?))
        .unwrap();
    let locations: Vec<String> = rows.flatten().collect();

    // Apply node-level partitioning for multi-node / SLURM job-array use.
    // --node-start and --node-count carve out this node's slice of the location list
    // before the internal worker processes further sub-divide it with -j.
    let node_start = args.node_start.min(locations.len());
    let node_end = if args.node_count > 0 {
        (node_start + args.node_count).min(locations.len())
    } else {
        locations.len()
    };
    let locations = locations[node_start..node_end].to_vec();

    if args.units {
        return print_units(&realization, &locations);
    }

    // Merge: CLI > TOML > default
    let toml_cfg = load_toml_config();
    let jobs = args.jobs.or(toml_cfg.jobs);
    let progress = args
        .progress
        .or(toml_cfg.progress)
        .unwrap_or(ProgressMode::Summary);

    if let (Some(start), Some(end)) = (args.worker_start, args.worker_end) {
        let output_path = data_dir.join("outputs").join("bmi-driver");
        run_worker(&realization, &locations[start..end], &output_path, start)
    } else {
        run_parent(&data_dir, &realization, &locations, jobs, progress)
    }
}

fn print_units(realization: &PathBuf, locations: &[String]) -> Result<(), BmiError> {
    let mut runner = ModelRunner::from_config(realization)?;
    if let Some(loc) = locations.first() {
        runner.initialize(loc)?;
        runner.print_unit_conversions(false);
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
    progress: ProgressMode,
) -> Result<(), BmiError> {
    // Check for missing mappings and print active unit conversions
    let output_format;
    #[cfg(feature = "zarr")]
    let mut discovered_vars: Vec<String> = Vec::new();
    {
        let mut runner = ModelRunner::from_config(realization)?;
        output_format = runner.config.output_format;
        if let Some(loc) = locations.first() {
            runner.initialize(loc)?;

            // Discover output variable names for zarr (needs to create arrays upfront)
            #[cfg(feature = "zarr")]
            {
                let config_output_vars: Vec<String> = runner
                    .config
                    .global
                    .formulations
                    .first()
                    .map(|f| f.params.output_variables.clone())
                    .unwrap_or_default();
                if config_output_vars.is_empty() {
                    for model in &runner.models {
                        if let Ok(names) = model.model.get_output_var_names() {
                            for name in names {
                                if !discovered_vars.contains(&name) {
                                    discovered_vars.push(name);
                                }
                            }
                        }
                    }
                } else {
                    discovered_vars = config_output_vars;
                }
            }

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
                    runner2.print_unit_conversions(true);
                    runner2.finalize()?;
                } else {
                    eprintln!("Skipping. Running with current config.");
                    runner.print_unit_conversions(true);
                    runner.finalize()?;
                }
            } else {
                runner.print_unit_conversions(true);
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

    // For zarr: create the store before spawning workers so they can write directly
    #[cfg(feature = "zarr")]
    let output_vars_csv: String = if output_format == OutputFormat::Zarr {
        let runner_tmp = ModelRunner::from_config(realization)?;
        let start_time = &runner_tmp.config.time.start_time;
        let interval = runner_tmp.config.time.output_interval;
        let start_epoch = bmi_driver::parse_datetime(start_time)?;
        let end_epoch = bmi_driver::parse_datetime(&runner_tmp.config.time.end_time)?;
        let total_steps = ((end_epoch - start_epoch) / interval) as usize;
        let zarr_path = output_path.join("results.zarr");

        bmi_driver::output::zarr::create_zarr_store(
            &zarr_path,
            start_time,
            interval,
            total_steps,
            locations,
            &discovered_vars,
        )?;
        discovered_vars.join(",")
    } else {
        String::new()
    };

    let mp = MultiProgress::new();

    // Overall progress bar (shown for Full and Summary)
    let overall_pb = if progress != ProgressMode::None {
        let pb = mp.add(ProgressBar::new(n_locations as u64));
        pb.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} ({per_sec}) {msg}",
            )
            .unwrap()
            .progress_chars("━╸─"),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(100));
        pb.set_message("total");
        Some(pb)
    } else {
        None
    };

    // Per-worker style (only used in Full mode)
    let worker_style = if progress == ProgressMode::Full {
        Some(
            ProgressStyle::with_template("  worker {msg}: {bar:30.white/black} {pos}/{len}")
                .unwrap()
                .progress_chars("━╸─"),
        )
    } else {
        None
    };

    let mut handles = Vec::new();

    for i in 0..n_workers {
        let start = i * chunk_size;
        if start >= n_locations {
            break;
        }
        let end = ((i + 1) * chunk_size).min(n_locations);
        let worker_count = (end - start) as u64;

        let mut cmd = Command::new(&exe);
        cmd.arg(data_dir)
            .arg("--worker-start")
            .arg(start.to_string())
            .arg("--worker-end")
            .arg(end.to_string());

        #[cfg(feature = "zarr")]
        if output_format == OutputFormat::Zarr && !output_vars_csv.is_empty() {
            cmd.arg("--output-vars").arg(&output_vars_csv);
        }

        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| BmiError::FunctionFailed {
                model: "runner".into(),
                func: format!("Failed to spawn worker: {}", e),
            })?;

        let worker_pb = worker_style.as_ref().map(|style| {
            let pb = mp.add(ProgressBar::new(worker_count));
            pb.set_style(style.clone());
            pb.set_message(format!("{}", i));
            pb
        });

        let stdout = child.stdout.take().unwrap();
        let overall_pb = overall_pb.clone();

        let handle = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            let mut prev = 0u64;
            for line in reader.lines().flatten() {
                if let Ok(count) = line.trim().parse::<u64>() {
                    let delta = count - prev;
                    if let Some(pb) = &worker_pb {
                        pb.inc(delta);
                    }
                    if let Some(pb) = &overall_pb {
                        pb.inc(delta);
                    }
                    prev = count;
                }
            }
            if let Some(pb) = worker_pb {
                pb.finish_and_clear();
            }
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
        if let Some(pb) = &overall_pb {
            pb.finish_with_message("failed");
        }
        return Err(BmiError::FunctionFailed {
            model: "runner".into(),
            func: "One or more workers failed".into(),
        });
    }

    if let Some(pb) = &overall_pb {
        pb.finish_with_message("done ✓");
    }

    // Merge per-worker NetCDF files into a single output file
    if output_format == OutputFormat::Netcdf {
        let mut worker_files = Vec::new();
        for i in 0..n_workers {
            let start = i * chunk_size;
            if start >= n_locations {
                break;
            }
            let first_loc = &locations[start];
            let tmp_path = output_path.join(format!("tmp_{}.nc", first_loc));
            if tmp_path.exists() {
                worker_files.push(tmp_path);
            }
        }
        if !worker_files.is_empty() {
            let final_path = output_path.join("results.nc");
            bmi_driver::output::netcdf::merge_netcdf_files(&worker_files, &final_path)?;
        }
    }
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
                    if !params
                        .get("variables_names_map")
                        .is_some_and(|v| v.is_object())
                    {
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

    let updated = serde_json::to_string_pretty(&root).map_err(|e| BmiError::FunctionFailed {
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
    #[cfg_attr(not(feature = "zarr"), allow(unused))] global_start: usize,
) -> Result<(), BmiError> {
    let mut runner = ModelRunner::from_config(realization)?;
    runner.suppress_warnings = true;
    let start_epoch = bmi_driver::parse_datetime(&runner.config.time.start_time)?;
    let interval = runner.config.time.output_interval;
    let output_format = runner.config.output_format;
    let output_vars: Vec<String> = runner
        .config
        .global
        .formulations
        .first()
        .map(|f| f.params.output_variables.clone())
        .unwrap_or_default();

    let mut store = create_output_store(
        output_format,
        output_path,
        &runner,
        start_epoch,
        interval,
        global_start,
        locations.first().map(|s| s.as_str()),
    )?;

    let report_interval = ((locations.len() as f64) * 0.01).ceil().max(1.0) as usize;

    for (i, location) in locations.iter().enumerate() {
        runner.initialize(location)?;
        runner.run()?;

        let columns = collect_columns(&runner, &output_vars);
        store.write_location(location, &columns)?;

        runner.finalize()?;

        if (i + 1) % report_interval == 0 || i + 1 == locations.len() {
            println!("{}", i + 1);
        }
    }

    store.finish()?;
    Ok(())
}

fn collect_columns(runner: &ModelRunner, output_vars: &[String]) -> Vec<(String, Vec<f64>)> {
    if output_vars.is_empty() {
        runner
            .outputs
            .iter()
            .map(|(name, vals)| (name.clone(), vals.clone()))
            .collect()
    } else {
        output_vars
            .iter()
            .filter_map(|name| {
                runner
                    .outputs(name)
                    .ok()
                    .map(|vals| (name.clone(), vals.clone()))
            })
            .collect()
    }
}

fn create_output_store(
    format: OutputFormat,
    output_path: &PathBuf,
    runner: &ModelRunner,
    start_epoch: i64,
    interval: i64,
    #[cfg_attr(not(feature = "zarr"), allow(unused))] global_start: usize,
    first_location: Option<&str>,
) -> Result<Box<dyn DivideDataStore>, BmiError> {
    match format {
        OutputFormat::Csv => Ok(Box::new(bmi_driver::output::csv::CsvStore::new(
            output_path.clone(),
            start_epoch,
            interval,
        ))),
        OutputFormat::Netcdf => {
            let first_loc = first_location.unwrap_or("output");
            let nc_path = output_path.join(format!("tmp_{}.nc", first_loc));
            let start_time = &runner.config.time.start_time;
            let end_epoch = bmi_driver::parse_datetime(&runner.config.time.end_time)?;
            let total_steps = ((end_epoch - start_epoch) / interval) as usize;
            Ok(Box::new(bmi_driver::output::netcdf::NetCdfWriter::new(
                nc_path,
                start_time,
                interval,
                total_steps,
            )?))
        }
        #[cfg(feature = "zarr")]
        OutputFormat::Zarr => {
            let zarr_path = output_path.join("results.zarr");
            Ok(Box::new(bmi_driver::output::zarr::ZarrStore::new(
                zarr_path,
                global_start,
            )))
        }
    }
}
