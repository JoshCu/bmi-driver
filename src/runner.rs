use std::collections::HashMap;
use std::path::Path;

use crate::adapters::{BmiC, BmiFortran, BmiSloth};
use crate::config::{parse_datetime, BmiAdapterType, ModuleConfig, RealizationConfig};
use crate::error::{BmiError, BmiResult};
use crate::forcings::{Forcings, NetCdfForcings};
use crate::traits::{Bmi, BmiExt};

pub struct ModelInstance {
    pub name: String,
    pub model: Box<dyn Bmi>,
    pub input_map: HashMap<String, String>,
    pub main_output: String,
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

        self.vars.clear();
        for name in self.forcings.var_names()? {
            self.vars.insert(name, VarSource::Forcing);
        }

        let start = parse_datetime(&self.config.time.start_time)?;
        let end = parse_datetime(&self.config.time.end_time)?;
        self.total_steps = ((end - start) / self.config.time.output_interval) as usize;

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
                return Err(BmiError::FunctionFailed {
                    model: "runner".into(),
                    func: format!("Missing dependencies: {:?}", missing),
                });
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
                BmiError::FunctionFailed {
                    model: module.params.model_type_name.clone(),
                    func: format!("Unknown adapter: {}", module.name),
                }
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

        for output in model.get_output_var_names()? {
            self.vars.insert(output.clone(), VarSource::Model(idx));
        }

        self.models.push(ModelInstance {
            name: module.params.model_type_name.clone(),
            model,
            input_map: module.params.variables_names_map.clone(),
            main_output: module.params.main_output_variable.clone(),
        });
        Ok(())
    }

    pub fn run(&mut self) -> BmiResult<()> {
        if self.has_run {
            return Err(BmiError::FunctionFailed {
                model: "runner".into(),
                func: "already run".into(),
            });
        }

        for i in 0..self.models.len() {
            self.run_model(i)?;
        }

        if let Some(main) = self.config.main_output() {
            if let Some(out) = self.outputs.get(main) {
                self.final_outputs = out.clone();
            }
        }

        self.has_run = true;
        Ok(())
    }

    fn run_model(&mut self, idx: usize) -> BmiResult<()> {
        let output_names: Vec<String> = self.models[idx].model.get_output_var_names()?;
        let mut outs: HashMap<String, Vec<f64>> = output_names
            .iter()
            .map(|n| (n.clone(), Vec::with_capacity(self.total_steps)))
            .collect();

        let input_map = self.models[idx].input_map.clone();

        for step in 0..self.total_steps + 1 {
            for (model_input, source) in &input_map {
                let val = self.get_var(source, step)?;
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

    fn get_var(&self, name: &str, step: usize) -> BmiResult<f64> {
        match self.vars.get(name) {
            Some(VarSource::Forcing) => self.forcings.get_f64(name, &self.location_id, step),
            Some(VarSource::Model(_)) => self
                .outputs
                .get(name)
                .and_then(|v| v.get(step).copied())
                .ok_or_else(|| BmiError::FunctionFailed {
                    model: "runner".into(),
                    func: format!("'{}' not available at step {}", name, step),
                }),
            None => Err(BmiError::FunctionFailed {
                model: "runner".into(),
                func: format!("Unknown variable: {}", name),
            }),
        }
    }

    pub fn total_steps(&self) -> usize {
        self.total_steps
    }

    pub fn main_outputs(&self) -> BmiResult<&Vec<f64>> {
        if !self.has_run {
            return Err(BmiError::FunctionFailed {
                model: "runner".into(),
                func: "call run() first".into(),
            });
        }
        Ok(&self.final_outputs)
    }

    pub fn outputs(&self, name: &str) -> BmiResult<&Vec<f64>> {
        if !self.has_run {
            return Err(BmiError::FunctionFailed {
                model: "runner".into(),
                func: "call run() first".into(),
            });
        }
        self.outputs
            .get(name)
            .ok_or_else(|| BmiError::FunctionFailed {
                model: "runner".into(),
                func: format!("Output '{}' not found", name),
            })
    }

    pub fn finalize(&mut self) -> BmiResult<()> {
        for m in &mut self.models {
            m.model.finalize()?;
        }
        self.models.clear();
        self.vars.clear();
        self.outputs.clear();
        self.final_outputs.clear();
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
