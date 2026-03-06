use super::check_initialized;
use crate::error::{function_failed, BmiResult};
use crate::traits::{Bmi, VarType};
use std::collections::HashMap;

#[derive(Debug, Clone)]
struct SlothVar {
    count: usize,
    var_type: VarType,
    type_str: String,
    units: String,
    location: String,
    value: f64,
}

pub struct BmiSloth {
    name: String,
    initialized: bool,
    variables: HashMap<String, SlothVar>,
    var_names: Vec<String>,
    type_cache: Option<HashMap<String, VarType>>,
    current_time: f64,
    time_step: f64,
}

impl BmiSloth {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            initialized: false,
            variables: HashMap::new(),
            var_names: Vec::new(),
            type_cache: None,
            current_time: 0.0,
            time_step: 3600.0,
        }
    }

    pub fn configure(&mut self, params: &HashMap<String, String>) -> BmiResult<()> {
        self.variables.clear();
        self.var_names.clear();

        for (key, val) in params {
            if let Some((name, var)) = Self::parse_param(key, val)? {
                self.var_names.push(name.clone());
                self.variables.insert(name, var);
            }
        }

        let cache: HashMap<String, VarType> = self
            .variables
            .iter()
            .map(|(n, v)| (n.clone(), v.var_type.clone()))
            .collect();
        self.type_cache = Some(cache);
        Ok(())
    }

    fn parse_param(key: &str, val: &str) -> BmiResult<Option<(String, SlothVar)>> {
        let (paren_start, paren_end) = match (key.find('('), key.find(')')) {
            (Some(s), Some(e)) => (s, e),
            _ => return Ok(None),
        };

        let name = key[..paren_start].to_string();
        let parts: Vec<&str> = key[paren_start + 1..paren_end].split(',').collect();

        if parts.len() != 4 {
            return Err(function_failed("SLOTH", format!("Invalid param format: {}", key)));
        }

        let count: usize = parts[0].trim().parse().unwrap_or(1);
        let type_str = parts[1].trim().to_string();
        let units = parts[2].trim().to_string();
        let location = parts[3].trim().to_string();

        let var_type = match type_str.as_str() {
            "double" | "float64" | "real8" => VarType::Double,
            "float" | "float32" | "real" | "real4" => VarType::Float,
            "int" | "integer" | "int32" => VarType::Int,
            _ => VarType::Double,
        };

        let value: f64 = val
            .trim()
            .parse()
            .map_err(|_| function_failed("SLOTH", format!("Invalid value '{}' for {}", val, name)))?;

        Ok(Some((
            name,
            SlothVar {
                count,
                var_type,
                type_str,
                units,
                location,
                value,
            },
        )))
    }

    fn get_var(&self, name: &str) -> BmiResult<&SlothVar> {
        self.variables
            .get(name)
            .ok_or_else(|| function_failed(&self.name, format!("Unknown variable: {}", name)))
    }
}

impl Bmi for BmiSloth {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_initialized(&self) -> bool {
        self.initialized
    }
    fn var_type_cache(&self) -> Option<&HashMap<String, VarType>> {
        self.type_cache.as_ref()
    }
    fn var_type_cache_mut(&mut self) -> &mut Option<HashMap<String, VarType>> {
        &mut self.type_cache
    }

    fn initialize(&mut self, _config: &str) -> BmiResult<()> {
        self.initialized = true;
        self.current_time = 0.0;
        Ok(())
    }

    fn update(&mut self) -> BmiResult<()> {
        check_initialized(self.initialized, &self.name)?;
        self.current_time += self.time_step;
        Ok(())
    }

    fn update_until(&mut self, time: f64) -> BmiResult<()> {
        check_initialized(self.initialized, &self.name)?;
        self.current_time = time;
        Ok(())
    }

    fn finalize(&mut self) -> BmiResult<()> {
        self.initialized = false;
        Ok(())
    }

    fn get_component_name(&self) -> BmiResult<String> {
        Ok(format!("SLOTH ({})", self.name))
    }
    fn get_input_item_count(&self) -> BmiResult<i32> {
        Ok(0)
    }
    fn get_output_item_count(&self) -> BmiResult<i32> {
        Ok(self.var_names.len() as i32)
    }
    fn get_input_var_names(&self) -> BmiResult<Vec<String>> {
        Ok(vec![])
    }
    fn get_output_var_names(&self) -> BmiResult<Vec<String>> {
        Ok(self.var_names.clone())
    }

    fn get_var_grid(&self, _name: &str) -> BmiResult<i32> {
        Ok(0)
    }

    fn get_var_type(&self, name: &str) -> BmiResult<String> {
        Ok(self.get_var(name)?.type_str.clone())
    }

    fn get_var_units(&self, name: &str) -> BmiResult<String> {
        Ok(self.get_var(name)?.units.clone())
    }

    fn get_var_itemsize(&self, name: &str) -> BmiResult<i32> {
        Ok(match self.get_var(name)?.var_type {
            VarType::Double => 8,
            VarType::Float | VarType::Int => 4,
            VarType::Unknown(_) => 8,
        })
    }

    fn get_var_nbytes(&self, name: &str) -> BmiResult<i32> {
        let var = self.get_var(name)?;
        let itemsize = match var.var_type {
            VarType::Double => 8,
            VarType::Float | VarType::Int => 4,
            VarType::Unknown(_) => 8,
        };
        Ok((var.count * itemsize) as i32)
    }

    fn get_var_location(&self, name: &str) -> BmiResult<String> {
        Ok(self.get_var(name)?.location.clone())
    }

    fn get_current_time(&self) -> BmiResult<f64> {
        Ok(self.current_time)
    }
    fn get_start_time(&self) -> BmiResult<f64> {
        Ok(0.0)
    }
    fn get_end_time(&self) -> BmiResult<f64> {
        Ok(f64::MAX)
    }
    fn get_time_units(&self) -> BmiResult<String> {
        Ok("s".into())
    }
    fn get_time_step(&self) -> BmiResult<f64> {
        Ok(self.time_step)
    }

    fn get_value_f64(&self, name: &str) -> BmiResult<Vec<f64>> {
        let var = self.get_var(name)?;
        Ok(vec![var.value; var.count])
    }

    fn get_value_f32(&self, name: &str) -> BmiResult<Vec<f32>> {
        let var = self.get_var(name)?;
        Ok(vec![var.value as f32; var.count])
    }

    fn get_value_i32(&self, name: &str) -> BmiResult<Vec<i32>> {
        let var = self.get_var(name)?;
        Ok(vec![var.value as i32; var.count])
    }

    fn set_value_f64(&mut self, name: &str, values: &[f64]) -> BmiResult<()> {
        if let Some(var) = self.variables.get_mut(name) {
            if let Some(&v) = values.first() {
                var.value = v;
            }
        }
        Ok(())
    }

    fn set_value_f32(&mut self, name: &str, values: &[f32]) -> BmiResult<()> {
        if let Some(var) = self.variables.get_mut(name) {
            if let Some(&v) = values.first() {
                var.value = v as f64;
            }
        }
        Ok(())
    }

    fn set_value_i32(&mut self, name: &str, values: &[i32]) -> BmiResult<()> {
        if let Some(var) = self.variables.get_mut(name) {
            if let Some(&v) = values.first() {
                var.value = v as f64;
            }
        }
        Ok(())
    }

    fn get_grid_rank(&self, _grid: i32) -> BmiResult<i32> {
        Ok(1)
    }
    fn get_grid_size(&self, _grid: i32) -> BmiResult<i32> {
        Ok(1)
    }
    fn get_grid_type(&self, _grid: i32) -> BmiResult<String> {
        Ok("scalar".into())
    }
}
