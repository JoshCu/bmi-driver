use std::collections::HashMap;
use std::path::Path;

#[cfg(feature = "fortran")]
use crate::adapters::BmiFortran;
#[cfg(feature = "python")]
use crate::adapters::BmiPython;
use crate::adapters::{BmiC, BmiSloth};
use crate::aliases;
use crate::config::{
    parse_datetime, BmiAdapterType, DownsampleMode, ModuleConfig, RealizationConfig, UpsampleMode,
};
use crate::error::{function_failed, BmiResult};

const TIMESTEP_EPSILON: f64 = 1e-9;
use crate::forcings::{Forcings, NetCdfForcings};
use crate::resample;
use crate::traits::{Bmi, BmiExt};
use crate::units::UnitConversion;

#[derive(Debug, Clone)]
pub struct TimestepInfo {
    pub dt_seconds: f64,
    pub num_steps: usize,
}

pub struct ModelInstance {
    pub name: String,
    pub model: Box<dyn Bmi>,
    pub input_map: HashMap<String, String>,
    pub main_output: String,
    /// Unit conversions keyed by model input variable name.
    pub input_conversions: HashMap<String, UnitConversion>,
    pub timestep_info: TimestepInfo,
    pub downsample_mode: DownsampleMode,
    pub upsample_mode: UpsampleMode,
}

/// A suggested variable mapping that could be added to the realization config.
#[derive(Debug, Clone)]
pub struct SuggestedMapping {
    pub model_name: String,
    pub model_idx: usize,
    pub model_input: String,
    pub suggested_source: String,
}

#[derive(Debug, Clone)]
pub enum VarSource {
    Forcing,
    Model(usize),
}

pub struct ModelRunner {
    pub config: RealizationConfig,
    pub forcings: NetCdfForcings,
    pub vars: HashMap<String, VarSource>,
    pub models: Vec<ModelInstance>,
    pub total_steps: usize,
    pub location_id: String,
    #[cfg(feature = "fortran")]
    pub fortran_middleware: Option<String>,
    pub outputs: HashMap<String, Vec<f64>>,
    pub final_outputs: Vec<f64>,
    pub has_run: bool,
    pub suppress_warnings: bool,
    pub source_timesteps: HashMap<String, TimestepInfo>,
    pub simulation_span_seconds: f64,
}

impl ModelRunner {
    pub fn from_config(path: impl AsRef<Path>) -> BmiResult<Self> {
        Self::new(RealizationConfig::from_file(path)?)
    }

    pub fn new(config: RealizationConfig) -> BmiResult<Self> {
        Ok(Self {
            config,
            forcings: NetCdfForcings::new("forcings"),
            vars: HashMap::new(),
            models: Vec::new(),
            total_steps: 0,
            location_id: String::new(),
            #[cfg(feature = "fortran")]
            fortran_middleware: None,
            outputs: HashMap::new(),
            final_outputs: Vec::new(),
            has_run: false,
            suppress_warnings: false,
            source_timesteps: HashMap::new(),
            simulation_span_seconds: 0.0,
        })
    }

    #[cfg(feature = "fortran")]
    pub fn set_fortran_middleware(&mut self, path: impl Into<String>) {
        self.fortran_middleware = Some(path.into());
    }

    pub fn initialize(&mut self, loc_id: &str) -> BmiResult<()> {
        self.location_id = loc_id.to_string();
        self.has_run = false;

        self.forcings.initialize(&self.config.global.forcing.path)?;
        self.forcings.preload_location(loc_id)?;

        let start = parse_datetime(&self.config.time.start_time)?;
        let end = parse_datetime(&self.config.time.end_time)?;
        let span = (end - start) as f64;
        self.simulation_span_seconds = span;
        self.total_steps = ((end - start) / self.config.time.output_interval) as usize;

        // Register forcing variables with their timestep info
        let forcing_dt = self
            .forcings
            .time_step()
            .unwrap_or(self.config.time.output_interval as f64);
        let forcing_steps = if forcing_dt > 0.0 {
            (span / forcing_dt) as usize
        } else {
            self.total_steps
        };
        let forcing_ts = TimestepInfo {
            dt_seconds: forcing_dt,
            num_steps: forcing_steps,
        };

        self.vars.clear();
        self.source_timesteps.clear();
        for name in self.forcings.var_names()? {
            self.source_timesteps
                .insert(name.clone(), forcing_ts.clone());
            self.vars.insert(name, VarSource::Forcing);
        }

        self.load_models(loc_id)?;
        Ok(())
    }

    fn load_models(&mut self, loc_id: &str) -> BmiResult<()> {
        let modules: Vec<ModuleConfig> = self.config.modules().into_iter().cloned().collect();
        let mut pending: Vec<(ModuleConfig, Vec<String>)> = modules
            .into_iter()
            .map(|m| {
                let deps: Vec<String> = m.params.variables_names_map.values().cloned().collect();
                (m, deps)
            })
            .collect();

        let mut idx = 0;
        let max_iter = pending.len() * 2;
        let mut iter = 0;

        while !pending.is_empty() && iter < max_iter {
            iter += 1;

            let resolved = pending
                .iter()
                .position(|(_, deps)| deps.iter().all(|d| self.vars.contains_key(d)));

            if let Some(i) = resolved {
                let (module, _) = pending.remove(i);
                self.load_model(&module, loc_id, idx)?;
                idx += 1;
            } else {
                let missing: Vec<_> = pending
                    .iter()
                    .flat_map(|(_, deps)| deps.iter())
                    .filter(|d| !self.vars.contains_key(*d))
                    .cloned()
                    .collect();
                return Err(function_failed("runner", format!("Missing dependencies: {:?}", missing)));
            }
        }
        Ok(())
    }

    fn load_model(&mut self, module: &ModuleConfig, loc_id: &str, idx: usize) -> BmiResult<()> {
        let is_sloth =
            module.name == "bmi_c++" && module.params.model_type_name.to_uppercase() == "SLOTH";

        let mut model: Box<dyn Bmi> = if is_sloth {
            let mut sloth = BmiSloth::new(&module.params.model_type_name);
            sloth.configure(&module.params.params_string())?;
            Box::new(sloth)
        } else {
            let adapter = BmiAdapterType::from_name(&module.name).ok_or_else(|| {
                function_failed(&module.params.model_type_name, format!("Unknown adapter: {}", module.name))
            })?;

            match adapter {
                BmiAdapterType::C => {
                    let reg = if module.params.registration_function.is_empty() {
                        "register_bmi"
                    } else {
                        &module.params.registration_function
                    };
                    Box::new(BmiC::load(
                        &module.params.model_type_name,
                        &module.params.library_file,
                        reg,
                    )?)
                }
                #[cfg(feature = "fortran")]
                BmiAdapterType::Fortran => {
                    if let Some(ref mw) = self.fortran_middleware {
                        Box::new(BmiFortran::load(
                            &module.params.model_type_name,
                            &module.params.library_file,
                            mw,
                            "register_bmi",
                        )?)
                    } else {
                        Box::new(BmiFortran::load_single(
                            &module.params.model_type_name,
                            &module.params.library_file,
                            "register_bmi",
                        )?)
                    }
                }
                #[cfg(feature = "python")]
                BmiAdapterType::Python => {
                    if !module.params.python_type.is_empty() {
                        Box::new(BmiPython::load_from_type(
                            &module.params.model_type_name,
                            &module.params.python_type,
                        )?)
                    } else if !module.params.library_file.is_empty()
                        && !module.params.registration_function.is_empty()
                    {
                        Box::new(BmiPython::load(
                            &module.params.model_type_name,
                            &module.params.library_file,
                            &module.params.registration_function,
                        )?)
                    } else {
                        return Err(function_failed(
                            &module.params.model_type_name,
                            "bmi_python requires either 'python_type' (e.g. \"lstm.bmi_lstm.bmi_LSTM\") \
                             or both 'library_file' and 'registration_function'",
                        ));
                    }
                }
            }
        };

        if is_sloth {
            model.initialize("/dev/null")?;
        } else {
            model.initialize(&module.params.init_config(loc_id))?;
            for (name, val) in module.params.params_f64() {
                let _ = model.set_value(&name, &[val]);
            }
        }

        // Query model timestep
        let model_dt_seconds = match (model.get_time_step(), model.get_time_units()) {
            (Ok(dt), Ok(units)) => {
                let factor = crate::traits::parse_time_units(&units);
                let dt_sec = dt * factor;
                if dt_sec > 0.0 {
                    dt_sec
                } else {
                    self.config.time.output_interval as f64
                }
            }
            _ => {
                if !self.suppress_warnings {
                    eprintln!(
                        "  WARNING [{}]: could not query timestep, assuming output_interval ({}s)",
                        module.params.model_type_name, self.config.time.output_interval
                    );
                }
                self.config.time.output_interval as f64
            }
        };
        let model_steps = (self.simulation_span_seconds / model_dt_seconds) as usize;
        let timestep_info = TimestepInfo {
            dt_seconds: model_dt_seconds,
            num_steps: model_steps,
        };

        for output in model.get_output_var_names()? {
            self.source_timesteps
                .insert(output.clone(), timestep_info.clone());
            self.vars.insert(output.clone(), VarSource::Model(idx));
        }

        // Build unit conversions for each input mapping
        let mut input_conversions = HashMap::new();
        for (model_input, source_var) in &module.params.variables_names_map {
            let dest_units = model.get_var_units(model_input).unwrap_or_default();
            let source_units = self.get_source_units(source_var);

            if !source_units.is_empty() && !dest_units.is_empty() {
                let (conv, warning) =
                    crate::units::find_conversion_or_identity(&source_units, &dest_units);
                if let Some(warn) = warning {
                    if !self.suppress_warnings {
                        eprintln!(
                            "  WARNING [{}]: {} ← {}: {}",
                            module.params.model_type_name, model_input, source_var, warn
                        );
                    }
                }
                input_conversions.insert(model_input.clone(), conv);
            }
        }

        self.models.push(ModelInstance {
            name: module.params.model_type_name.clone(),
            model,
            input_map: module.params.variables_names_map.clone(),
            main_output: module.params.main_output_variable.clone(),
            input_conversions,
            timestep_info,
            downsample_mode: module.params.downsample_mode,
            upsample_mode: module.params.upsample_mode,
        });
        Ok(())
    }

    pub fn run(&mut self) -> BmiResult<()> {
        if self.has_run {
            return Err(function_failed("runner", "already run"));
        }

        for i in 0..self.models.len() {
            self.run_model(i)?;
        }

        // Resample all outputs to output_interval grid for CSV compatibility
        self.resample_outputs_to_interval()?;

        if let Some(main) = self.config.main_output() {
            if let Some(out) = self.outputs.get(main) {
                self.final_outputs = out.clone();
            }
        }

        self.has_run = true;
        Ok(())
    }

    fn resample_outputs_to_interval(&mut self) -> BmiResult<()> {
        let output_dt = self.config.time.output_interval as f64;
        let output_steps = self.total_steps;
        let mut resampled = HashMap::new();

        for (name, vals) in &self.outputs {
            let source_ts = match self.source_timesteps.get(name) {
                Some(ts) => ts,
                None => {
                    resampled.insert(name.clone(), vals.clone());
                    continue;
                }
            };

            if (source_ts.dt_seconds - output_dt).abs() < TIMESTEP_EPSILON {
                // Same timestep, no resampling needed
                resampled.insert(name.clone(), vals.clone());
                continue;
            }

            let mut out = Vec::with_capacity(output_steps + 1);
            for step in 0..output_steps + 1 {
                let t = step as f64 * output_dt;
                let v = resample::resample_value(
                    vals,
                    source_ts.dt_seconds,
                    t,
                    output_dt,
                    DownsampleMode::default(),
                    UpsampleMode::default(),
                )?;
                out.push(v);
            }
            resampled.insert(name.clone(), out);
        }

        self.outputs = resampled;
        Ok(())
    }

    fn run_model(&mut self, idx: usize) -> BmiResult<()> {
        let output_names: Vec<String> = self.models[idx].model.get_output_var_names()?;
        let model_ts = self.models[idx].timestep_info.clone();
        let model_steps = model_ts.num_steps;
        let downsample_mode = self.models[idx].downsample_mode;
        let upsample_mode = self.models[idx].upsample_mode;

        let mut outs: HashMap<String, Vec<f64>> = output_names
            .iter()
            .map(|n| (n.clone(), Vec::with_capacity(model_steps + 1)))
            .collect();

        let input_map = self.models[idx].input_map.clone();
        let conversions = self.models[idx].input_conversions.clone();

        for step in 0..model_steps + 1 {
            let dest_time = step as f64 * model_ts.dt_seconds;

            for (model_input, source) in &input_map {
                let val = self.get_var_resampled(
                    source,
                    dest_time,
                    model_ts.dt_seconds,
                    downsample_mode,
                    upsample_mode,
                )?;
                let val = if let Some(conv) = conversions.get(model_input) {
                    conv.convert(val)
                } else {
                    val
                };
                self.models[idx].model.set_value(model_input, &[val])?;
            }

            self.models[idx].model.update()?;

            for name in &output_names {
                if let Ok(v) = self.models[idx].model.get_scalar(name) {
                    outs.get_mut(name).unwrap().push(v);
                }
            }
        }

        for (name, vals) in outs {
            self.outputs.insert(name, vals);
        }
        Ok(())
    }

    fn get_var_resampled(
        &self,
        name: &str,
        dest_time: f64,
        dest_dt: f64,
        downsample_mode: DownsampleMode,
        upsample_mode: UpsampleMode,
    ) -> BmiResult<f64> {
        let source_ts = self
            .source_timesteps
            .get(name)
            .ok_or_else(|| function_failed("runner", format!("No timestep info for variable: {}", name)))?;
        let source_dt = source_ts.dt_seconds;

        match self.vars.get(name) {
            Some(VarSource::Forcing) => {
                // Fast path: same timestep
                if (source_dt - dest_dt).abs() < TIMESTEP_EPSILON {
                    let step = (dest_time / source_dt).round() as usize;
                    return self.forcings.get_f64(name, &self.location_id, step);
                }
                self.resample_forcing(
                    name,
                    source_dt,
                    dest_time,
                    dest_dt,
                    downsample_mode,
                    upsample_mode,
                )
            }
            Some(VarSource::Model(_)) => {
                let source_data = self
                    .outputs
                    .get(name)
                    .ok_or_else(|| function_failed("runner", format!("'{}' not yet computed", name)))?;
                // Fast path: same timestep
                if (source_dt - dest_dt).abs() < TIMESTEP_EPSILON {
                    let step = (dest_time / source_dt).round() as usize;
                    return source_data.get(step).copied().ok_or_else(|| {
                        function_failed("runner", format!("'{}' not available at step {}", name, step))
                    });
                }
                resample::resample_value(
                    source_data,
                    source_dt,
                    dest_time,
                    dest_dt,
                    downsample_mode,
                    upsample_mode,
                )
            }
            None => Err(function_failed("runner", format!("Unknown variable: {}", name))),
        }
    }

    fn resample_forcing(
        &self,
        name: &str,
        source_dt: f64,
        dest_time: f64,
        dest_dt: f64,
        downsample_mode: DownsampleMode,
        upsample_mode: UpsampleMode,
    ) -> BmiResult<f64> {
        let ratio = source_dt / dest_dt;

        if ratio > 1.0 {
            // Source coarser than dest (downsample)
            let fractional_idx = dest_time / source_dt;
            let lower_idx = fractional_idx.floor() as usize;
            match downsample_mode {
                DownsampleMode::Repeat => self.forcings.get_f64(name, &self.location_id, lower_idx),
                DownsampleMode::Interpolate => {
                    let lower_val = self.forcings.get_f64(name, &self.location_id, lower_idx)?;
                    let frac = fractional_idx - lower_idx as f64;
                    if frac.abs() < TIMESTEP_EPSILON {
                        return Ok(lower_val);
                    }
                    match self
                        .forcings
                        .get_f64(name, &self.location_id, lower_idx + 1)
                    {
                        Ok(upper_val) => Ok(lower_val + frac * (upper_val - lower_val)),
                        Err(_) => Ok(lower_val), // boundary: use last value
                    }
                }
            }
        } else {
            // Source finer than dest (upsample) — collect forcing values over window
            let start_idx = (dest_time / source_dt).floor() as usize;
            let end_idx = ((dest_time + dest_dt) / source_dt).ceil() as usize;
            let mut vals = Vec::with_capacity(end_idx - start_idx);
            for i in start_idx..end_idx {
                match self.forcings.get_f64(name, &self.location_id, i) {
                    Ok(v) => vals.push(v),
                    Err(_) => break, // stop at end of data
                }
            }
            if vals.is_empty() {
                return Err(function_failed("runner", format!("No forcing data for '{}' in window", name)));
            }
            resample::aggregate(&vals, upsample_mode)
        }
    }

    /// Get the units for a source variable (from forcings or a previous model's output).
    fn get_source_units(&self, name: &str) -> String {
        match self.vars.get(name) {
            Some(VarSource::Forcing) => self.forcings.var_units(name).unwrap_or_default(),
            Some(VarSource::Model(idx)) => {
                if let Some(m) = self.models.get(*idx) {
                    m.model.get_var_units(name).unwrap_or_default()
                } else {
                    String::new()
                }
            }
            None => String::new(),
        }
    }

    /// Print unit conversions to stderr.
    /// If `active_only` is true, only prints non-identity conversions.
    /// If false, prints all variable mappings including those without unit info.
    pub fn print_unit_conversions(&self, active_only: bool) {
        if !active_only {
            eprintln!("Unit conversions for this run:");
        }
        let mut any = false;
        for m in &self.models {
            for (model_input, source_var) in &m.input_map {
                let source_label = self.source_label(source_var);
                if let Some(conv) = m.input_conversions.get(model_input) {
                    if active_only && conv.is_identity() {
                        continue;
                    }
                    eprintln!(
                        "  {}: {} ← {} ({}): {}",
                        m.name, model_input, source_var, source_label, conv
                    );
                } else if !active_only {
                    eprintln!(
                        "  {}: {} ← {} ({}): no unit info available",
                        m.name, model_input, source_var, source_label
                    );
                } else {
                    continue;
                }
                any = true;
            }
        }
        if active_only && any {
            eprintln!();
        }
        if !active_only && !any {
            eprintln!("  (no variable mappings)");
        }
    }

    fn source_label(&self, source_var: &str) -> String {
        match self.vars.get(source_var) {
            Some(VarSource::Forcing) => "forcing".to_string(),
            Some(VarSource::Model(idx)) => self
                .models
                .get(*idx)
                .map(|src| src.name.clone())
                .unwrap_or_else(|| format!("model[{}]", idx)),
            None => "unknown".to_string(),
        }
    }

    /// Check each model's expected inputs against what's available.
    /// For unmapped inputs, check the alias table for available matches.
    /// Returns a list of suggested mappings to add to the realization config.
    pub fn find_missing_mappings(&self) -> Vec<SuggestedMapping> {
        let mut suggestions = Vec::new();

        for (model_idx, m) in self.models.iter().enumerate() {
            let input_names = match m.model.get_input_var_names() {
                Ok(names) => names,
                Err(_) => continue,
            };

            let mapped_inputs: std::collections::HashSet<&str> =
                m.input_map.keys().map(|s| s.as_str()).collect();

            for input_name in &input_names {
                // Skip if already mapped
                if mapped_inputs.contains(input_name.as_str()) {
                    continue;
                }

                // Skip if the variable is directly available (no mapping needed)
                if self.vars.contains_key(input_name) {
                    continue;
                }

                // Check aliases for this input name
                let alias_names = aliases::find_aliases(input_name);
                for alias in alias_names {
                    if self.vars.contains_key(alias) {
                        suggestions.push(SuggestedMapping {
                            model_name: m.name.clone(),
                            model_idx,
                            model_input: input_name.clone(),
                            suggested_source: alias.to_string(),
                        });
                        break; // first match is enough
                    }
                }
            }
        }

        suggestions
    }

    pub fn total_steps(&self) -> usize {
        self.total_steps
    }

    pub fn main_outputs(&self) -> BmiResult<&Vec<f64>> {
        if !self.has_run {
            return Err(function_failed("runner", "call run() first"));
        }
        Ok(&self.final_outputs)
    }

    pub fn outputs(&self, name: &str) -> BmiResult<&Vec<f64>> {
        if !self.has_run {
            return Err(function_failed("runner", "call run() first"));
        }
        self.outputs
            .get(name)
            .ok_or_else(|| function_failed("runner", format!("Output '{}' not found", name)))
    }

    pub fn finalize(&mut self) -> BmiResult<()> {
        for m in &mut self.models {
            m.model.finalize()?;
        }
        self.models.clear();
        self.vars.clear();
        self.outputs.clear();
        self.final_outputs.clear();
        self.source_timesteps.clear();
        self.has_run = false;
        Ok(())
    }

    pub fn close(&mut self) -> BmiResult<()> {
        self.finalize()?;
        self.forcings.finalize()?;
        Ok(())
    }

    pub fn forcings(&self) -> &NetCdfForcings {
        &self.forcings
    }
    pub fn model(&self, idx: usize) -> Option<&ModelInstance> {
        self.models.get(idx)
    }
    pub fn model_count(&self) -> usize {
        self.models.len()
    }
    pub fn location(&self) -> &str {
        &self.location_id
    }
}

impl Drop for ModelRunner {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
