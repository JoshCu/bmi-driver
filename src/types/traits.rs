use crate::error::{BmiError, BmiResult};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum VarType {
    Float,
    Double,
    Int,
    Unknown(String),
}

impl VarType {
    pub fn from_bmi(type_name: &str, item_size: i32) -> Self {
        let t = type_name.to_lowercase();
        match t.as_str() {
            "float" | "real" | "float32" | "real*4" | "real4" => VarType::Float,
            "double" | "double precision" | "float64" | "real*8" | "real8" => VarType::Double,
            "int" | "integer" | "int32" | "integer*4" | "integer4" | "i32" => VarType::Int,
            _ if t.contains("double") => VarType::Double,
            _ if t.contains("float") || t.contains("real") => {
                if item_size == 8 {
                    VarType::Double
                } else {
                    VarType::Float
                }
            }
            _ if t.contains("int") => VarType::Int,
            _ => match item_size {
                4 => VarType::Float,
                8 => VarType::Double,
                _ => VarType::Unknown(type_name.into()),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum BmiValue {
    Float(Vec<f32>),
    Double(Vec<f64>),
    Int(Vec<i32>),
}

impl BmiValue {
    pub fn as_f64(&self) -> Vec<f64> {
        match self {
            BmiValue::Double(v) => v.clone(),
            BmiValue::Float(v) => v.iter().map(|&x| x as f64).collect(),
            BmiValue::Int(v) => v.iter().map(|&x| x as f64).collect(),
        }
    }

    pub fn scalar(&self) -> Option<f64> {
        match self {
            BmiValue::Double(v) => v.first().copied(),
            BmiValue::Float(v) => v.first().map(|&x| x as f64),
            BmiValue::Int(v) => v.first().map(|&x| x as f64),
        }
    }
}

pub trait Bmi {
    fn name(&self) -> &str;
    fn is_initialized(&self) -> bool;
    fn var_type_cache(&self) -> Option<&HashMap<String, VarType>>;
    fn var_type_cache_mut(&mut self) -> &mut Option<HashMap<String, VarType>>;

    fn initialize(&mut self, config: &str) -> BmiResult<()>;
    fn update(&mut self) -> BmiResult<()>;
    fn update_until(&mut self, time: f64) -> BmiResult<()>;
    fn finalize(&mut self) -> BmiResult<()>;

    fn get_component_name(&self) -> BmiResult<String>;
    fn get_input_item_count(&self) -> BmiResult<i32>;
    fn get_output_item_count(&self) -> BmiResult<i32>;
    fn get_input_var_names(&self) -> BmiResult<Vec<String>>;
    fn get_output_var_names(&self) -> BmiResult<Vec<String>>;

    fn get_var_grid(&self, name: &str) -> BmiResult<i32>;
    fn get_var_type(&self, name: &str) -> BmiResult<String>;
    fn get_var_units(&self, name: &str) -> BmiResult<String>;
    fn get_var_itemsize(&self, name: &str) -> BmiResult<i32>;
    fn get_var_nbytes(&self, name: &str) -> BmiResult<i32>;
    fn get_var_location(&self, name: &str) -> BmiResult<String>;

    fn get_current_time(&self) -> BmiResult<f64>;
    fn get_start_time(&self) -> BmiResult<f64>;
    fn get_end_time(&self) -> BmiResult<f64>;
    fn get_time_units(&self) -> BmiResult<String>;
    fn get_time_step(&self) -> BmiResult<f64>;

    fn get_value_f64(&self, name: &str) -> BmiResult<Vec<f64>>;
    fn get_value_f32(&self, name: &str) -> BmiResult<Vec<f32>>;
    fn get_value_i32(&self, name: &str) -> BmiResult<Vec<i32>>;

    fn set_value_f64(&mut self, name: &str, values: &[f64]) -> BmiResult<()>;
    fn set_value_f32(&mut self, name: &str, values: &[f32]) -> BmiResult<()>;
    fn set_value_i32(&mut self, name: &str, values: &[i32]) -> BmiResult<()>;

    fn get_grid_rank(&self, grid: i32) -> BmiResult<i32>;
    fn get_grid_size(&self, grid: i32) -> BmiResult<i32>;
    fn get_grid_type(&self, grid: i32) -> BmiResult<String>;

    fn time_factor(&self) -> f64 {
        1.0
    }

    fn to_seconds(&self, model_time: f64) -> f64 {
        model_time * self.time_factor()
    }

    fn from_seconds(&self, seconds: f64) -> f64 {
        seconds / self.time_factor()
    }

    fn cached_type(&self, name: &str) -> Option<VarType> {
        self.var_type_cache().and_then(|c| c.get(name).cloned())
    }

    fn cache_types(&mut self) -> BmiResult<()> {
        if self.var_type_cache().is_some() {
            return Ok(());
        }

        let mut cache = HashMap::new();
        for name in self
            .get_input_var_names()?
            .into_iter()
            .chain(self.get_output_var_names()?)
        {
            let t = self.get_var_type(&name)?;
            let s = self.get_var_itemsize(&name)?;
            cache.insert(name, VarType::from_bmi(&t, s));
        }
        *self.var_type_cache_mut() = Some(cache);
        Ok(())
    }
}

pub trait BmiExt: Bmi {
    fn get_value(&self, name: &str) -> BmiResult<BmiValue> {
        let vt = self.cached_type(name).unwrap_or_else(|| {
            let t = self.get_var_type(name).unwrap_or_default();
            let s = self.get_var_itemsize(name).unwrap_or(8);
            VarType::from_bmi(&t, s)
        });

        match vt {
            VarType::Float => Ok(BmiValue::Float(self.get_value_f32(name)?)),
            VarType::Double => Ok(BmiValue::Double(self.get_value_f64(name)?)),
            VarType::Int => Ok(BmiValue::Int(self.get_value_i32(name)?)),
            VarType::Unknown(t) => Err(BmiError::FunctionFailed {
                model: self.name().into(),
                func: format!("unknown type '{}'", t),
            }),
        }
    }

    fn set_value(&mut self, name: &str, values: &[f64]) -> BmiResult<()> {
        let vt = self.cached_type(name).unwrap_or_else(|| {
            let t = self.get_var_type(name).unwrap_or_default();
            let s = self.get_var_itemsize(name).unwrap_or(8);
            VarType::from_bmi(&t, s)
        });

        match vt {
            VarType::Float => {
                self.set_value_f32(name, &values.iter().map(|&x| x as f32).collect::<Vec<_>>())
            }
            VarType::Double => self.set_value_f64(name, values),
            VarType::Int => {
                self.set_value_i32(name, &values.iter().map(|&x| x as i32).collect::<Vec<_>>())
            }
            VarType::Unknown(t) => Err(BmiError::FunctionFailed {
                model: self.name().into(),
                func: format!("unknown type '{}'", t),
            }),
        }
    }

    fn get_scalar(&self, name: &str) -> BmiResult<f64> {
        self.get_value(name)?
            .scalar()
            .ok_or_else(|| BmiError::FunctionFailed {
                model: self.name().into(),
                func: "empty result".into(),
            })
    }
}

impl<T: Bmi + ?Sized> BmiExt for T {}

pub fn parse_time_units(units: &str) -> f64 {
    let u = units.to_lowercase();
    match u.trim() {
        "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
        "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
        "d" | "day" | "days" => 86400.0,
        _ if u.contains("minute") => 60.0,
        _ if u.contains("hour") => 3600.0,
        _ if u.contains("day") => 86400.0,
        _ => 1.0,
    }
}
