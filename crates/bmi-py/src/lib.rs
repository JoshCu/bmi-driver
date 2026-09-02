//! Python bindings: one `Model` class for every BMI adapter.
//!
//! The point of this module is that a Python caller should not have to know whether a model is C,
//! Fortran or Python, nor which of `get_value_double` / `get_value_float` / `get_value_int` the
//! model expects. Pick the adapter once when you construct the model, optionally pin a dtype, and
//! from then on it is just `get_value` / `set_value`.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyType;

use bmi_core::error::BmiError;
use bmi_core::traits::{Bmi, BmiExt, VarType};

mod dtype;
#[cfg(feature = "driver")]
mod driver;

use dtype::Dtype;

fn to_py_err(e: BmiError) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

/// A loaded BMI model.
///
/// Construct one with the classmethod matching your model's language:
///
/// ```python
/// Model.c("libcfebmi.so", register="register_bmi_cfe")
/// Model.fortran("libsurfacebmi.so")
/// Model.python("lstm.bmi_lstm.bmi_LSTM")
/// Model.sloth({"land_surface_water__runoff_depth": "0.0"})
/// ```
#[pyclass(unsendable, module = "bmi_driver")]
pub struct Model {
    inner: Box<dyn Bmi>,
    dtype: Dtype,
}

impl Model {
    fn new(inner: Box<dyn Bmi>, dtype: Dtype) -> Self {
        Self { inner, dtype }
    }
}

#[pymethods]
impl Model {
    /// Load a C BMI model from a shared library.
    #[classmethod]
    #[pyo3(signature = (library, register = "register_bmi", name = None, dtype = "auto"))]
    fn c(
        _cls: &Bound<'_, PyType>,
        library: &str,
        register: &str,
        name: Option<&str>,
        dtype: &str,
    ) -> PyResult<Self> {
        let name = name.unwrap_or(library);
        let model = bmi_core::BmiC::load(name, library, register).map_err(to_py_err)?;
        Ok(Self::new(Box::new(model), Dtype::parse(dtype)?))
    }

    /// Load a Fortran BMI model from a shared library.
    ///
    /// `middleware` is the path to the iso_c_binding shim library, for models whose registration
    /// function lives in a separate object from the model itself. Omit it when the model library
    /// exports `register_bmi` directly.
    #[cfg(feature = "fortran")]
    #[classmethod]
    #[pyo3(signature = (library, register = "register_bmi", middleware = None, name = None, dtype = "auto"))]
    fn fortran(
        _cls: &Bound<'_, PyType>,
        library: &str,
        register: &str,
        middleware: Option<&str>,
        name: Option<&str>,
        dtype: &str,
    ) -> PyResult<Self> {
        let name = name.unwrap_or(library);
        let model = match middleware {
            Some(mw) => bmi_core::BmiFortran::load(name, library, mw, register),
            None => bmi_core::BmiFortran::load_single(name, library, register),
        }
        .map_err(to_py_err)?;
        Ok(Self::new(Box::new(model), Dtype::parse(dtype)?))
    }

    /// Load a Python BMI class by dotted path, e.g. `"lstm.bmi_lstm.bmi_LSTM"`.
    #[cfg(feature = "python-models")]
    #[classmethod]
    #[pyo3(signature = (python_type, name = None, dtype = "auto"))]
    fn python(
        _cls: &Bound<'_, PyType>,
        python_type: &str,
        name: Option<&str>,
        dtype: &str,
    ) -> PyResult<Self> {
        let name = name.unwrap_or(python_type);
        let model =
            bmi_core::BmiPython::load_from_type(name, python_type).map_err(to_py_err)?;
        Ok(Self::new(Box::new(model), Dtype::parse(dtype)?))
    }

    /// Load a Python BMI class from a `.py` file path plus the class name.
    #[cfg(feature = "python-models")]
    #[classmethod]
    #[pyo3(signature = (path, class_name, name = None, dtype = "auto"))]
    fn python_file(
        _cls: &Bound<'_, PyType>,
        path: &str,
        class_name: &str,
        name: Option<&str>,
        dtype: &str,
    ) -> PyResult<Self> {
        let name = name.unwrap_or(class_name);
        let model = bmi_core::BmiPython::load(name, path, class_name).map_err(to_py_err)?;
        Ok(Self::new(Box::new(model), Dtype::parse(dtype)?))
    }

    /// A SLOTH-style constant provider: every entry becomes an output variable holding a fixed
    /// value. Useful for standing in for a model you have not written yet.
    #[classmethod]
    #[pyo3(signature = (values, name = "SLOTH", dtype = "auto"))]
    fn sloth(
        _cls: &Bound<'_, PyType>,
        values: std::collections::HashMap<String, String>,
        name: &str,
        dtype: &str,
    ) -> PyResult<Self> {
        let mut model = bmi_core::BmiSloth::new(name);
        model.configure(&values).map_err(to_py_err)?;
        Ok(Self::new(Box::new(model), Dtype::parse(dtype)?))
    }

    // --- lifecycle ---

    /// Initialize the model from its config file. Also populates the variable type cache, so
    /// `get_value`/`set_value` know which typed BMI call each variable needs.
    fn initialize(&mut self, config: &str) -> PyResult<()> {
        self.inner.initialize(config).map_err(to_py_err)?;
        // Best effort: models that do not report types still work through the dtype fallback.
        let _ = self.inner.cache_types();
        Ok(())
    }

    fn update(&mut self) -> PyResult<()> {
        self.inner.update().map_err(to_py_err)
    }

    fn update_until(&mut self, time: f64) -> PyResult<()> {
        self.inner.update_until(time).map_err(to_py_err)
    }

    fn finalize(&mut self) -> PyResult<()> {
        self.inner.finalize().map_err(to_py_err)
    }

    // --- values ---

    /// Read a variable. Returns a numpy array whose dtype is the model's own type for that
    /// variable, or the dtype pinned at construction.
    fn get_value<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyAny>> {
        let value = self.inner.get_value(name).map_err(to_py_err)?;
        dtype::value_to_py(py, value, self.dtype)
    }

    /// Read a variable as a Python float. Fails if the variable is not a scalar.
    fn get_scalar(&self, name: &str) -> PyResult<f64> {
        self.inner.get_scalar(name).map_err(to_py_err)
    }

    /// Write a variable. Accepts a scalar, a list, or a numpy array; the values are converted to
    /// whichever type the model actually wants.
    fn set_value(&mut self, name: &str, values: &Bound<'_, PyAny>) -> PyResult<()> {
        let values = dtype::py_to_f64_vec(values)?;
        let target = self
            .dtype
            .resolve(self.inner.cached_type(name).unwrap_or(VarType::Double));
        match target {
            VarType::Float => {
                let v: Vec<f32> = values.iter().map(|&x| x as f32).collect();
                self.inner.set_value_f32(name, &v)
            }
            VarType::Int => {
                let v: Vec<i32> = values.iter().map(|&x| x as i32).collect();
                self.inner.set_value_i32(name, &v)
            }
            _ => self.inner.set_value_f64(name, &values),
        }
        .map_err(to_py_err)
    }

    // --- metadata ---

    fn get_component_name(&self) -> PyResult<String> {
        self.inner.get_component_name().map_err(to_py_err)
    }

    fn get_input_var_names(&self) -> PyResult<Vec<String>> {
        self.inner.get_input_var_names().map_err(to_py_err)
    }

    fn get_output_var_names(&self) -> PyResult<Vec<String>> {
        self.inner.get_output_var_names().map_err(to_py_err)
    }

    fn get_input_item_count(&self) -> PyResult<i32> {
        self.inner.get_input_item_count().map_err(to_py_err)
    }

    fn get_output_item_count(&self) -> PyResult<i32> {
        self.inner.get_output_item_count().map_err(to_py_err)
    }

    fn get_var_type(&self, name: &str) -> PyResult<String> {
        self.inner.get_var_type(name).map_err(to_py_err)
    }

    fn get_var_units(&self, name: &str) -> PyResult<String> {
        self.inner.get_var_units(name).map_err(to_py_err)
    }

    fn get_var_itemsize(&self, name: &str) -> PyResult<i32> {
        self.inner.get_var_itemsize(name).map_err(to_py_err)
    }

    fn get_var_nbytes(&self, name: &str) -> PyResult<i32> {
        self.inner.get_var_nbytes(name).map_err(to_py_err)
    }

    fn get_var_location(&self, name: &str) -> PyResult<String> {
        self.inner.get_var_location(name).map_err(to_py_err)
    }

    fn get_var_grid(&self, name: &str) -> PyResult<i32> {
        self.inner.get_var_grid(name).map_err(to_py_err)
    }

    /// The numpy dtype string this binding uses for a variable: `"float64"`, `"float32"` or
    /// `"int32"`. This is what saves you from choosing between the typed BMI calls.
    fn dtype_of(&self, name: &str) -> PyResult<String> {
        let vt = match self.inner.cached_type(name) {
            Some(vt) => vt,
            None => {
                let t = self.inner.get_var_type(name).map_err(to_py_err)?;
                let s = self.inner.get_var_itemsize(name).unwrap_or(8);
                VarType::from_bmi(&t, s)
            }
        };
        Ok(dtype::name_of(self.dtype.resolve(vt)))
    }

    // --- time ---

    fn get_current_time(&self) -> PyResult<f64> {
        self.inner.get_current_time().map_err(to_py_err)
    }

    fn get_start_time(&self) -> PyResult<f64> {
        self.inner.get_start_time().map_err(to_py_err)
    }

    fn get_end_time(&self) -> PyResult<f64> {
        self.inner.get_end_time().map_err(to_py_err)
    }

    fn get_time_step(&self) -> PyResult<f64> {
        self.inner.get_time_step().map_err(to_py_err)
    }

    fn get_time_units(&self) -> PyResult<String> {
        self.inner.get_time_units().map_err(to_py_err)
    }

    /// The model's timestep converted to seconds, whatever `get_time_units` reports.
    fn time_step_seconds(&self) -> PyResult<f64> {
        let dt = self.inner.get_time_step().map_err(to_py_err)?;
        Ok(self.inner.to_seconds(dt))
    }

    // --- grids ---

    fn get_grid_rank(&self, grid: i32) -> PyResult<i32> {
        self.inner.get_grid_rank(grid).map_err(to_py_err)
    }

    fn get_grid_size(&self, grid: i32) -> PyResult<i32> {
        self.inner.get_grid_size(grid).map_err(to_py_err)
    }

    fn get_grid_type(&self, grid: i32) -> PyResult<String> {
        self.inner.get_grid_type(grid).map_err(to_py_err)
    }

    // --- python conveniences ---

    #[getter]
    fn name(&self) -> &str {
        self.inner.name()
    }

    #[getter]
    fn initialized(&self) -> bool {
        self.inner.is_initialized()
    }

    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    #[pyo3(signature = (*_args))]
    fn __exit__(&mut self, _args: &Bound<'_, PyAny>) -> PyResult<bool> {
        if self.inner.is_initialized() {
            self.inner.finalize().map_err(to_py_err)?;
        }
        Ok(false)
    }

    fn __repr__(&self) -> String {
        format!(
            "<Model {:?} initialized={}>",
            self.inner.name(),
            self.inner.is_initialized()
        )
    }
}

/// Convert between physical units, e.g. `convert(1.0, "mm h-1", "m s-1")`.
///
/// Rate/accumulation conversions need the model timestep, which is what `timestep_seconds` is
/// for: converting `"mm"` to `"mm h-1"` is only defined once you know how long a step is.
#[pyfunction]
#[pyo3(signature = (value, from_units, to_units, timestep_seconds = None))]
fn convert(
    value: f64,
    from_units: &str,
    to_units: &str,
    timestep_seconds: Option<f64>,
) -> PyResult<f64> {
    let (conv, warning) =
        bmi_core::units::find_conversion_or_identity(from_units, to_units, timestep_seconds);
    if let Some(w) = warning {
        return Err(PyValueError::new_err(w));
    }
    Ok(conv.convert(value))
}

/// dlopen libm and friends with RTLD_GLOBAL, so model libraries that expect them already
/// resolved can load. Harmless to call more than once.
#[pyfunction]
fn preload_dependencies() {
    bmi_core::preload_dependencies();
}

#[pymodule]
#[pyo3(name = "_bmi_driver")]
fn bmi_driver_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Model>()?;
    m.add_function(wrap_pyfunction!(convert, m)?)?;
    m.add_function(wrap_pyfunction!(preload_dependencies, m)?)?;
    #[cfg(feature = "driver")]
    driver::register(m)?;
    Ok(())
}
