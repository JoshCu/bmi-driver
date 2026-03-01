use std::collections::HashMap;
use std::path::Path;

use pyo3::prelude::*;

use crate::error::{BmiError, BmiResult};
use crate::traits::{parse_time_units, Bmi, VarType};
use super::check_initialized;

/// BMI adapter for Python classes implementing the BMI interface.
///
/// Two loading modes are supported:
///
/// 1. **File-based** (`load`): Set `library_file` to the path of a `.py` file and
///    `registration_function` to the class name. The file's parent directory is prepended
///    to `sys.path` so that the module can be imported by its stem name.
///
/// 2. **Dotted module path** (`load_from_type`): Set `python_type` to a dotted path like
///    `"lstm.bmi_lstm.bmi_LSTM"`. The last component is the class name and everything
///    before it is the module path. The package must be installed or already on `PYTHONPATH`.
///    This matches the ngen `python_type` realization convention.
///
/// The class is instantiated with no arguments; `initialize(config)` is called separately.
///
/// ## BMI functions called
///
/// Python method names match BMI function names exactly, with two notable exceptions for value
/// exchange (see below). All calls acquire the GIL via PyO3.
///
/// **Lifecycle** — called once per simulation:
/// - `initialize(config)` — passes the config path as a Python string.
/// - `finalize()` — called on drop if the model was successfully initialized.
///
/// **Per-timestep** — called inside the simulation loop:
/// - `update()` — advance the model by one timestep.
/// - `get_value_ptr(name)` followed by `.tolist()` — **not** `get_value`. Using `get_value_ptr`
///   returns a numpy array whose `.tolist()` converts elements to native Python types that PyO3
///   can extract as `Vec<T>` without requiring the numpy crate. The adapter does **not** call
///   `get_var_nbytes`; Python manages the buffer.
/// - `set_value(name, arr)` — the driver converts the Rust slice to a numpy array via
///   `numpy.array(values)` then calls `set_value` with that array.
///
/// **After `initialize`** (setup):
/// - `get_time_units()` — to compute the internal `time_factor`.
/// - `get_input_var_names()` / `get_output_var_names()` → `get_var_type(name)` +
///   `get_var_itemsize(name)` (per variable) — type cache. Note: `get_input_item_count` and
///   `get_output_item_count` are NOT called here; Python returns the full list directly.
///
/// **On demand**:
/// - `update_until(time)`, `get_component_name()`, `get_input_item_count()`,
///   `get_output_item_count()`, `get_var_grid(name)`, `get_var_units(name)`,
///   `get_var_nbytes(name)`, `get_var_location(name)`, `get_current_time()`, `get_start_time()`,
///   `get_end_time()`, `get_time_step()`, `get_grid_rank(grid)`, `get_grid_size(grid)`,
///   `get_grid_type(grid)`.
///
/// ## Functions NOT called
///
/// `get_value` (plain, non-ptr variant), `get_value_at_indices`, `set_value_at_indices`, and all
/// extended grid functions are never invoked by this adapter.
pub struct BmiPython {
    model_name: String,
    obj: PyObject,
    initialized: bool,
    time_factor: f64,
    type_cache: Option<HashMap<String, VarType>>,
}

impl BmiPython {
    /// Import a Python BMI class from a dotted `python_type` path.
    ///
    /// `python_type` should be a dotted string like `"lstm.bmi_lstm.bmi_LSTM"` where the
    /// last component is the class name and the preceding components form the module path.
    /// The package must already be importable (installed or on `PYTHONPATH`).
    pub fn load_from_type(
        name: impl Into<String>,
        python_type: &str,
    ) -> BmiResult<Self> {
        let name = name.into();

        let dot_pos = python_type.rfind('.').ok_or_else(|| BmiError::FunctionFailed {
            model: name.clone(),
            func: format!(
                "python_type '{}' must be a dotted path like 'package.module.ClassName'",
                python_type
            ),
        })?;
        let module_path = &python_type[..dot_pos];
        let class_name = &python_type[dot_pos + 1..];

        let obj = Python::with_gil(|py| -> PyResult<PyObject> {
            let instance = py.import_bound(module_path)?.getattr(class_name)?.call0()?;
            Ok(instance.unbind())
        })
        .map_err(|e| BmiError::FunctionFailed {
            model: name.clone(),
            func: format!("load_from_type: {e}"),
        })?;

        Ok(Self { model_name: name, obj, initialized: false, time_factor: 1.0, type_cache: None })
    }

    /// Import a `.py` file as a Python module and instantiate `class_name`.
    ///
    /// The file's parent directory is prepended to `sys.path` so the module can be imported.
    /// Use this when `library_file` and `registration_function` are set in the realization config.
    pub fn load(
        name: impl Into<String>,
        module_path: impl AsRef<Path>,
        class_name: &str,
    ) -> BmiResult<Self> {
        let name = name.into();
        let path = module_path.as_ref();

        let obj = Python::with_gil(|py| -> PyResult<PyObject> {
            // Prepend the module's parent directory to sys.path.
            let dir = path.parent().unwrap_or(Path::new("."));
            let sys = py.import_bound("sys")?;
            sys.getattr("path")?
                .call_method1("insert", (0, dir.to_str().unwrap_or(".")))?;

            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("model");
            let instance = py.import_bound(stem)?.getattr(class_name)?.call0()?;
            Ok(instance.unbind())
        })
        .map_err(|e| BmiError::FunctionFailed {
            model: name.clone(),
            func: format!("load: {e}"),
        })?;

        Ok(Self { model_name: name, obj, initialized: false, time_factor: 1.0, type_cache: None })
    }

    fn py_err(&self, func: &str, e: PyErr) -> BmiError {
        BmiError::FunctionFailed { model: self.model_name.clone(), func: format!("{func}: {e}") }
    }

    fn call0_str(&self, py: Python<'_>, method: &str) -> BmiResult<String> {
        self.obj.call_method0(py, method)
            .and_then(|r| r.extract::<String>(py))
            .map_err(|e| self.py_err(method, e))
    }

    fn call0_i32(&self, py: Python<'_>, method: &str) -> BmiResult<i32> {
        self.obj.call_method0(py, method)
            .and_then(|r| r.extract::<i32>(py))
            .map_err(|e| self.py_err(method, e))
    }

    fn call0_f64(&self, py: Python<'_>, method: &str) -> BmiResult<f64> {
        self.obj.call_method0(py, method)
            .and_then(|r| r.extract::<f64>(py))
            .map_err(|e| self.py_err(method, e))
    }

    fn call1_str(&self, py: Python<'_>, method: &str, arg: &str) -> BmiResult<String> {
        self.obj.call_method1(py, method, (arg,))
            .and_then(|r| r.extract::<String>(py))
            .map_err(|e| self.py_err(method, e))
    }

    fn call1_i32_str(&self, py: Python<'_>, method: &str, arg: &str) -> BmiResult<i32> {
        self.obj.call_method1(py, method, (arg,))
            .and_then(|r| r.extract::<i32>(py))
            .map_err(|e| self.py_err(method, e))
    }

    fn call1_i32_grid(&self, py: Python<'_>, method: &str, grid: i32) -> BmiResult<i32> {
        self.obj.call_method1(py, method, (grid,))
            .and_then(|r| r.extract::<i32>(py))
            .map_err(|e| self.py_err(method, e))
    }

    fn call1_str_grid(&self, py: Python<'_>, method: &str, grid: i32) -> BmiResult<String> {
        self.obj.call_method1(py, method, (grid,))
            .and_then(|r| r.extract::<String>(py))
            .map_err(|e| self.py_err(method, e))
    }

    /// Call `get_value_ptr(name)` on the Python object and return its `.tolist()` result.
    /// Using `tolist()` converts any numpy array dtype to native Python types, which pyo3
    /// can then extract as `Vec<T>` without needing the numpy crate.
    fn get_value_ptr_list<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
        let arr = self.obj.call_method1(py, "get_value_ptr", (name,))?;
        arr.bind(py).call_method0("tolist")
    }

    /// Create a numpy array from a Rust slice and pass it to `set_value(name, arr)`.
    fn set_value_via_numpy<T>(&self, py: Python<'_>, name: &str, values: Vec<T>) -> PyResult<()>
    where
        T: IntoPy<PyObject>,
        Vec<T>: IntoPy<PyObject>,
    {
        let np = py.import_bound("numpy")?;
        let arr = np.call_method1("array", (values,))?;
        self.obj.call_method1(py, "set_value", (name, arr))?;
        Ok(())
    }
}

impl Bmi for BmiPython {
    fn name(&self) -> &str { &self.model_name }
    fn is_initialized(&self) -> bool { self.initialized }
    fn var_type_cache(&self) -> Option<&HashMap<String, VarType>> { self.type_cache.as_ref() }
    fn var_type_cache_mut(&mut self) -> &mut Option<HashMap<String, VarType>> { &mut self.type_cache }
    fn time_factor(&self) -> f64 { self.time_factor }

    fn initialize(&mut self, config: &str) -> BmiResult<()> {
        if self.initialized {
            return Err(BmiError::AlreadyInitialized { model: self.model_name.clone() });
        }
        if !Path::new(config).exists() {
            return Err(BmiError::ConfigNotFound { path: config.into() });
        }
        Python::with_gil(|py| {
            self.obj.call_method1(py, "initialize", (config,))
                .map_err(|e| self.py_err("initialize", e))
        })?;
        self.initialized = true;
        self.time_factor = self.get_time_units().map(|u| parse_time_units(&u)).unwrap_or(1.0);
        self.cache_types()?;
        Ok(())
    }

    fn update(&mut self) -> BmiResult<()> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| {
            self.obj.call_method0(py, "update").map_err(|e| self.py_err("update", e))
        })?;
        Ok(())
    }

    fn update_until(&mut self, time: f64) -> BmiResult<()> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| {
            self.obj.call_method1(py, "update_until", (time,))
                .map_err(|e| self.py_err("update_until", e))
        })?;
        Ok(())
    }

    fn finalize(&mut self) -> BmiResult<()> {
        if !self.initialized { return Ok(()); }
        Python::with_gil(|py| {
            self.obj.call_method0(py, "finalize").map_err(|e| self.py_err("finalize", e))
        })?;
        self.initialized = false;
        Ok(())
    }

    fn get_component_name(&self) -> BmiResult<String> {
        Python::with_gil(|py| self.call0_str(py, "get_component_name"))
    }

    fn get_input_item_count(&self) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call0_i32(py, "get_input_item_count"))
    }

    fn get_output_item_count(&self) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call0_i32(py, "get_output_item_count"))
    }

    fn get_input_var_names(&self) -> BmiResult<Vec<String>> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| {
            self.obj.call_method0(py, "get_input_var_names")
                .and_then(|r| r.extract::<Vec<String>>(py))
                .map_err(|e| self.py_err("get_input_var_names", e))
        })
    }

    fn get_output_var_names(&self) -> BmiResult<Vec<String>> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| {
            self.obj.call_method0(py, "get_output_var_names")
                .and_then(|r| r.extract::<Vec<String>>(py))
                .map_err(|e| self.py_err("get_output_var_names", e))
        })
    }

    fn get_var_grid(&self, name: &str) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call1_i32_str(py, "get_var_grid", name))
    }

    fn get_var_type(&self, name: &str) -> BmiResult<String> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call1_str(py, "get_var_type", name))
    }

    fn get_var_units(&self, name: &str) -> BmiResult<String> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call1_str(py, "get_var_units", name))
    }

    fn get_var_itemsize(&self, name: &str) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call1_i32_str(py, "get_var_itemsize", name))
    }

    fn get_var_nbytes(&self, name: &str) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call1_i32_str(py, "get_var_nbytes", name))
    }

    fn get_var_location(&self, name: &str) -> BmiResult<String> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call1_str(py, "get_var_location", name))
    }

    fn get_current_time(&self) -> BmiResult<f64> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call0_f64(py, "get_current_time"))
    }

    fn get_start_time(&self) -> BmiResult<f64> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call0_f64(py, "get_start_time"))
    }

    fn get_end_time(&self) -> BmiResult<f64> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call0_f64(py, "get_end_time"))
    }

    fn get_time_units(&self) -> BmiResult<String> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call0_str(py, "get_time_units"))
    }

    fn get_time_step(&self) -> BmiResult<f64> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call0_f64(py, "get_time_step"))
    }

    fn get_value_f64(&self, name: &str) -> BmiResult<Vec<f64>> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| {
            self.get_value_ptr_list(py, name)
                .and_then(|l| l.extract::<Vec<f64>>())
                .map_err(|e| self.py_err("get_value_ptr", e))
        })
    }

    fn get_value_f32(&self, name: &str) -> BmiResult<Vec<f32>> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| {
            self.get_value_ptr_list(py, name)
                .and_then(|l| l.extract::<Vec<f32>>())
                .map_err(|e| self.py_err("get_value_ptr", e))
        })
    }

    fn get_value_i32(&self, name: &str) -> BmiResult<Vec<i32>> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| {
            self.get_value_ptr_list(py, name)
                .and_then(|l| l.extract::<Vec<i32>>())
                .map_err(|e| self.py_err("get_value_ptr", e))
        })
    }

    fn set_value_f64(&mut self, name: &str, values: &[f64]) -> BmiResult<()> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.set_value_via_numpy(py, name, values.to_vec()))
            .map_err(|e| self.py_err("set_value", e))
    }

    fn set_value_f32(&mut self, name: &str, values: &[f32]) -> BmiResult<()> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.set_value_via_numpy(py, name, values.to_vec()))
            .map_err(|e| self.py_err("set_value", e))
    }

    fn set_value_i32(&mut self, name: &str, values: &[i32]) -> BmiResult<()> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.set_value_via_numpy(py, name, values.to_vec()))
            .map_err(|e| self.py_err("set_value", e))
    }

    fn get_grid_rank(&self, grid: i32) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call1_i32_grid(py, "get_grid_rank", grid))
    }

    fn get_grid_size(&self, grid: i32) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call1_i32_grid(py, "get_grid_size", grid))
    }

    fn get_grid_type(&self, grid: i32) -> BmiResult<String> {
        check_initialized(self.initialized, &self.model_name)?;
        Python::with_gil(|py| self.call1_str_grid(py, "get_grid_type", grid))
    }
}

impl Drop for BmiPython {
    fn drop(&mut self) {
        if self.initialized { let _ = self.finalize(); }
    }
}
