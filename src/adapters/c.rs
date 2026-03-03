use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::path::Path;

use super::{check_initialized, cstr_to_string};
use crate::error::{BmiError, BmiResult};
use crate::ffi::{Bmi as BmiStruct, BMI_MAX_NAME, BMI_SUCCESS};
use crate::library::GlobalLibrary;
use crate::traits::{parse_time_units, Bmi, VarType};

/// BMI adapter for C shared libraries.
///
/// Loads a C `.so` via `dlopen` with `RTLD_GLOBAL`, calls a registration function to populate a
/// [`crate::ffi::Bmi`] struct of function pointers, then dispatches all BMI calls through those
/// pointers.
///
/// ## BMI functions called
///
/// **Lifecycle** — called once per simulation:
/// - `initialize` — passes the config path as a `*const c_char`.
/// - `finalize` — called on drop if the model was successfully initialized.
///
/// **Per-timestep** — called inside the simulation loop:
/// - `update` — advance the model by one timestep.
/// - `get_value` — retrieve output values via a type-erased `*mut c_void` buffer. The adapter
///   calls `get_var_nbytes` first to size the buffer, then passes the raw pointer to the single
///   C `get_value` function pointer; all three Rust variants (`get_value_f64`, `get_value_f32`,
///   `get_value_i32`) share this one pointer.
/// - `set_value` — push input values via a type-erased `*mut c_void`; same single pointer for
///   all three Rust typed variants.
///
/// **After `initialize`** (setup, not repeated per timestep):
/// - `get_time_units` — used to compute the internal `time_factor`.
/// - `get_input_var_names` / `get_output_var_names` (preceded by `get_input_item_count` /
///   `get_output_item_count` to allocate buffers) — together with `get_var_type` and
///   `get_var_itemsize` for each variable, to populate the type cache.
///
/// **On demand** (e.g. from the runner or `BmiExt`):
/// - `update_until`, `get_component_name`, `get_var_grid`, `get_var_units`, `get_var_nbytes`,
///   `get_var_location`, `get_current_time`, `get_start_time`, `get_end_time`, `get_time_step`,
///   `get_grid_rank`, `get_grid_size`, `get_grid_type`.
///
/// ## Functions NOT called
///
/// The C FFI struct contains optional pointers for `get_value_ptr`, `get_value_at_indices`,
/// `set_value_at_indices`, and all extended grid functions (`get_grid_shape`, `get_grid_x`, …).
/// None of these are wrapped by the Rust [`crate::traits::Bmi`] trait so the driver never calls
/// them, but they may be populated by the model library.
pub struct BmiC {
    name: String,
    _library: GlobalLibrary,
    bmi: Box<BmiStruct>,
    initialized: bool,
    time_factor: f64,
    type_cache: Option<HashMap<String, VarType>>,
}

impl BmiC {
    pub fn load(
        name: impl Into<String>,
        lib_path: impl AsRef<Path>,
        reg_func: &str,
    ) -> BmiResult<Self> {
        let name = name.into();
        let lib_path = lib_path.as_ref();

        let library =
            unsafe { GlobalLibrary::new(lib_path) }.map_err(|e| BmiError::LibraryLoad {
                path: lib_path.display().to_string(),
                source: e,
            })?;

        let register: unsafe extern "C" fn(*mut BmiStruct) -> *mut BmiStruct =
            unsafe { library.get_fn(reg_func) }.map_err(|e| BmiError::SymbolNotFound {
                func: reg_func.into(),
                source: e,
            })?;

        let mut bmi = Box::new(BmiStruct::default());
        unsafe {
            register(bmi.as_mut());
        }

        Ok(Self {
            name,
            _library: library,
            bmi,
            initialized: false,
            time_factor: 1.0,
            type_cache: None,
        })
    }

    fn ptr(&self) -> *mut BmiStruct {
        self.bmi.as_ref() as *const BmiStruct as *mut BmiStruct
    }

    fn call_string<F>(&self, func_name: &str, f: F) -> BmiResult<String>
    where
        F: FnOnce() -> Option<unsafe extern "C" fn(*mut BmiStruct, *mut c_char) -> i32>,
    {
        let func = f().ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: func_name.into(),
        })?;
        let mut buf = vec![0u8; BMI_MAX_NAME];
        if unsafe { func(self.ptr(), buf.as_mut_ptr() as *mut c_char) } != BMI_SUCCESS {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: func_name.into(),
            });
        }
        cstr_to_string(&buf)
    }

    fn call_int<F>(&self, func_name: &str, f: F) -> BmiResult<i32>
    where
        F: FnOnce() -> Option<unsafe extern "C" fn(*mut BmiStruct, *mut i32) -> i32>,
    {
        let func = f().ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: func_name.into(),
        })?;
        let mut val = 0i32;
        if unsafe { func(self.ptr(), &mut val) } != BMI_SUCCESS {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: func_name.into(),
            });
        }
        Ok(val)
    }

    fn call_double<F>(&self, func_name: &str, f: F) -> BmiResult<f64>
    where
        F: FnOnce() -> Option<unsafe extern "C" fn(*mut BmiStruct, *mut f64) -> i32>,
    {
        let func = f().ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: func_name.into(),
        })?;
        let mut val = 0.0f64;
        if unsafe { func(self.ptr(), &mut val) } != BMI_SUCCESS {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: func_name.into(),
            });
        }
        Ok(val)
    }

    fn var_string(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut BmiStruct, *const c_char, *mut c_char) -> i32>,
        name: &str,
    ) -> BmiResult<String> {
        let func = func.ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: func_name.into(),
        })?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut buf = vec![0u8; BMI_MAX_NAME];
        if unsafe { func(self.ptr(), cname.as_ptr(), buf.as_mut_ptr() as *mut c_char) }
            != BMI_SUCCESS
        {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: func_name.into(),
            });
        }
        cstr_to_string(&buf)
    }

    fn var_int(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut BmiStruct, *const c_char, *mut i32) -> i32>,
        name: &str,
    ) -> BmiResult<i32> {
        let func = func.ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: func_name.into(),
        })?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut val = 0i32;
        if unsafe { func(self.ptr(), cname.as_ptr(), &mut val) } != BMI_SUCCESS {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: func_name.into(),
            });
        }
        Ok(val)
    }

    fn get_var_names(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut BmiStruct, *mut *mut c_char) -> i32>,
        count: usize,
    ) -> BmiResult<Vec<String>> {
        let func = func.ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: func_name.into(),
        })?;
        let mut bufs: Vec<Vec<u8>> = (0..count).map(|_| vec![0u8; BMI_MAX_NAME]).collect();
        let mut ptrs: Vec<*mut c_char> = bufs
            .iter_mut()
            .map(|b| b.as_mut_ptr() as *mut c_char)
            .collect();
        if unsafe { func(self.ptr(), ptrs.as_mut_ptr()) } != BMI_SUCCESS {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: func_name.into(),
            });
        }
        bufs.iter().map(|b| cstr_to_string(b)).collect()
    }
}

impl Bmi for BmiC {
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
    fn time_factor(&self) -> f64 {
        self.time_factor
    }

    fn initialize(&mut self, config: &str) -> BmiResult<()> {
        if self.initialized {
            return Err(BmiError::AlreadyInitialized {
                model: self.name.clone(),
            });
        }
        if !Path::new(config).exists() {
            return Err(BmiError::ConfigNotFound {
                path: config.into(),
            });
        }

        let func = self
            .bmi
            .initialize
            .ok_or_else(|| BmiError::NotImplemented {
                model: self.name.clone(),
                func: "initialize".into(),
            })?;
        let cconfig = CString::new(config).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe { func(self.bmi.as_mut(), cconfig.as_ptr()) } != BMI_SUCCESS {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: "initialize".into(),
            });
        }

        self.initialized = true;
        self.time_factor = self
            .get_time_units()
            .map(|u| parse_time_units(&u))
            .unwrap_or(1.0);
        self.cache_types()?;
        Ok(())
    }

    fn update(&mut self) -> BmiResult<()> {
        check_initialized(self.initialized, &self.name)?;
        let func = self.bmi.update.ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: "update".into(),
        })?;
        if unsafe { func(self.bmi.as_mut()) } != BMI_SUCCESS {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: "update".into(),
            });
        }
        Ok(())
    }

    fn update_until(&mut self, time: f64) -> BmiResult<()> {
        check_initialized(self.initialized, &self.name)?;
        let func = self
            .bmi
            .update_until
            .ok_or_else(|| BmiError::NotImplemented {
                model: self.name.clone(),
                func: "update_until".into(),
            })?;
        if unsafe { func(self.bmi.as_mut(), time) } != BMI_SUCCESS {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: "update_until".into(),
            });
        }
        Ok(())
    }

    fn finalize(&mut self) -> BmiResult<()> {
        if !self.initialized {
            return Ok(());
        }
        let func = self.bmi.finalize.ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: "finalize".into(),
        })?;
        let result = unsafe { func(self.bmi.as_mut()) };
        self.initialized = false;
        if result != BMI_SUCCESS {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: "finalize".into(),
            });
        }
        Ok(())
    }

    fn get_component_name(&self) -> BmiResult<String> {
        self.call_string("get_component_name", || self.bmi.get_component_name)
    }

    fn get_input_item_count(&self) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.name)?;
        self.call_int("get_input_item_count", || self.bmi.get_input_item_count)
    }

    fn get_output_item_count(&self) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.name)?;
        self.call_int("get_output_item_count", || self.bmi.get_output_item_count)
    }

    fn get_input_var_names(&self) -> BmiResult<Vec<String>> {
        check_initialized(self.initialized, &self.name)?;
        let count = self.get_input_item_count()? as usize;
        self.get_var_names("get_input_var_names", self.bmi.get_input_var_names, count)
    }

    fn get_output_var_names(&self) -> BmiResult<Vec<String>> {
        check_initialized(self.initialized, &self.name)?;
        let count = self.get_output_item_count()? as usize;
        self.get_var_names("get_output_var_names", self.bmi.get_output_var_names, count)
    }

    fn get_var_grid(&self, name: &str) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.name)?;
        self.var_int("get_var_grid", self.bmi.get_var_grid, name)
    }

    fn get_var_type(&self, name: &str) -> BmiResult<String> {
        check_initialized(self.initialized, &self.name)?;
        self.var_string("get_var_type", self.bmi.get_var_type, name)
    }

    fn get_var_units(&self, name: &str) -> BmiResult<String> {
        check_initialized(self.initialized, &self.name)?;
        self.var_string("get_var_units", self.bmi.get_var_units, name)
    }

    fn get_var_itemsize(&self, name: &str) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.name)?;
        self.var_int("get_var_itemsize", self.bmi.get_var_itemsize, name)
    }

    fn get_var_nbytes(&self, name: &str) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.name)?;
        self.var_int("get_var_nbytes", self.bmi.get_var_nbytes, name)
    }

    fn get_var_location(&self, name: &str) -> BmiResult<String> {
        check_initialized(self.initialized, &self.name)?;
        self.var_string("get_var_location", self.bmi.get_var_location, name)
    }

    fn get_current_time(&self) -> BmiResult<f64> {
        check_initialized(self.initialized, &self.name)?;
        self.call_double("get_current_time", || self.bmi.get_current_time)
    }

    fn get_start_time(&self) -> BmiResult<f64> {
        check_initialized(self.initialized, &self.name)?;
        self.call_double("get_start_time", || self.bmi.get_start_time)
    }

    fn get_end_time(&self) -> BmiResult<f64> {
        check_initialized(self.initialized, &self.name)?;
        self.call_double("get_end_time", || self.bmi.get_end_time)
    }

    fn get_time_units(&self) -> BmiResult<String> {
        check_initialized(self.initialized, &self.name)?;
        self.call_string("get_time_units", || self.bmi.get_time_units)
    }

    fn get_time_step(&self) -> BmiResult<f64> {
        check_initialized(self.initialized, &self.name)?;
        self.call_double("get_time_step", || self.bmi.get_time_step)
    }

    fn get_value_f64(&self, name: &str) -> BmiResult<Vec<f64>> {
        check_initialized(self.initialized, &self.name)?;
        let func = self.bmi.get_value.ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: "get_value".into(),
        })?;
        let count = self.get_var_nbytes(name)? as usize / std::mem::size_of::<f64>();
        let mut vals = vec![0.0f64; count];
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe { func(self.ptr(), cname.as_ptr(), vals.as_mut_ptr() as *mut c_void) }
            != BMI_SUCCESS
        {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: "get_value".into(),
            });
        }
        Ok(vals)
    }

    fn get_value_f32(&self, name: &str) -> BmiResult<Vec<f32>> {
        check_initialized(self.initialized, &self.name)?;
        let func = self.bmi.get_value.ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: "get_value".into(),
        })?;
        let count = self.get_var_nbytes(name)? as usize / std::mem::size_of::<f32>();
        let mut vals = vec![0.0f32; count];
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe { func(self.ptr(), cname.as_ptr(), vals.as_mut_ptr() as *mut c_void) }
            != BMI_SUCCESS
        {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: "get_value".into(),
            });
        }
        Ok(vals)
    }

    fn get_value_i32(&self, name: &str) -> BmiResult<Vec<i32>> {
        check_initialized(self.initialized, &self.name)?;
        let func = self.bmi.get_value.ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: "get_value".into(),
        })?;
        let count = self.get_var_nbytes(name)? as usize / std::mem::size_of::<i32>();
        let mut vals = vec![0i32; count];
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe { func(self.ptr(), cname.as_ptr(), vals.as_mut_ptr() as *mut c_void) }
            != BMI_SUCCESS
        {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: "get_value".into(),
            });
        }
        Ok(vals)
    }

    fn set_value_f64(&mut self, name: &str, values: &[f64]) -> BmiResult<()> {
        check_initialized(self.initialized, &self.name)?;
        let func = self.bmi.set_value.ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: "set_value".into(),
        })?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe {
            func(
                self.bmi.as_mut(),
                cname.as_ptr(),
                values.as_ptr() as *mut c_void,
            )
        } != BMI_SUCCESS
        {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: "set_value".into(),
            });
        }
        Ok(())
    }

    fn set_value_f32(&mut self, name: &str, values: &[f32]) -> BmiResult<()> {
        check_initialized(self.initialized, &self.name)?;
        let func = self.bmi.set_value.ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: "set_value".into(),
        })?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe {
            func(
                self.bmi.as_mut(),
                cname.as_ptr(),
                values.as_ptr() as *mut c_void,
            )
        } != BMI_SUCCESS
        {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: "set_value".into(),
            });
        }
        Ok(())
    }

    fn set_value_i32(&mut self, name: &str, values: &[i32]) -> BmiResult<()> {
        check_initialized(self.initialized, &self.name)?;
        let func = self.bmi.set_value.ok_or_else(|| BmiError::NotImplemented {
            model: self.name.clone(),
            func: "set_value".into(),
        })?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe {
            func(
                self.bmi.as_mut(),
                cname.as_ptr(),
                values.as_ptr() as *mut c_void,
            )
        } != BMI_SUCCESS
        {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: "set_value".into(),
            });
        }
        Ok(())
    }

    fn get_grid_rank(&self, grid: i32) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.name)?;
        let func = self
            .bmi
            .get_grid_rank
            .ok_or_else(|| BmiError::NotImplemented {
                model: self.name.clone(),
                func: "get_grid_rank".into(),
            })?;
        let mut val = 0i32;
        if unsafe { func(self.ptr(), grid, &mut val) } != BMI_SUCCESS {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: "get_grid_rank".into(),
            });
        }
        Ok(val)
    }

    fn get_grid_size(&self, grid: i32) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.name)?;
        let func = self
            .bmi
            .get_grid_size
            .ok_or_else(|| BmiError::NotImplemented {
                model: self.name.clone(),
                func: "get_grid_size".into(),
            })?;
        let mut val = 0i32;
        if unsafe { func(self.ptr(), grid, &mut val) } != BMI_SUCCESS {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: "get_grid_size".into(),
            });
        }
        Ok(val)
    }

    fn get_grid_type(&self, grid: i32) -> BmiResult<String> {
        check_initialized(self.initialized, &self.name)?;
        let func = self
            .bmi
            .get_grid_type
            .ok_or_else(|| BmiError::NotImplemented {
                model: self.name.clone(),
                func: "get_grid_type".into(),
            })?;
        let mut buf = vec![0u8; BMI_MAX_NAME];
        if unsafe { func(self.ptr(), grid, buf.as_mut_ptr() as *mut c_char) } != BMI_SUCCESS {
            return Err(BmiError::FunctionFailed {
                model: self.name.clone(),
                func: "get_grid_type".into(),
            });
        }
        cstr_to_string(&buf)
    }
}

impl Drop for BmiC {
    fn drop(&mut self) {
        if self.initialized {
            let _ = self.finalize();
        }
    }
}
