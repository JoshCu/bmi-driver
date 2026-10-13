use std::collections::HashMap;
use std::path::Path;

use crate::adapters::BmiFortran;
#[cfg(feature = "python")]
use crate::adapters::BmiPython;
use crate::adapters::{BmiC, BmiSloth};
use crate::aliases;
use crate::config::{
    parse_datetime, BmiAdapterType, DownsampleMode, ModuleConfig, RealizationConfig, UpsampleMode,
};
use crate::error::{function_failed, BmiResult};

use crate::forcings::{Forcings, NetCdfForcings};
use crate::resample::{self, TIMESTEP_EPSILON};
use crate::traits::{Bmi, BmiExt};
use crate::units::UnitConversion;
use crate::variables::binding::VarBinding;

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
    /// Allow coupling through `get_value_ptr` (see [`ModuleParams::use_value_ptr`]).
    ///
    /// [`ModuleParams::use_value_ptr`]: crate::config::ModuleParams::use_value_ptr
    pub use_value_ptr: bool,
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
            fortran_middleware: None,
            outputs: HashMap::new(),
            final_outputs: Vec::new(),
            has_run: false,
            suppress_warnings: false,
            source_timesteps: HashMap::new(),
            simulation_span_seconds: 0.0,
        })
    }

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
                return Err(function_failed(
                    "runner",
                    format!("Missing dependencies: {:?}", missing),
                ));
            }
        }
        Ok(())
    }

    fn load_model(&mut self, module: &ModuleConfig, loc_id: &str, idx: usize) -> BmiResult<()> {
        let model = self.create_adapter(module, loc_id)?;
        let timestep_info = self.query_model_timestep(&*model, module);

        for output in model.get_output_var_names()? {
            self.source_timesteps
                .insert(output.clone(), timestep_info.clone());
            self.vars.insert(output.clone(), VarSource::Model(idx));
        }

        let input_conversions =
            self.build_unit_conversions(&*model, module, timestep_info.dt_seconds);

        self.models.push(ModelInstance {
            name: module.params.model_type_name.clone(),
            model,
            input_map: module.params.variables_names_map.clone(),
            main_output: module.params.main_output_variable.clone(),
            input_conversions,
            timestep_info,
            downsample_mode: module.params.downsample_mode,
            upsample_mode: module.params.upsample_mode,
            use_value_ptr: module.params.use_value_ptr,
        });
        Ok(())
    }

    /// Create and initialize the appropriate BMI adapter for a module config.
    fn create_adapter(&self, module: &ModuleConfig, loc_id: &str) -> BmiResult<Box<dyn Bmi>> {
        let is_sloth =
            module.name == "bmi_c++" && module.params.model_type_name.to_uppercase() == "SLOTH";

        let mut model: Box<dyn Bmi> = if is_sloth {
            let mut sloth = BmiSloth::new(&module.params.model_type_name);
            sloth.configure(&module.params.params_string())?;
            Box::new(sloth)
        } else {
            let adapter = BmiAdapterType::from_name(&module.name).ok_or_else(|| {
                function_failed(
                    &module.params.model_type_name,
                    format!("Unknown adapter: {}", module.name),
                )
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

        Ok(model)
    }

    /// Query a model's timestep and return TimestepInfo.
    fn query_model_timestep(&self, model: &dyn Bmi, module: &ModuleConfig) -> TimestepInfo {
        let model_dt_seconds = match (model.get_time_step(), model.get_time_units()) {
            (Ok(dt), Ok(units)) => {
                let dt_sec = dt * crate::traits::parse_time_units(&units);
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
        TimestepInfo {
            dt_seconds: model_dt_seconds,
            num_steps: (self.simulation_span_seconds / model_dt_seconds) as usize,
        }
    }

    /// Build unit conversions for each input variable mapping.
    /// `model_dt` is the model's timestep in seconds, used for rate↔accumulation conversions.
    fn build_unit_conversions(
        &self,
        model: &dyn Bmi,
        module: &ModuleConfig,
        model_dt: f64,
    ) -> HashMap<String, UnitConversion> {
        let mut conversions = HashMap::new();
        for (model_input, source_var) in &module.params.variables_names_map {
            let dest_units = model.get_var_units(model_input).unwrap_or_default();
            let source_units = self.get_source_units(source_var);

            if !source_units.is_empty() && !dest_units.is_empty() {
                let (conv, warning) = crate::units::find_conversion_or_identity(
                    &source_units,
                    &dest_units,
                    Some(model_dt),
                );
                if let Some(warn) = warning {
                    if !self.suppress_warnings {
                        eprintln!(
                            "  WARNING [{}]: {} ← {}: {}",
                            module.params.model_type_name, model_input, source_var, warn
                        );
                    }
                }
                conversions.insert(model_input.clone(), conv);
            }
        }
        conversions
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
        let mut resampled = HashMap::with_capacity(self.outputs.len());

        for (name, vals) in std::mem::take(&mut self.outputs) {
            let source_ts = match self.source_timesteps.get(&name) {
                Some(ts) if (ts.dt_seconds - output_dt).abs() >= TIMESTEP_EPSILON => ts,
                // No timestep info, or already at the output interval
                _ => {
                    resampled.insert(name, vals);
                    continue;
                }
            };

            let mut out = Vec::with_capacity(output_steps + 1);
            for step in 0..output_steps + 1 {
                let t = step as f64 * output_dt;
                let v = resample::resample_value(
                    &vals,
                    source_ts.dt_seconds,
                    t,
                    output_dt,
                    DownsampleMode::default(),
                    UpsampleMode::default(),
                )?;
                out.push(v);
            }
            resampled.insert(name, out);
        }

        self.outputs = resampled;
        Ok(())
    }

    fn run_model(&mut self, idx: usize) -> BmiResult<()> {
        // The loop reads sources from `self.forcings`/`self.outputs` while driving the model in
        // `self.models`, so borrow those fields separately.
        let ModelInstance {
            model,
            input_map,
            input_conversions,
            timestep_info: model_ts,
            downsample_mode,
            upsample_mode,
            use_value_ptr,
            ..
        } = &mut self.models[idx];
        let model = model.as_mut();
        let (downsample_mode, upsample_mode, use_ptr) =
            (*downsample_mode, *upsample_mode, *use_value_ptr);
        let model_steps = model_ts.num_steps;
        let output_names: Vec<String> = model.get_output_var_names()?;

        // Resolve every coupled variable once so the loop below does no lookups or allocation.
        let mut inputs = Vec::with_capacity(input_map.len());
        for (model_input, source) in input_map.iter() {
            let source = InputSource::resolve(
                source,
                &self.vars,
                &self.source_timesteps,
                &self.forcings,
                &self.outputs,
                &self.location_id,
            )?;
            let conv = input_conversions.get(model_input).cloned();
            let var = VarBinding::new(model, model_input, use_ptr)?;
            inputs.push((source, conv, var));
        }
        // Outputs that can't be bound (unknown type) are left empty, as failed reads always were.
        let mut outs: Vec<(String, Option<VarBinding>, Vec<f64>)> = output_names
            .into_iter()
            .map(|name| {
                let var = VarBinding::new(model, &name, use_ptr).ok();
                (name, var, Vec::with_capacity(model_steps + 1))
            })
            .collect();

        for step in 0..model_steps + 1 {
            let dest_time = step as f64 * model_ts.dt_seconds;

            for (source, conv, var) in &mut inputs {
                let val = source.value_at(
                    dest_time,
                    model_ts.dt_seconds,
                    downsample_mode,
                    upsample_mode,
                )?;
                let val = conv.as_ref().map_or(val, |c| c.convert(val));
                var.set(model, val)?;
            }

            model.update()?;

            let bound_outs = outs.iter_mut().filter_map(|(_, var, _)| var.as_mut());
            for var in inputs.iter_mut().map(|(_, _, var)| var).chain(bound_outs) {
                var.recheck_ptr(model);
            }

            for (_, var, vals) in &mut outs {
                if let Some(var) = var {
                    if let Ok(v) = var.get(model) {
                        vals.push(v);
                    }
                }
            }

            let bound_outs = outs.iter_mut().filter_map(|(_, var, _)| var.as_mut());
            for var in inputs.iter_mut().map(|(_, _, var)| var).chain(bound_outs) {
                var.promote();
            }
        }

        drop(inputs); // releases its borrows of `self.outputs`
        for (name, _, vals) in outs {
            self.outputs.insert(name, vals);
        }
        Ok(())
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

    pub fn source_label(&self, source_var: &str) -> String {
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

/// Where an input's values come from: a forcing series (stored as `f32`) or an upstream model's
/// outputs, which have all been computed by the time this model runs.
enum Series<'a> {
    Forcing(&'a [f32]),
    Model(&'a [f64]),
}

struct InputSource<'a> {
    name: &'a str,
    series: Series<'a>,
    source_dt: f64,
}

impl<'a> InputSource<'a> {
    /// Look the source up once, so each step reads a slice instead of resolving it by name.
    fn resolve(
        name: &'a str,
        vars: &HashMap<String, VarSource>,
        timesteps: &HashMap<String, TimestepInfo>,
        forcings: &'a NetCdfForcings,
        outputs: &'a HashMap<String, Vec<f64>>,
        location_id: &str,
    ) -> BmiResult<Self> {
        let source_dt = timesteps
            .get(name)
            .ok_or_else(|| {
                function_failed("runner", format!("No timestep info for variable: {}", name))
            })?
            .dt_seconds;
        let series = match vars.get(name) {
            Some(VarSource::Forcing) => Series::Forcing(forcings.cached_series(name, location_id)?),
            Some(VarSource::Model(_)) => Series::Model(outputs.get(name).ok_or_else(|| {
                function_failed("runner", format!("'{}' not yet computed", name))
            })?),
            None => {
                return Err(function_failed(
                    "runner",
                    format!("Unknown variable: {}", name),
                ))
            }
        };
        Ok(Self {
            name,
            series,
            source_dt,
        })
    }

    fn value_at(
        &self,
        dest_time: f64,
        dest_dt: f64,
        downsample_mode: DownsampleMode,
        upsample_mode: UpsampleMode,
    ) -> BmiResult<f64> {
        let source_dt = self.source_dt;
        match self.series {
            Series::Forcing(values) => resample::resample_forcing(
                self.name,
                values,
                source_dt,
                dest_time,
                dest_dt,
                downsample_mode,
                upsample_mode,
            ),
            Series::Model(values) => resample::resample_value(
                values,
                source_dt,
                dest_time,
                dest_dt,
                downsample_mode,
                upsample_mode,
            ),
        }
    }
}

impl Drop for ModelRunner {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
