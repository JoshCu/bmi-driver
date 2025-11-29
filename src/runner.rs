//! Model runner that executes BMI models based on configuration.

use std::collections::HashMap;
use std::path::Path;

use crate::adapter_sloth::BmiSloth;
use crate::config::{BmiAdapterType, ModuleConfig, RealizationConfig};
use crate::error::{BmiError, BmiResult};
use crate::forcings::Forcings;
use crate::forcings_netcdf::NetCdfForcings;
use crate::traits::{Bmi, BmiExt};
use crate::BmiC;
use crate::BmiFortran;

/// Path to the Fortran BMI middleware library.
/// Set this based on your environment.

/// A running model instance with its configuration.
pub struct ModelInstance {
    /// The model name/type
    pub name: String,
    /// The BMI model (boxed for dynamic dispatch)
    pub model: Box<dyn Bmi>,
    /// Mapping from model input variable to source variable name
    pub input_map: HashMap<String, String>,
    /// The main output variable this model produces
    pub main_output: String,
    /// All output variables this model can provide
    pub outputs: Vec<String>,
}

/// Manages the execution of multiple models for a location.
pub struct ModelRunner {
    /// Configuration
    config: RealizationConfig,
    /// Forcing data provider
    forcings: NetCdfForcings,
    /// Available variables and their sources (variable_name -> source)
    available_vars: HashMap<String, VarSource>,
    /// Model instances in dependency order
    models: Vec<ModelInstance>,
    /// Current timestep index
    current_step: usize,
    /// Total number of timesteps
    total_steps: usize,
    /// Current location ID
    location_id: String,
    /// Path to Fortran middleware
    fortran_middleware_path: Option<String>,
}

/// Source of a variable value.
#[derive(Debug, Clone)]
pub enum VarSource {
    /// From forcing data
    Forcing,
    /// From a model output (index into models vec)
    Model(usize),
}

impl ModelRunner {
    /// Create a new model runner from a configuration file.
    pub fn from_config(config_path: impl AsRef<Path>) -> BmiResult<Self> {
        let config = RealizationConfig::from_file(config_path)?;
        Self::new(config)
    }

    /// Create a new model runner from a parsed configuration.
    pub fn new(config: RealizationConfig) -> BmiResult<Self> {
        Ok(Self {
            config,
            forcings: NetCdfForcings::new("forcings"),
            available_vars: HashMap::new(),
            models: Vec::new(),
            current_step: 0,
            total_steps: 0,
            location_id: String::new(),
            fortran_middleware_path: None,
        })
    }

    /// Set the path to the Fortran BMI middleware library.
    pub fn set_fortran_middleware(&mut self, path: impl Into<String>) {
        self.fortran_middleware_path = Some(path.into());
    }

    /// Initialize the runner for a specific location.
    pub fn initialize(&mut self, location_id: &str) -> BmiResult<()> {
        self.location_id = location_id.to_string();

        // Initialize forcings
        self.forcings.initialize(&self.config.global.forcing.path)?;

        // Register forcing variables as available
        for var_name in self.forcings.get_output_var_names()? {
            self.available_vars.insert(var_name, VarSource::Forcing);
        }

        // Get total timesteps from config time range
        let start_epoch = crate::config::parse_datetime(&self.config.time.start_time)?;
        let end_epoch = crate::config::parse_datetime(&self.config.time.end_time)?;
        let interval = self.config.time.output_interval;
        self.total_steps = ((end_epoch - start_epoch) / interval) as usize;

        // Load and initialize models in dependency order
        self.load_models(location_id)?;

        Ok(())
    }

    /// Load all models and determine execution order based on dependencies.
    fn load_models(&mut self, location_id: &str) -> BmiResult<()> {
        let modules: Vec<ModuleConfig> = self.config.get_modules().into_iter().cloned().collect();

        // Build dependency graph and load models
        let mut pending: Vec<(ModuleConfig, Vec<String>)> = Vec::new();

        for module in modules {
            // Collect dependencies (input variables that need to be available)
            let deps: Vec<String> = module
                .params
                .variables_names_map
                .values()
                .cloned()
                .collect();
            pending.push((module, deps));
        }

        // Resolve dependencies in order
        let mut resolved_count = 0;
        let max_iterations = pending.len() * 2; // Prevent infinite loops
        let mut iterations = 0;

        while !pending.is_empty() && iterations < max_iterations {
            iterations += 1;

            // Find a module whose dependencies are all satisfied
            let mut resolved_idx = None;
            for (i, (_, deps)) in pending.iter().enumerate() {
                let all_satisfied = deps.iter().all(|dep| self.available_vars.contains_key(dep));
                if all_satisfied {
                    resolved_idx = Some(i);
                    break;
                }
            }

            if let Some(idx) = resolved_idx {
                let (module, _) = pending.remove(idx);
                self.load_single_model(&module, location_id, resolved_count)?;
                resolved_count += 1;
            } else {
                // No progress - check for missing dependencies
                let missing: Vec<String> = pending
                    .iter()
                    .flat_map(|(_, deps)| deps.iter())
                    .filter(|dep| !self.available_vars.contains_key(*dep))
                    .cloned()
                    .collect();

                return Err(BmiError::BmiFunctionFailed {
                    model: "runner".to_string(),
                    func: format!("Circular dependency or missing variables: {:?}", missing),
                });
            }
        }

        Ok(())
    }

    /// Load and initialize a single model.
    fn load_single_model(
        &mut self,
        module: &ModuleConfig,
        location_id: &str,
        model_index: usize,
    ) -> BmiResult<()> {
        // Check for SLOTH model (bmi_c++ with model_type_name SLOTH)
        let is_sloth =
            module.name == "bmi_c++" && module.params.model_type_name.to_uppercase() == "SLOTH";

        let mut model: Box<dyn Bmi> = if is_sloth {
            // Create SLOTH model with configured parameters
            let mut sloth = BmiSloth::new(&module.params.model_type_name);

            // Get model_params as strings for SLOTH
            let string_params = module.params.get_model_params_string();
            sloth.configure(&string_params)?;

            Box::new(sloth)
        } else {
            let adapter_type = BmiAdapterType::from_name(&module.name).ok_or_else(|| {
                BmiError::BmiFunctionFailed {
                    model: module.params.model_type_name.clone(),
                    func: format!("Unknown adapter type: {}", module.name),
                }
            })?;

            let init_config = module.params.get_init_config(location_id);

            // Load the appropriate adapter
            match adapter_type {
                BmiAdapterType::C => {
                    let reg_func = if module.params.registration_function.is_empty() {
                        "register_bmi"
                    } else {
                        &module.params.registration_function
                    };
                    Box::new(BmiC::load(
                        &module.params.model_type_name,
                        &module.params.library_file,
                        reg_func,
                    )?)
                }
                BmiAdapterType::Fortran => {
                    if let Some(ref mw_path) = &self.fortran_middleware_path {
                        Box::new(BmiFortran::load(
                            &module.params.model_type_name,
                            &module.params.library_file,
                            mw_path,
                            "register_bmi",
                        )?)
                    } else {
                        Box::new(BmiFortran::load_single_library(
                            &module.params.model_type_name,
                            &module.params.library_file,
                            "register_bmi",
                        )?)
                    }
                }
            }
        };

        // Initialize the model
        if is_sloth {
            // SLOTH just needs any path, config is already done
            model.initialize("/dev/null")?;
        } else {
            let init_config = module.params.get_init_config(location_id);
            model.initialize(&init_config)?;

            // Set model parameters (non-SLOTH models)
            for (param_name, value) in module.params.get_model_params_f64() {
                if let Err(e) = model.set_value(&param_name, &[value]) {
                    eprintln!(
                        "Warning: Failed to set parameter '{}' on {}: {}",
                        param_name, module.params.model_type_name, e
                    );
                }
            }
        }

        // Register this model's outputs as available
        for output in model.get_output_var_names()? {
            self.available_vars
                .insert(output.clone(), VarSource::Model(model_index));
        }

        // Store the model instance
        self.models.push(ModelInstance {
            name: module.params.model_type_name.clone(),
            model,
            input_map: module.params.variables_names_map.clone(),
            main_output: module.params.main_output_variable.clone(),
            outputs: module.params.variables_names_map.keys().cloned().collect(),
        });

        Ok(())
    }

    /// Run a single timestep for all models.
    pub fn update(&mut self) -> BmiResult<()> {
        if self.current_step >= self.total_steps {
            return Err(BmiError::BmiFunctionFailed {
                model: "runner".to_string(),
                func: "No more timesteps".to_string(),
            });
        }

        // Collect values from forcings and previous models
        let mut current_values: HashMap<String, f64> = HashMap::new();

        // Run each model in order
        for model_idx in 0..self.models.len() {
            // Get input values for this model
            let input_map = self.models[model_idx].input_map.clone();

            for (model_input, source_var) in &input_map {
                let value = self.get_variable_value(&source_var, &current_values)?;

                // Set the value on the model
                self.models[model_idx]
                    .model
                    .set_value(model_input, &[value])?;
            }

            // Update the model
            self.models[model_idx].model.update()?;

            // Collect outputs from this model for use by subsequent models
            for output_name in self.models[model_idx].model.get_output_var_names()? {
                if let Ok(value) = self.models[model_idx].model.get_value_scalar(&output_name) {
                    current_values.insert(output_name, value);
                }
            }
        }

        self.current_step += 1;
        Ok(())
    }

    /// Get a variable value from either forcings or a model output.
    fn get_variable_value(
        &self,
        var_name: &str,
        current_values: &HashMap<String, f64>,
    ) -> BmiResult<f64> {
        // First check if we already have it from a model this timestep
        if let Some(&value) = current_values.get(var_name) {
            return Ok(value);
        }

        // Check the source
        match self.available_vars.get(var_name) {
            Some(VarSource::Forcing) => self
                .forcings
                .get_value_at_index_f32(var_name, &self.location_id, self.current_step)
                .map(|v| v as f64),
            Some(VarSource::Model(idx)) => self.models[*idx].model.get_value_scalar(var_name),
            None => Err(BmiError::BmiFunctionFailed {
                model: "runner".to_string(),
                func: format!("Variable '{}' not available", var_name),
            }),
        }
    }

    /// Get the current timestep index.
    pub fn current_step(&self) -> usize {
        self.current_step
    }

    /// Get the total number of timesteps.
    pub fn total_steps(&self) -> usize {
        self.total_steps
    }

    /// Check if there are more timesteps to run.
    pub fn has_more_steps(&self) -> bool {
        self.current_step < self.total_steps
    }

    /// Get the value of an output variable at the current timestep.
    pub fn get_output(&self, var_name: &str) -> BmiResult<f64> {
        // Find which model provides this variable
        for model in self.models.iter().rev() {
            if let Ok(value) = model.model.get_value_scalar(var_name) {
                return Ok(value);
            }
        }

        Err(BmiError::BmiFunctionFailed {
            model: "runner".to_string(),
            func: format!("Output variable '{}' not found", var_name),
        })
    }

    /// Get the main output variable name.
    pub fn get_main_output_name(&self) -> BmiResult<String> {
        let main_var =
            self.config
                .get_main_output_variable()
                .ok_or_else(|| BmiError::BmiFunctionFailed {
                    model: "runner".to_string(),
                    func: "No main output variable defined".to_string(),
                })?;

        Ok(main_var.to_string())
    }

    /// Get the main output variable value.
    pub fn get_main_output(&self) -> BmiResult<f64> {
        let main_var = self.get_main_output_name()?;
        self.get_output(&main_var)
    }

    /// Get all requested output variables.
    pub fn get_outputs(&self) -> BmiResult<HashMap<String, f64>> {
        let mut outputs = HashMap::new();

        for var_name in self.config.get_output_variables() {
            if let Ok(value) = self.get_output(var_name) {
                outputs.insert(var_name.to_string(), value);
            }
        }

        Ok(outputs)
    }

    /// Finalize all models and forcings.
    pub fn finalize(&mut self) -> BmiResult<()> {
        for model in &mut self.models {
            model.model.finalize()?;
        }
        self.forcings.finalize()?;
        self.models.clear();
        self.available_vars.clear();
        Ok(())
    }

    /// Get a reference to the forcings.
    pub fn forcings(&self) -> &NetCdfForcings {
        &self.forcings
    }

    /// Get a reference to a model by index.
    pub fn get_model(&self, index: usize) -> Option<&ModelInstance> {
        self.models.get(index)
    }

    /// Get the number of models.
    pub fn model_count(&self) -> usize {
        self.models.len()
    }

    /// Get the location ID.
    pub fn location_id(&self) -> &str {
        &self.location_id
    }
}

impl Drop for ModelRunner {
    fn drop(&mut self) {
        let _ = self.finalize();
    }
}
