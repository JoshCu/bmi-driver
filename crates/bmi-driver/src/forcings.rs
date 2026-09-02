//! Reading NetCDF forcing data.
//!
//! ## Why the file handle is shared
//!
//! libnetcdf's HDF5 backend shares per-file state between two `ncid`s opened on the same path,
//! and tears that state down when the first of them closes -- so two independent handles to one
//! forcing file will segfault the process, whether or not the calls themselves are serialized.
//! [`NetCdfForcings`] therefore does not own a handle. It looks the path up in a process-wide
//! registry, which opens each file exactly once and keeps it open, and reads through a mutex.
//!
//! Each `NetCdfForcings` keeps its own copy of the file's metadata and its own cache of one
//! location's values, so everything except the bulk read of a new location is lock-free.

use bmi_core::error::{BmiError, BmiResult};
use bmi_core::traits::VarType;
use netcdf::{self, File as NetCdfFile};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

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
    itemsize: i32,
    type_str: String,
}

/// Everything about a forcing file that can be read once and then shared.
#[derive(Debug, Default)]
struct Meta {
    location_ids: Vec<String>,
    location_index: HashMap<String, usize>,
    var_names: Vec<String>,
    var_info: HashMap<String, VarInfo>,
    timestep_count: usize,
    time_step: f64,
    start_time: f64,
    end_time: f64,
}

/// One open forcing file, shared by every reader of that path in this process.
struct SharedFile {
    meta: Arc<Meta>,
    file: Mutex<NetCdfFile>,
}

fn registry() -> &'static Mutex<HashMap<PathBuf, Arc<SharedFile>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Arc<SharedFile>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn err_for(name: &str, msg: &str) -> BmiError {
    BmiError::FunctionFailed {
        model: name.to_string(),
        func: msg.into(),
    }
}

/// Open `path` once per process and hand out a shared handle.
fn open_shared(name: &str, path: &Path) -> BmiResult<Arc<SharedFile>> {
    let key = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());

    if let Some(existing) = reg.get(&key) {
        return Ok(Arc::clone(existing));
    }

    // HDF5 prints verbose diagnostics to stderr for internal probes that are not errors.
    crate::output::netcdf::suppress_hdf5_errors();
    let file = netcdf::open(&key).map_err(|e| err_for(name, &format!("Failed to open: {}", e)))?;
    let meta = Arc::new(load_metadata(name, &file)?);

    let shared = Arc::new(SharedFile {
        meta,
        file: Mutex::new(file),
    });
    reg.insert(key, Arc::clone(&shared));
    Ok(shared)
}

fn load_metadata(name: &str, file: &NetCdfFile) -> BmiResult<Meta> {
    let mut meta = Meta::default();

    let ids_var = file
        .variable("ids")
        .ok_or_else(|| err_for(name, "Missing 'ids' variable"))?;
    for i in 0..ids_var.len() {
        let id = ids_var
            .get_string(i)
            .map_err(|e| err_for(name, &format!("Failed to get id {}: {}", i, e)))?;
        meta.location_index.insert(id.clone(), i);
        meta.location_ids.push(id);
    }

    meta.timestep_count = file
        .dimension("time")
        .map(|d| d.len())
        .ok_or_else(|| err_for(name, "Missing 'time' dimension"))?;

    load_time_info(name, file, &mut meta)?;
    discover_vars(file, &mut meta);
    Ok(meta)
}

fn load_time_info(name: &str, file: &NetCdfFile, meta: &mut Meta) -> BmiResult<()> {
    if let Some(time_var) = file.variable("Time") {
        let dims = time_var.dimensions();
        let times: Vec<i64> = if dims.len() == 2 {
            time_var
                .get_values((0usize, ..))
                .map_err(|e| err_for(name, &format!("Time read error: {}", e)))?
        } else {
            time_var
                .get_values(..)
                .map_err(|e| err_for(name, &format!("Time read error: {}", e)))?
        };

        if !times.is_empty() {
            meta.start_time = times[0] as f64;
            meta.end_time = *times.last().unwrap() as f64;
            if times.len() > 1 {
                meta.time_step = (times[1] - times[0]) as f64;
            }
        }
    } else {
        meta.start_time = 0.0;
        meta.time_step = 3600.0;
        meta.end_time = meta.start_time + (meta.timestep_count as f64 - 1.0) * meta.time_step;
    }
    Ok(())
}

fn discover_vars(file: &NetCdfFile, meta: &mut Meta) {
    let exclude = ["ids", "Time"];

    for var in file.variables() {
        let name = var.name();
        if exclude.contains(&name.as_str()) {
            continue;
        }

        let dims: Vec<String> = var.dimensions().iter().map(|d| d.name()).collect();
        if !dims.contains(&"catchment-id".to_string()) || !dims.contains(&"time".to_string()) {
            continue;
        }

        let units = var
            .attribute("units")
            .and_then(|a| a.value().ok())
            .map(|v| match v {
                netcdf::AttributeValue::Str(s) => s,
                _ => String::new(),
            })
            .unwrap_or_default();

        let (type_str, itemsize, _var_type) = type_info(&var);
        meta.var_names.push(name.clone());
        meta.var_info.insert(
            name,
            VarInfo {
                units,
                itemsize,
                type_str,
            },
        );
    }
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

pub struct NetCdfForcings {
    name: String,
    shared: Option<Arc<SharedFile>>,
    meta: Arc<Meta>,
    path: Option<String>,
    cached_loc: Option<String>,
    cached_data: HashMap<String, Vec<f32>>,
}

impl NetCdfForcings {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            shared: None,
            meta: Arc::new(Meta::default()),
            path: None,
            cached_loc: None,
            cached_data: HashMap::new(),
        }
    }

    /// Read every variable for one location into this instance's cache. Subsequent
    /// `get_f32`/`get_f64` calls for that location touch no shared state at all.
    pub fn preload_location(&mut self, loc_id: &str) -> BmiResult<()> {
        if self.cached_loc.as_deref() == Some(loc_id) {
            return Ok(());
        }

        let loc_idx = self.loc_idx(loc_id)?;
        let shared = self
            .shared
            .as_ref()
            .ok_or_else(|| self.err("not initialized"))?;

        let file = shared.file.lock().unwrap_or_else(|e| e.into_inner());
        let mut data = HashMap::with_capacity(self.meta.var_names.len());
        for var_name in &self.meta.var_names {
            let var = file
                .variable(var_name)
                .ok_or_else(|| self.err(&format!("Variable '{}' not found", var_name)))?;
            let values: Vec<f32> = var
                .get_values((loc_idx, ..))
                .map_err(|e| self.err(&format!("Failed to read '{}': {}", var_name, e)))?;
            data.insert(var_name.clone(), values);
        }
        drop(file);

        self.cached_data = data;
        self.cached_loc = Some(loc_id.to_string());
        Ok(())
    }

    /// The whole time series for one variable at one location, as f64.
    ///
    /// Loads and caches the location's data if it is not already cached, so calling this for
    /// several variables at the same location reads the file once.
    pub fn series(&mut self, name: &str, loc_id: &str) -> BmiResult<Vec<f64>> {
        self.preload_location(loc_id)?;
        self.cached_data
            .get(name)
            .map(|v| v.iter().map(|&x| x as f64).collect())
            .ok_or_else(|| self.err(&format!("Variable '{}' not found", name)))
    }

    fn err(&self, msg: &str) -> BmiError {
        err_for(&self.name, msg)
    }

    fn loc_idx(&self, id: &str) -> BmiResult<usize> {
        self.meta
            .location_index
            .get(id)
            .copied()
            .ok_or_else(|| self.err(&format!("Unknown location: '{}'", id)))
    }
}

impl Forcings for NetCdfForcings {
    fn name(&self) -> &str {
        &self.name
    }

    fn is_initialized(&self) -> bool {
        self.shared.is_some()
    }

    fn initialize(&mut self, path: &str) -> BmiResult<()> {
        if self.is_initialized() && self.path.as_deref() == Some(path) {
            return Ok(());
        }
        let p = Path::new(path);
        if !p.exists() {
            return Err(BmiError::ConfigNotFound { path: path.into() });
        }

        let shared = open_shared(&self.name, p)?;
        self.meta = Arc::clone(&shared.meta);
        self.shared = Some(shared);
        self.path = Some(path.to_string());
        self.cached_loc = None;
        self.cached_data.clear();
        Ok(())
    }

    /// Release this reader. The underlying file stays open in the process-wide registry, because
    /// closing it while another reader holds a handle to the same path is what crashes HDF5.
    fn finalize(&mut self) -> BmiResult<()> {
        self.shared = None;
        self.meta = Arc::new(Meta::default());
        self.path = None;
        self.cached_loc = None;
        self.cached_data.clear();
        Ok(())
    }

    fn var_names(&self) -> BmiResult<Vec<String>> {
        Ok(self.meta.var_names.clone())
    }

    fn var_type(&self, name: &str) -> BmiResult<String> {
        self.meta
            .var_info
            .get(name)
            .map(|i| i.type_str.clone())
            .ok_or_else(|| self.err(&format!("Unknown var: {}", name)))
    }

    fn var_units(&self, name: &str) -> BmiResult<String> {
        self.meta
            .var_info
            .get(name)
            .map(|i| i.units.clone())
            .ok_or_else(|| self.err(&format!("Unknown var: {}", name)))
    }

    fn var_itemsize(&self, name: &str) -> BmiResult<i32> {
        self.meta
            .var_info
            .get(name)
            .map(|i| i.itemsize)
            .ok_or_else(|| self.err(&format!("Unknown var: {}", name)))
    }

    fn start_time(&self) -> BmiResult<f64> {
        Ok(self.meta.start_time)
    }
    fn end_time(&self) -> BmiResult<f64> {
        Ok(self.meta.end_time)
    }
    fn time_step(&self) -> BmiResult<f64> {
        Ok(self.meta.time_step)
    }
    fn timestep_count(&self) -> BmiResult<usize> {
        Ok(self.meta.timestep_count)
    }

    fn location_ids(&self) -> BmiResult<Vec<String>> {
        Ok(self.meta.location_ids.clone())
    }

    fn location_index(&self, id: &str) -> BmiResult<usize> {
        self.loc_idx(id)
    }

    fn get_f32(&self, name: &str, loc: &str, step: usize) -> BmiResult<f32> {
        let data = self
            .cached_loc
            .as_ref()
            .filter(|l| *l == loc)
            .and_then(|_| self.cached_data.get(name))
            .ok_or_else(|| {
                // The runner always preloads before reading; anything else is a caller bug.
                self.err(&format!("'{}' at '{}' is not loaded", name, loc))
            })?;

        data.get(step)
            .copied()
            .ok_or_else(|| self.err("Index out of bounds"))
    }

    fn get_f64(&self, name: &str, loc: &str, step: usize) -> BmiResult<f64> {
        self.get_f32(name, loc, step).map(|v| v as f64)
    }
}
