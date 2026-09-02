//! Embedding API: run a model chain for one location and get the results back in memory.
//!
//! This is the layer other Rust programs should use. It owns a [`ModelRunner`], resolves which
//! output variables the config asks for, and hands back plain `Vec<f64>` columns instead of
//! writing files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bmi_core::error::{function_failed, BmiResult};

use crate::config::{parse_datetime, RealizationConfig};
use crate::runner::ModelRunner;

/// Results of running one location.
pub struct RunOutput {
    pub location: String,
    /// Unix epoch seconds of the first timestep.
    pub start_epoch: i64,
    /// Seconds between timesteps.
    pub interval: i64,
    /// Output columns, one entry per variable, each of length [`RunOutput::len`].
    pub variables: HashMap<String, Vec<f64>>,
}

impl RunOutput {
    pub fn get(&self, name: &str) -> Option<&[f64]> {
        self.variables.get(name).map(|v| v.as_slice())
    }

    /// Take ownership of one variable's values, leaving the rest behind.
    pub fn take(&mut self, name: &str) -> Option<Vec<f64>> {
        self.variables.remove(name)
    }

    /// Number of timesteps. Zero if the run produced no variables.
    pub fn len(&self) -> usize {
        self.variables.values().next().map_or(0, |v| v.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Unix epoch seconds for each timestep.
    pub fn times(&self) -> impl Iterator<Item = i64> + '_ {
        let (start, interval) = (self.start_epoch, self.interval);
        (0..self.len() as i64).map(move |step| start + step * interval)
    }

    pub fn variable_names(&self) -> impl Iterator<Item = &str> {
        self.variables.keys().map(|s| s.as_str())
    }
}

/// A configured model chain, ready to run locations one at a time.
///
/// The config is parsed once and the forcing file is opened once; each [`Simulation::run`] call
/// loads fresh model instances for that location and drops them afterwards, which is what the
/// models' own lifecycle requires.
///
/// Not `Send`: see the crate-level docs on threads. Build one per worker thread.
pub struct Simulation {
    runner: ModelRunner,
    /// `None` means "every variable the models expose".
    output_vars: Option<Vec<String>>,
    start_epoch: i64,
    interval: i64,
}

impl Simulation {
    /// Open `<data_dir>/config/realization.json`.
    pub fn open(data_dir: impl AsRef<Path>) -> BmiResult<Self> {
        let data_dir = data_dir.as_ref();
        Self::with_config(data_dir, data_dir.join("config").join("realization.json"))
    }

    /// Open an explicit config file. Relative paths inside it resolve against `data_dir`,
    /// not against the config file's own location and not against the process working directory.
    pub fn with_config(data_dir: impl AsRef<Path>, config: impl AsRef<Path>) -> BmiResult<Self> {
        Self::from_runner(ModelRunner::from_config(config, data_dir.as_ref())?)
    }

    /// Build from an already-parsed config.
    pub fn from_realization(
        data_dir: impl AsRef<Path>,
        config: RealizationConfig,
    ) -> BmiResult<Self> {
        Self::from_runner(ModelRunner::new(config, data_dir.as_ref())?)
    }

    fn from_runner(mut runner: ModelRunner) -> BmiResult<Self> {
        runner.suppress_warnings = true;
        let start_epoch = parse_datetime(&runner.config.time.start_time)?;
        let interval = runner.config.time.output_interval;
        let output_vars = resolve_output_vars(&runner.config);
        Ok(Self {
            runner,
            output_vars,
            start_epoch,
            interval,
        })
    }

    /// Print unit-conversion and missing-mapping warnings while running. Off by default, because
    /// a library writing to stderr on every location is rarely what the host program wants.
    pub fn set_warnings(&mut self, enabled: bool) {
        self.runner.suppress_warnings = !enabled;
    }

    /// Restrict the output to these variables, overriding the config's `output_variables`.
    pub fn set_output_variables(&mut self, vars: impl IntoIterator<Item = String>) {
        self.output_vars = Some(vars.into_iter().collect());
    }

    /// Run every variable the config asks for.
    pub fn run(&mut self, location: &str) -> BmiResult<RunOutput> {
        self.runner.initialize(location)?;
        self.runner.run()?;

        let variables = match &self.output_vars {
            None => self.runner.outputs.clone(),
            Some(names) => names
                .iter()
                .filter_map(|name| {
                    self.runner
                        .outputs(name)
                        .ok()
                        .map(|vals| (name.clone(), vals.clone()))
                })
                .collect(),
        };

        self.runner.finalize()?;

        Ok(RunOutput {
            location: location.to_string(),
            start_epoch: self.start_epoch,
            interval: self.interval,
            variables,
        })
    }

    /// Run and return a single variable. Cheaper to call than [`Simulation::run`] when you only
    /// want one column, and it fails loudly if the variable is missing rather than returning an
    /// empty map.
    pub fn run_variable(&mut self, location: &str, variable: &str) -> BmiResult<Vec<f64>> {
        self.runner.initialize(location)?;
        self.runner.run()?;
        let values = self.runner.outputs(variable).cloned();
        self.runner.finalize()?;
        values
    }

    /// Units of a model output or forcing variable, as reported by the model. Empty if unknown.
    ///
    /// Only meaningful once the models have been loaded, so this initializes the chain for
    /// `location` if it is not already loaded.
    pub fn var_units(&mut self, location: &str, variable: &str) -> BmiResult<String> {
        if self.runner.location() != location || self.runner.model_count() == 0 {
            self.runner.initialize(location)?;
        }
        Ok(self.runner.var_units(variable))
    }

    /// Location ids from the hydrofabric GeoPackage in `<data_dir>/config/`.
    #[cfg(feature = "gpkg")]
    pub fn locations(&self) -> BmiResult<Vec<String>> {
        crate::locations::find_gpkg_locations(&self.runner.base_dir)
    }

    /// Number of output timesteps per location.
    pub fn total_steps(&self) -> usize {
        self.runner.total_steps
    }

    pub fn start_epoch(&self) -> i64 {
        self.start_epoch
    }

    pub fn output_interval(&self) -> i64 {
        self.interval
    }

    pub fn config(&self) -> &RealizationConfig {
        &self.runner.config
    }

    /// The underlying runner, for callers that need the individual model instances.
    pub fn runner(&self) -> &ModelRunner {
        &self.runner
    }

    pub fn runner_mut(&mut self) -> &mut ModelRunner {
        &mut self.runner
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.runner.base_dir
    }
}

/// Run one location and return one variable, opening and closing everything in the process.
///
/// Convenient for a single run; if you are looping over locations build a [`Simulation`] instead
/// so the config parse and forcing file open are not repeated.
pub fn run_location(
    data_dir: impl AsRef<Path>,
    location: &str,
    variable: &str,
) -> BmiResult<Vec<f64>> {
    Simulation::open(data_dir)?.run_variable(location, variable)
}

/// Resolve which output variables to write based on the realization config.
///
/// - `output_variables: ["all"]` -> None (every available variable)
/// - `output_variables: [...]`   -> Some(list)
/// - `output_variables` empty    -> Some(vec![main_output_variable])
pub fn resolve_output_vars(config: &RealizationConfig) -> Option<Vec<String>> {
    let formulation = config.global.formulations.first()?;
    let output_vars = &formulation.params.output_variables;

    if output_vars.len() == 1 && output_vars[0].eq_ignore_ascii_case("all") {
        None
    } else if output_vars.is_empty() {
        let main = &formulation.params.main_output_variable;
        if main.is_empty() {
            None
        } else {
            Some(vec![main.clone()])
        }
    } else {
        Some(output_vars.clone())
    }
}

/// Number of output timesteps implied by a config's time range.
pub fn total_steps(config: &RealizationConfig) -> BmiResult<usize> {
    let start = parse_datetime(&config.time.start_time)?;
    let end = parse_datetime(&config.time.end_time)?;
    let interval = config.time.output_interval;
    if interval <= 0 {
        return Err(function_failed("config", "output_interval must be positive"));
    }
    Ok(((end - start) / interval) as usize)
}
