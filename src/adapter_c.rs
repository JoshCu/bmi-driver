//! C BMI adapter implementation.
//!
//! This module provides a safe wrapper for BMI models that use the C interface
//! (function pointer struct with registration function).

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::path::Path;

use crate::bmi_ffi::{
    Bmi as BmiStruct, BmiRegistrationFn, BMI_MAX_COMPONENT_NAME, BMI_MAX_LOCATION_NAME,
    BMI_MAX_TYPE_NAME, BMI_MAX_UNITS_NAME, BMI_MAX_VAR_NAME, BMI_SUCCESS,
};
use crate::error::{BmiError, BmiResult};
use crate::library::GlobalLibrary;
use crate::traits::{Bmi, VarType};

/// A safe wrapper around a BMI C model loaded from a shared library.
///
/// This adapter is for models that implement the standard C BMI interface
/// with a registration function that populates a struct of function pointers.
pub struct BmiC {
    model_name: String,
    _library: GlobalLibrary,
    bmi: Box<BmiStruct>,
    initialized: bool,
    time_convert_factor: f64,
    /// Cache of variable types for auto-typing
    var_type_cache: Option<HashMap<String, VarType>>,
}

impl BmiC {
    /// Load a BMI C model from a shared library.
    ///
    /// # Arguments
    ///
    /// * `model_name` - A descriptive name for this model (used in error messages)
    /// * `library_path` - Path to the shared library (.so, .dylib, or .dll)
    /// * `registration_func` - Name of the function that registers BMI function pointers
    pub fn load(
        model_name: impl Into<String>,
        library_path: impl AsRef<Path>,
        registration_func: &str,
    ) -> BmiResult<Self> {
        let model_name = model_name.into();
        let library_path = library_path.as_ref();

        let library =
            unsafe { GlobalLibrary::new(library_path) }.map_err(|e| BmiError::LibraryLoad {
                path: library_path.display().to_string(),
                source: e,
            })?;

        let register_fn: BmiRegistrationFn =
            unsafe { library.get_fn(registration_func) }.map_err(|e| {
                BmiError::RegistrationFunctionNotFound {
                    func: registration_func.to_string(),
                    source: e,
                }
            })?;

        let mut bmi = Box::new(BmiStruct::default());
        unsafe {
            register_fn(bmi.as_mut());
        }

        Ok(Self {
            model_name,
            _library: library,
            bmi,
            initialized: false,
            time_convert_factor: 1.0,
            var_type_cache: None,
        })
    }

    fn require_initialized(&self) -> BmiResult<()> {
        if !self.initialized {
            Err(BmiError::NotInitialized {
                model: self.model_name.clone(),
            })
        } else {
            Ok(())
        }
    }

    fn calculate_time_convert_factor(&self) -> f64 {
        let time_units = match self.get_time_units() {
            Ok(units) => units.to_lowercase(),
            Err(_) => return 1.0,
        };

        match time_units.trim() {
            "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
            "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
            "d" | "day" | "days" => 86400.0,
            "week" | "weeks" => 604800.0,
            _ => {
                if time_units.contains("second") {
                    1.0
                } else if time_units.contains("minute") {
                    60.0
                } else if time_units.contains("hour") {
                    3600.0
                } else if time_units.contains("day") {
                    86400.0
                } else {
                    1.0
                }
            }
        }
    }

    // Helper methods for FFI calls
    fn get_string_field(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut BmiStruct, *mut c_char) -> i32>,
        max_len: usize,
    ) -> BmiResult<String> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let mut buffer = vec![0u8; max_len];
        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                buffer.as_mut_ptr() as *mut c_char,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: func_name.to_string(),
            });
        }

        cstr_to_string(&buffer)
    }

    fn get_int_field(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut BmiStruct, *mut i32) -> i32>,
    ) -> BmiResult<i32> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let mut value: i32 = 0;
        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                &mut value,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: func_name.to_string(),
            });
        }

        Ok(value)
    }

    fn get_double_field(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut BmiStruct, *mut f64) -> i32>,
    ) -> BmiResult<f64> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let mut value: f64 = 0.0;
        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                &mut value,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: func_name.to_string(),
            });
        }

        Ok(value)
    }

    fn get_int_for_var(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut BmiStruct, *const c_char, *mut i32) -> i32>,
        name: &str,
    ) -> BmiResult<i32> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut value: i32 = 0;
        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                name_cstr.as_ptr(),
                &mut value,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: func_name.to_string(),
            });
        }

        Ok(value)
    }

    fn get_string_for_var(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut BmiStruct, *const c_char, *mut c_char) -> i32>,
        name: &str,
        max_len: usize,
    ) -> BmiResult<String> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut buffer = vec![0u8; max_len];
        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                name_cstr.as_ptr(),
                buffer.as_mut_ptr() as *mut c_char,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: func_name.to_string(),
            });
        }

        cstr_to_string(&buffer)
    }

    fn get_int_for_grid(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut BmiStruct, i32, *mut i32) -> i32>,
        grid: i32,
    ) -> BmiResult<i32> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let mut value: i32 = 0;
        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                grid,
                &mut value,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: func_name.to_string(),
            });
        }

        Ok(value)
    }

    fn get_var_names(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut BmiStruct, *mut *mut c_char) -> i32>,
        count: usize,
    ) -> BmiResult<Vec<String>> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let mut buffers: Vec<Vec<u8>> = (0..count).map(|_| vec![0u8; BMI_MAX_VAR_NAME]).collect();
        let mut ptrs: Vec<*mut c_char> = buffers
            .iter_mut()
            .map(|b| b.as_mut_ptr() as *mut c_char)
            .collect();

        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                ptrs.as_mut_ptr(),
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: func_name.to_string(),
            });
        }

        buffers.iter().map(|b| cstr_to_string(b)).collect()
    }
}

impl Bmi for BmiC {
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn is_initialized(&self) -> bool {
        self.initialized
    }

    fn initialize(&mut self, config_file: &str) -> BmiResult<()> {
        if self.initialized {
            return Err(BmiError::AlreadyInitialized {
                model: self.model_name.clone(),
            });
        }

        let config_path = Path::new(config_file);
        if !config_path.exists() {
            return Err(BmiError::ConfigFileNotFound {
                path: config_file.to_string(),
            });
        }

        let config_cstr = CString::new(config_file).map_err(|_| BmiError::InvalidUtf8)?;
        let func = self
            .bmi
            .initialize
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "initialize".to_string(),
            })?;

        let result = unsafe { func(self.bmi.as_mut(), config_cstr.as_ptr()) };
        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "initialize".to_string(),
            });
        }

        self.initialized = true;
        self.time_convert_factor = self.calculate_time_convert_factor();

        // Cache variable types for auto-typing
        self.cache_var_types()?;

        Ok(())
    }

    fn update(&mut self) -> BmiResult<()> {
        self.require_initialized()?;

        let func = self
            .bmi
            .update
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "update".to_string(),
            })?;

        let result = unsafe { func(self.bmi.as_mut()) };
        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "update".to_string(),
            });
        }
        Ok(())
    }

    fn update_until(&mut self, time: f64) -> BmiResult<()> {
        self.require_initialized()?;

        let func = self
            .bmi
            .update_until
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "update_until".to_string(),
            })?;

        let result = unsafe { func(self.bmi.as_mut(), time) };
        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "update_until".to_string(),
            });
        }
        Ok(())
    }

    fn update_for_duration_seconds(&mut self, duration_seconds: f64) -> BmiResult<()> {
        self.require_initialized()?;

        let duration_model_units = self.convert_seconds_to_model_time(duration_seconds);
        let model_time_step = self.get_time_step()?;
        let current_time = self.get_current_time()?;

        if (duration_model_units - model_time_step).abs() < 1e-10 {
            self.update()
        } else {
            self.update_until(current_time + duration_model_units)
        }
    }

    fn finalize(&mut self) -> BmiResult<()> {
        if !self.initialized {
            return Ok(());
        }

        let func = self
            .bmi
            .finalize
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "finalize".to_string(),
            })?;

        let result = unsafe { func(self.bmi.as_mut()) };
        self.initialized = false;

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "finalize".to_string(),
            });
        }
        Ok(())
    }

    fn get_component_name(&self) -> BmiResult<String> {
        self.get_string_field(
            "get_component_name",
            self.bmi.get_component_name,
            BMI_MAX_COMPONENT_NAME,
        )
    }

    fn get_input_item_count(&self) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_field("get_input_item_count", self.bmi.get_input_item_count)
    }

    fn get_output_item_count(&self) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_field("get_output_item_count", self.bmi.get_output_item_count)
    }

    fn get_input_var_names(&self) -> BmiResult<Vec<String>> {
        self.require_initialized()?;
        let count = self.get_input_item_count()? as usize;
        self.get_var_names("get_input_var_names", self.bmi.get_input_var_names, count)
    }

    fn get_output_var_names(&self) -> BmiResult<Vec<String>> {
        self.require_initialized()?;
        let count = self.get_output_item_count()? as usize;
        self.get_var_names("get_output_var_names", self.bmi.get_output_var_names, count)
    }

    fn get_var_grid(&self, name: &str) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_for_var("get_var_grid", self.bmi.get_var_grid, name)
    }

    fn get_var_type(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;
        self.get_string_for_var(
            "get_var_type",
            self.bmi.get_var_type,
            name,
            BMI_MAX_TYPE_NAME,
        )
    }

    fn get_var_units(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;
        self.get_string_for_var(
            "get_var_units",
            self.bmi.get_var_units,
            name,
            BMI_MAX_UNITS_NAME,
        )
    }

    fn get_var_itemsize(&self, name: &str) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_for_var("get_var_itemsize", self.bmi.get_var_itemsize, name)
    }

    fn get_var_nbytes(&self, name: &str) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_for_var("get_var_nbytes", self.bmi.get_var_nbytes, name)
    }

    fn get_var_location(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;
        self.get_string_for_var(
            "get_var_location",
            self.bmi.get_var_location,
            name,
            BMI_MAX_LOCATION_NAME,
        )
    }

    fn get_current_time(&self) -> BmiResult<f64> {
        self.require_initialized()?;
        self.get_double_field("get_current_time", self.bmi.get_current_time)
    }

    fn get_start_time(&self) -> BmiResult<f64> {
        self.require_initialized()?;
        self.get_double_field("get_start_time", self.bmi.get_start_time)
    }

    fn get_end_time(&self) -> BmiResult<f64> {
        self.require_initialized()?;
        self.get_double_field("get_end_time", self.bmi.get_end_time)
    }

    fn get_time_units(&self) -> BmiResult<String> {
        self.require_initialized()?;
        self.get_string_field(
            "get_time_units",
            self.bmi.get_time_units,
            BMI_MAX_UNITS_NAME,
        )
    }

    fn get_time_step(&self) -> BmiResult<f64> {
        self.require_initialized()?;
        self.get_double_field("get_time_step", self.bmi.get_time_step)
    }

    fn get_time_convert_factor(&self) -> f64 {
        self.time_convert_factor
    }

    fn convert_model_time_to_seconds(&self, model_time: f64) -> f64 {
        model_time * self.time_convert_factor
    }

    fn convert_seconds_to_model_time(&self, seconds: f64) -> f64 {
        seconds / self.time_convert_factor
    }

    fn get_value_f64(&self, name: &str) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;

        let func = self
            .bmi
            .get_value
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "get_value".to_string(),
            })?;

        let nbytes = self.get_var_nbytes(name)? as usize;
        let count = nbytes / std::mem::size_of::<f64>();
        let mut values = vec![0.0f64; count];

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                name_cstr.as_ptr(),
                values.as_mut_ptr() as *mut c_void,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_value".to_string(),
            });
        }

        Ok(values)
    }

    fn get_value_f32(&self, name: &str) -> BmiResult<Vec<f32>> {
        self.require_initialized()?;

        let func = self
            .bmi
            .get_value
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "get_value".to_string(),
            })?;

        let nbytes = self.get_var_nbytes(name)? as usize;
        let count = nbytes / std::mem::size_of::<f32>();
        let mut values = vec![0.0f32; count];

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                name_cstr.as_ptr(),
                values.as_mut_ptr() as *mut c_void,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_value".to_string(),
            });
        }

        Ok(values)
    }

    fn get_value_i32(&self, name: &str) -> BmiResult<Vec<i32>> {
        self.require_initialized()?;

        let func = self
            .bmi
            .get_value
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "get_value".to_string(),
            })?;

        let nbytes = self.get_var_nbytes(name)? as usize;
        let count = nbytes / std::mem::size_of::<i32>();
        let mut values = vec![0i32; count];

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                name_cstr.as_ptr(),
                values.as_mut_ptr() as *mut c_void,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_value".to_string(),
            });
        }

        Ok(values)
    }

    fn get_value_at_indices_f64(&self, name: &str, indices: &[i32]) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;

        let func =
            self.bmi
                .get_value_at_indices
                .ok_or_else(|| BmiError::FunctionNotImplemented {
                    model: self.model_name.clone(),
                    func: "get_value_at_indices".to_string(),
                })?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut values = vec![0.0f64; indices.len()];

        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                name_cstr.as_ptr(),
                values.as_mut_ptr() as *mut c_void,
                indices.as_ptr() as *mut i32,
                indices.len() as i32,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_value_at_indices".to_string(),
            });
        }

        Ok(values)
    }

    fn set_value_f64(&mut self, name: &str, values: &[f64]) -> BmiResult<()> {
        self.require_initialized()?;

        let func = self
            .bmi
            .set_value
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "set_value".to_string(),
            })?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = unsafe {
            func(
                self.bmi.as_mut(),
                name_cstr.as_ptr(),
                values.as_ptr() as *mut c_void,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "set_value".to_string(),
            });
        }

        Ok(())
    }

    fn set_value_f32(&mut self, name: &str, values: &[f32]) -> BmiResult<()> {
        self.require_initialized()?;

        let func = self
            .bmi
            .set_value
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "set_value".to_string(),
            })?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = unsafe {
            func(
                self.bmi.as_mut(),
                name_cstr.as_ptr(),
                values.as_ptr() as *mut c_void,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "set_value".to_string(),
            });
        }

        Ok(())
    }

    fn set_value_i32(&mut self, name: &str, values: &[i32]) -> BmiResult<()> {
        self.require_initialized()?;

        let func = self
            .bmi
            .set_value
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "set_value".to_string(),
            })?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = unsafe {
            func(
                self.bmi.as_mut(),
                name_cstr.as_ptr(),
                values.as_ptr() as *mut c_void,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "set_value".to_string(),
            });
        }

        Ok(())
    }

    fn set_value_at_indices_f64(
        &mut self,
        name: &str,
        indices: &[i32],
        values: &[f64],
    ) -> BmiResult<()> {
        self.require_initialized()?;

        let func =
            self.bmi
                .set_value_at_indices
                .ok_or_else(|| BmiError::FunctionNotImplemented {
                    model: self.model_name.clone(),
                    func: "set_value_at_indices".to_string(),
                })?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = unsafe {
            func(
                self.bmi.as_mut(),
                name_cstr.as_ptr(),
                indices.as_ptr() as *mut i32,
                indices.len() as i32,
                values.as_ptr() as *mut c_void,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "set_value_at_indices".to_string(),
            });
        }

        Ok(())
    }

    fn get_grid_rank(&self, grid: i32) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_for_grid("get_grid_rank", self.bmi.get_grid_rank, grid)
    }

    fn get_grid_size(&self, grid: i32) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_for_grid("get_grid_size", self.bmi.get_grid_size, grid)
    }

    fn get_grid_type(&self, grid: i32) -> BmiResult<String> {
        self.require_initialized()?;

        let func = self
            .bmi
            .get_grid_type
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "get_grid_type".to_string(),
            })?;

        let mut buffer = vec![0u8; BMI_MAX_TYPE_NAME];
        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                grid,
                buffer.as_mut_ptr() as *mut c_char,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_grid_type".to_string(),
            });
        }

        cstr_to_string(&buffer)
    }

    fn get_grid_shape(&self, grid: i32) -> BmiResult<Vec<i32>> {
        self.require_initialized()?;
        let rank = self.get_grid_rank(grid)? as usize;

        let func = self
            .bmi
            .get_grid_shape
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "get_grid_shape".to_string(),
            })?;

        let mut values = vec![0i32; rank];
        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                grid,
                values.as_mut_ptr(),
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_grid_shape".to_string(),
            });
        }

        Ok(values)
    }

    fn get_grid_spacing(&self, grid: i32) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;
        let rank = self.get_grid_rank(grid)? as usize;

        let func = self
            .bmi
            .get_grid_spacing
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "get_grid_spacing".to_string(),
            })?;

        let mut values = vec![0.0f64; rank];
        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                grid,
                values.as_mut_ptr(),
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_grid_spacing".to_string(),
            });
        }

        Ok(values)
    }

    fn get_grid_origin(&self, grid: i32) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;
        let rank = self.get_grid_rank(grid)? as usize;

        let func = self
            .bmi
            .get_grid_origin
            .ok_or_else(|| BmiError::FunctionNotImplemented {
                model: self.model_name.clone(),
                func: "get_grid_origin".to_string(),
            })?;

        let mut values = vec![0.0f64; rank];
        let result = unsafe {
            func(
                self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct,
                grid,
                values.as_mut_ptr(),
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_grid_origin".to_string(),
            });
        }

        Ok(values)
    }

    fn get_var_type_cache(&self) -> Option<&HashMap<String, VarType>> {
        self.var_type_cache.as_ref()
    }

    fn get_var_type_cache_mut(&mut self) -> &mut Option<HashMap<String, VarType>> {
        &mut self.var_type_cache
    }
}

impl Drop for BmiC {
    fn drop(&mut self) {
        if self.initialized {
            let _ = self.finalize();
        }
    }
}

// Helper functions
fn cstr_to_string(buffer: &[u8]) -> BmiResult<String> {
    let cstr = unsafe { CStr::from_ptr(buffer.as_ptr() as *const c_char) };
    cstr.to_str()
        .map(|s| s.to_string())
        .map_err(|_| BmiError::InvalidUtf8)
}
