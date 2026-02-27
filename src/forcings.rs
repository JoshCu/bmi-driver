use std::collections::HashMap;
use std::path::Path;
use netcdf::{self, File as NetCdfFile};
use crate::error::{BmiError, BmiResult};
use crate::traits::VarType;

pub trait Forcings {
    fn name(&self) -> &str;
    fn is_initialized(&self) -> bool;
    fn initialize(&mut self, path: &str) -> BmiResult<()>;
    fn finalize(&mut self) -> BmiResult<()>;

    fn var_names(&self) -> BmiResult<Vec<String>>;
    fn var_type(&self, name: &str) -> BmiResult<String>;
    fn var_units(&self, name: &str) -> BmiResult<String>;
    fn var_itemsize(&self, name: &str) -> BmiResult<i32>;

    fn start_time(&self) -> BmiResult<f64>;
    fn end_time(&self) -> BmiResult<f64>;
    fn time_step(&self) -> BmiResult<f64>;
    fn timestep_count(&self) -> BmiResult<usize>;

    fn location_ids(&self) -> BmiResult<Vec<String>>;
    fn location_index(&self, id: &str) -> BmiResult<usize>;

    fn get_f32(&self, name: &str, loc: &str, step: usize) -> BmiResult<f32>;
    fn get_f64(&self, name: &str, loc: &str, step: usize) -> BmiResult<f64>;
}

#[derive(Debug, Clone)]
struct VarInfo {
    units: String,
    var_type: VarType,
    itemsize: i32,
    type_str: String,
}

pub struct NetCdfForcings {
    name: String,
    file: Option<NetCdfFile>,
    initialized: bool,
    path: Option<String>,
    location_ids: Vec<String>,
    location_index: HashMap<String, usize>,
    var_names: Vec<String>,
    var_info: HashMap<String, VarInfo>,
    timestep_count: usize,
    time_step: f64,
    start_time: f64,
    end_time: f64,
    cached_loc: Option<String>,
    cached_data: HashMap<String, Vec<f32>>,
}

impl NetCdfForcings {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            file: None,
            initialized: false,
            path: None,
            location_ids: Vec::new(),
            location_index: HashMap::new(),
            var_names: Vec::new(),
            var_info: HashMap::new(),
            timestep_count: 0,
            time_step: 0.0,
            start_time: 0.0,
            end_time: 0.0,
            cached_loc: None,
            cached_data: HashMap::new(),
        }
    }

    pub fn preload_location(&mut self, loc_id: &str) -> BmiResult<()> {
        if self.cached_loc.as_deref() == Some(loc_id) { return Ok(()); }

        let loc_idx = self.loc_idx(loc_id)?;
        let file = self.file.as_ref().ok_or_else(|| self.err("not initialized"))?;

        self.cached_data.clear();
        for var_name in &self.var_names {
            let var = file.variable(var_name).ok_or_else(|| self.err(&format!("Variable '{}' not found", var_name)))?;
            let values: Vec<f32> = var.get_values((loc_idx, ..))
                .map_err(|e| self.err(&format!("Failed to read '{}': {}", var_name, e)))?;
            self.cached_data.insert(var_name.clone(), values);
        }

        self.cached_loc = Some(loc_id.to_string());
        Ok(())
    }

    fn err(&self, msg: &str) -> BmiError {
        BmiError::FunctionFailed { model: self.name.clone(), func: msg.into() }
    }

    fn loc_idx(&self, id: &str) -> BmiResult<usize> {
        self.location_index.get(id).copied()
            .ok_or_else(|| self.err(&format!("Unknown location: '{}'", id)))
    }

    fn load_metadata(&mut self) -> BmiResult<()> {
        let file = self.file.as_ref().ok_or_else(|| self.err("not initialized"))?;

        let ids_var = file.variable("ids").ok_or_else(|| self.err("Missing 'ids' variable"))?;
        self.location_ids.clear();
        self.location_index.clear();

        for i in 0..ids_var.len() {
            let id = ids_var.get_string(i).map_err(|e| self.err(&format!("Failed to get id {}: {}", i, e)))?;
            self.location_index.insert(id.clone(), i);
            self.location_ids.push(id);
        }

        self.timestep_count = file.dimension("time").map(|d| d.len()).ok_or_else(|| self.err("Missing 'time' dimension"))?;

        self.load_time_info()?;
        self.discover_vars()?;
        Ok(())
    }

    fn load_time_info(&mut self) -> BmiResult<()> {
        let file = self.file.as_ref().ok_or_else(|| self.err("not initialized"))?;

        if let Some(time_var) = file.variable("Time") {
            let dims = time_var.dimensions();
            let times: Vec<i64> = if dims.len() == 2 {
                time_var.get_values((0usize, ..)).map_err(|e| self.err(&format!("Time read error: {}", e)))?
            } else {
                time_var.get_values(..).map_err(|e| self.err(&format!("Time read error: {}", e)))?
            };

            if !times.is_empty() {
                self.start_time = times[0] as f64;
                self.end_time = *times.last().unwrap() as f64;
                if times.len() > 1 { self.time_step = (times[1] - times[0]) as f64; }
            }
        } else {
            self.start_time = 0.0;
            self.time_step = 3600.0;
            self.end_time = self.start_time + (self.timestep_count as f64 - 1.0) * self.time_step;
        }
        Ok(())
    }

    fn discover_vars(&mut self) -> BmiResult<()> {
        let file = self.file.as_ref().ok_or_else(|| self.err("not initialized"))?;
        let exclude = ["ids", "Time"];

        self.var_names.clear();
        self.var_info.clear();

        for var in file.variables() {
            let name = var.name();
            if exclude.contains(&name.as_str()) { continue; }

            let dims: Vec<String> = var.dimensions().iter().map(|d| d.name()).collect();
            if !dims.contains(&"catchment-id".to_string()) || !dims.contains(&"time".to_string()) { continue; }

            let units = var.attribute("units")
                .and_then(|a| a.value().ok())
                .map(|v| match v { netcdf::AttributeValue::Str(s) => s, _ => String::new() })
                .unwrap_or_default();

            let (type_str, itemsize, var_type) = Self::type_info(&var);
            self.var_names.push(name.clone());
            self.var_info.insert(name, VarInfo { units, var_type, itemsize, type_str });
        }
        Ok(())
    }

    fn type_info(var: &netcdf::Variable) -> (String, i32, VarType) {
        use netcdf::types::{FloatType, IntType, NcVariableType};
        match var.vartype() {
            NcVariableType::Float(FloatType::F32) => ("float".into(), 4, VarType::Float),
            NcVariableType::Float(FloatType::F64) => ("double".into(), 8, VarType::Double),
            NcVariableType::Int(IntType::I32) => ("int".into(), 4, VarType::Int),
            _ => ("float".into(), 4, VarType::Float),
        }
    }
}

impl Forcings for NetCdfForcings {
    fn name(&self) -> &str { &self.name }
    fn is_initialized(&self) -> bool { self.initialized }

    fn initialize(&mut self, path: &str) -> BmiResult<()> {
        if self.initialized && self.path.as_deref() == Some(path) {
            return Ok(());
        }
        if !Path::new(path).exists() { return Err(BmiError::ConfigNotFound { path: path.into() }); }

        self.file = Some(netcdf::open(path).map_err(|e| self.err(&format!("Failed to open: {}", e)))?);
        self.path = Some(path.to_string());
        self.load_metadata()?;
        self.initialized = true;
        Ok(())
    }

    fn finalize(&mut self) -> BmiResult<()> {
        self.file = None;
        self.path = None;
        self.initialized = false;
        self.location_ids.clear();
        self.location_index.clear();
        self.var_names.clear();
        self.var_info.clear();
        self.cached_loc = None;
        self.cached_data.clear();
        Ok(())
    }

    fn var_names(&self) -> BmiResult<Vec<String>> { Ok(self.var_names.clone()) }

    fn var_type(&self, name: &str) -> BmiResult<String> {
        self.var_info.get(name).map(|i| i.type_str.clone()).ok_or_else(|| self.err(&format!("Unknown var: {}", name)))
    }

    fn var_units(&self, name: &str) -> BmiResult<String> {
        self.var_info.get(name).map(|i| i.units.clone()).ok_or_else(|| self.err(&format!("Unknown var: {}", name)))
    }

    fn var_itemsize(&self, name: &str) -> BmiResult<i32> {
        self.var_info.get(name).map(|i| i.itemsize).ok_or_else(|| self.err(&format!("Unknown var: {}", name)))
    }

    fn start_time(&self) -> BmiResult<f64> { Ok(self.start_time) }
    fn end_time(&self) -> BmiResult<f64> { Ok(self.end_time) }
    fn time_step(&self) -> BmiResult<f64> { Ok(self.time_step) }
    fn timestep_count(&self) -> BmiResult<usize> { Ok(self.timestep_count) }

    fn location_ids(&self) -> BmiResult<Vec<String>> { Ok(self.location_ids.clone()) }

    fn location_index(&self, id: &str) -> BmiResult<usize> { self.loc_idx(id) }

    fn get_f32(&self, name: &str, loc: &str, step: usize) -> BmiResult<f32> {
        if let Some(data) = self.cached_loc.as_ref().filter(|l| *l == loc).and_then(|_| self.cached_data.get(name)) {
            return data.get(step).copied().ok_or_else(|| self.err("Index out of bounds"));
        }

        let loc_idx = self.loc_idx(loc)?;
        let file = self.file.as_ref().ok_or_else(|| self.err("not initialized"))?;
        let var = file.variable(name).ok_or_else(|| self.err(&format!("Variable '{}' not found", name)))?;
        let vals: Vec<f32> = var.get_values((loc_idx, step..step + 1)).map_err(|e| self.err(&format!("Read error: {}", e)))?;
        vals.into_iter().next().ok_or_else(|| self.err("Empty result"))
    }

    fn get_f64(&self, name: &str, loc: &str, step: usize) -> BmiResult<f64> {
        self.get_f32(name, loc, step).map(|v| v as f64)
    }
}

impl Drop for NetCdfForcings {
    fn drop(&mut self) { let _ = self.finalize(); }
}
