//! Python access to the driver layer: forcing data and whole-config runs.
//!
//! Together with `Model`, this is enough to write the rest of a model engine in Python: read the
//! real forcings, drive the real model libraries, and compare against what the Rust driver
//! produces for the same location.

use std::collections::HashMap;
use std::path::PathBuf;

use numpy::PyArray1;
use pyo3::prelude::*;

use bmi_driver::forcings::Forcings as _;
use bmi_driver::{NetCdfForcings, Simulation};

use crate::to_py_err;

/// A NetCDF forcing file.
#[pyclass(unsendable, module = "bmi_driver")]
pub struct Forcings {
    inner: NetCdfForcings,
    path: String,
}

#[pymethods]
impl Forcings {
    #[new]
    fn new(path: &str) -> PyResult<Self> {
        let mut inner = NetCdfForcings::new("forcings");
        inner.initialize(path).map_err(to_py_err)?;
        Ok(Self {
            inner,
            path: path.to_string(),
        })
    }

    fn variable_names(&self) -> PyResult<Vec<String>> {
        self.inner.var_names().map_err(to_py_err)
    }

    fn location_ids(&self) -> PyResult<Vec<String>> {
        self.inner.location_ids().map_err(to_py_err)
    }

    fn units(&self, name: &str) -> PyResult<String> {
        self.inner.var_units(name).map_err(to_py_err)
    }

    /// Seconds between forcing timesteps.
    fn time_step(&self) -> PyResult<f64> {
        self.inner.time_step().map_err(to_py_err)
    }

    fn timestep_count(&self) -> PyResult<usize> {
        self.inner.timestep_count().map_err(to_py_err)
    }

    /// The full time series for one variable at one location.
    fn get<'py>(
        &mut self,
        py: Python<'py>,
        variable: &str,
        location: &str,
    ) -> PyResult<Bound<'py, PyArray1<f64>>> {
        let values = self.inner.series(variable, location).map_err(to_py_err)?;
        Ok(PyArray1::from_vec_bound(py, values))
    }

    /// One value, for stepping through a series without materialising it.
    fn get_at(&self, variable: &str, location: &str, step: usize) -> PyResult<f64> {
        self.inner
            .get_f64(variable, location, step)
            .map_err(to_py_err)
    }

    fn __repr__(&self) -> String {
        format!("<Forcings {:?}>", self.path)
    }
}

/// A whole realization config: model chain, forcings and time range.
///
/// ```python
/// sim = Simulation("/data/my_run")
/// out = sim.run("cat-123")           # dict of numpy arrays
/// q = sim.run_variable("cat-123", "Q_OUT")
/// ```
#[pyclass(unsendable, module = "bmi_driver")]
pub struct PySimulation {
    inner: Simulation,
}

#[pymethods]
impl PySimulation {
    /// Open `<data_dir>/config/realization.json`, or an explicit `config` path whose relative
    /// paths still resolve against `data_dir`.
    #[new]
    #[pyo3(signature = (data_dir, config = None, warnings = false))]
    fn new(data_dir: PathBuf, config: Option<PathBuf>, warnings: bool) -> PyResult<Self> {
        let mut inner = match config {
            Some(cfg) => Simulation::with_config(&data_dir, cfg),
            None => Simulation::open(&data_dir),
        }
        .map_err(to_py_err)?;
        inner.set_warnings(warnings);
        Ok(Self { inner })
    }

    /// Run one location. Returns `{variable: numpy array}` for the variables the config selects.
    fn run<'py>(
        &mut self,
        py: Python<'py>,
        location: &str,
    ) -> PyResult<HashMap<String, Bound<'py, PyArray1<f64>>>> {
        let out = self.inner.run(location).map_err(to_py_err)?;
        Ok(out
            .variables
            .into_iter()
            .map(|(k, v)| (k, PyArray1::from_vec_bound(py, v)))
            .collect())
    }

    /// Run one location and return a single variable.
    fn run_variable<'py>(
        &mut self,
        py: Python<'py>,
        location: &str,
        variable: &str,
    ) -> PyResult<Bound<'py, PyArray1<f64>>> {
        let values = self
            .inner
            .run_variable(location, variable)
            .map_err(to_py_err)?;
        Ok(PyArray1::from_vec_bound(py, values))
    }

    /// Location ids from the hydrofabric GeoPackage in `<data_dir>/config/`.
    fn locations(&self) -> PyResult<Vec<String>> {
        self.inner.locations().map_err(to_py_err)
    }

    /// Units of an output or forcing variable, as reported by the model that produces it.
    fn var_units(&mut self, location: &str, variable: &str) -> PyResult<String> {
        self.inner.var_units(location, variable).map_err(to_py_err)
    }

    /// Unix epoch seconds for each output timestep.
    fn times<'py>(&self, py: Python<'py>) -> Bound<'py, PyArray1<i64>> {
        let (start, interval) = (self.inner.start_epoch(), self.inner.output_interval());
        let steps = self.inner.total_steps() as i64 + 1;
        PyArray1::from_vec_bound(py, (0..steps).map(|s| start + s * interval).collect())
    }

    #[getter]
    fn total_steps(&self) -> usize {
        self.inner.total_steps()
    }

    #[getter]
    fn start_epoch(&self) -> i64 {
        self.inner.start_epoch()
    }

    #[getter]
    fn output_interval(&self) -> i64 {
        self.inner.output_interval()
    }

    fn __repr__(&self) -> String {
        format!("<Simulation {:?}>", self.inner.data_dir().display())
    }
}

pub fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Forcings>()?;
    m.add_class::<PySimulation>()?;
    Ok(())
}
