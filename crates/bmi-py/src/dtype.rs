//! Choosing which typed BMI call to make, and moving values across the Python boundary.

use numpy::PyArray1;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use bmi_core::traits::{BmiValue, VarType};

/// Which element type to use for a variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dtype {
    /// Whatever the model itself reports for each variable. The default, and almost always right.
    Auto,
    F64,
    F32,
    I32,
}

impl Dtype {
    pub fn parse(s: &str) -> PyResult<Self> {
        match s.to_lowercase().as_str() {
            "auto" | "" => Ok(Dtype::Auto),
            "f64" | "float64" | "double" => Ok(Dtype::F64),
            "f32" | "float32" | "float" => Ok(Dtype::F32),
            "i32" | "int32" | "int" => Ok(Dtype::I32),
            other => Err(PyValueError::new_err(format!(
                "unknown dtype {other:?}: expected one of auto, float64, float32, int32"
            ))),
        }
    }

    /// The type to actually use, given what the model says the variable is.
    pub fn resolve(self, native: VarType) -> VarType {
        match self {
            Dtype::Auto => native,
            Dtype::F64 => VarType::Double,
            Dtype::F32 => VarType::Float,
            Dtype::I32 => VarType::Int,
        }
    }
}

pub fn name_of(vt: VarType) -> String {
    match vt {
        VarType::Double => "float64",
        VarType::Float => "float32",
        VarType::Int => "int32",
        VarType::Unknown(_) => "float64",
    }
    .to_string()
}

/// Hand a BMI value to Python as a numpy array, honouring a pinned dtype.
pub fn value_to_py<'py>(
    py: Python<'py>,
    value: BmiValue,
    dtype: Dtype,
) -> PyResult<Bound<'py, PyAny>> {
    let native = match &value {
        BmiValue::Double(_) => VarType::Double,
        BmiValue::Float(_) => VarType::Float,
        BmiValue::Int(_) => VarType::Int,
    };
    Ok(match dtype.resolve(native) {
        VarType::Double => PyArray1::from_vec_bound(py, value.as_f64()).into_any(),
        VarType::Float => {
            let v: Vec<f32> = value.as_f64().into_iter().map(|x| x as f32).collect();
            PyArray1::from_vec_bound(py, v).into_any()
        }
        VarType::Int => {
            let v: Vec<i32> = value.as_f64().into_iter().map(|x| x as i32).collect();
            PyArray1::from_vec_bound(py, v).into_any()
        }
        VarType::Unknown(_) => PyArray1::from_vec_bound(py, value.as_f64()).into_any(),
    })
}

/// Coerce a scalar, sequence or numpy array into a `Vec<f64>`.
pub fn py_to_f64_vec(obj: &Bound<'_, PyAny>) -> PyResult<Vec<f64>> {
    if let Ok(v) = obj.extract::<Vec<f64>>() {
        return Ok(v);
    }
    if let Ok(x) = obj.extract::<f64>() {
        return Ok(vec![x]);
    }
    // numpy arrays and anything else array-like that can flatten itself to a Python list
    if let Ok(list) = obj.call_method0("tolist") {
        if let Ok(v) = list.extract::<Vec<f64>>() {
            return Ok(v);
        }
        if let Ok(x) = list.extract::<f64>() {
            return Ok(vec![x]);
        }
    }
    Err(PyValueError::new_err(
        "expected a float, a sequence of floats, or a 1-D numeric array",
    ))
}
