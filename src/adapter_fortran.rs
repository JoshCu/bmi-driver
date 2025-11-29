//! Fortran BMI adapter implementation.
//!
//! This module provides a safe wrapper for BMI models that use the Fortran interface
//! (free functions with iso_c_binding that accept an opaque handle).
//!
//! Fortran BMI uses a different pattern than C BMI:
//! - The model library exports a creation function that returns an opaque handle
//! - A separate "middleware" library provides the BMI proxy functions
//! - These proxy functions accept a pointer to the opaque handle
//! - Typed getters/setters: get_value_int, get_value_float, get_value_double
//!
//! This matches the ngen architecture where `Bmi_Fortran_Common.h` declares the
//! extern free functions that are statically linked from the Fortran iso_c_binding
//! middleware library.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_double, c_float, c_int, c_void};
use std::path::Path;

use crate::bmi_ffi::{
    BMI_MAX_COMPONENT_NAME, BMI_MAX_LOCATION_NAME, BMI_MAX_TYPE_NAME, BMI_MAX_UNITS_NAME,
    BMI_MAX_VAR_NAME, BMI_SUCCESS,
};
use crate::error::{BmiError, BmiResult};
use crate::library::GlobalLibrary;
use crate::traits::Bmi;

// Function pointer types for Fortran BMI functions
// The handle is passed as `void*` in the C declaration, but the C++ code passes `&handle`
// This means we store the handle and pass a pointer to it.

type InitializeFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type UpdateFn = unsafe extern "C" fn(*mut c_void) -> c_int;
type UpdateUntilFn = unsafe extern "C" fn(*mut c_void, *mut c_double) -> c_int;
type FinalizeFn = unsafe extern "C" fn(*mut c_void) -> c_int;

type GetComponentNameFn = unsafe extern "C" fn(*mut c_void, *mut c_char) -> c_int;
type GetItemCountFn = unsafe extern "C" fn(*mut c_void, *mut c_int) -> c_int;
type GetVarNamesFn = unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> c_int;

type GetVarGridFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_int) -> c_int;
type GetVarStringFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_char) -> c_int;
type GetVarIntFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_int) -> c_int;

type GetTimeFn = unsafe extern "C" fn(*mut c_void, *mut c_double) -> c_int;
type GetTimeUnitsFn = unsafe extern "C" fn(*mut c_void, *mut c_char) -> c_int;

type GetValueIntFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_int) -> c_int;
type GetValueFloatFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_float) -> c_int;
type GetValueDoubleFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_double) -> c_int;

type SetValueIntFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_int) -> c_int;
type SetValueFloatFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_float) -> c_int;
type SetValueDoubleFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_double) -> c_int;

// Grid functions - grid ID is passed by pointer for Fortran
type GetGridIntFn = unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_int) -> c_int;
type GetGridTypeFn = unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_char) -> c_int;
type GetGridDoubleFn = unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_double) -> c_int;

/// Holds the function pointers loaded from a Fortran BMI library.
struct FortranBmiFunctions {
    initialize: InitializeFn,
    update: UpdateFn,
    update_until: UpdateUntilFn,
    finalize: FinalizeFn,

    get_component_name: GetComponentNameFn,
    get_input_item_count: GetItemCountFn,
    get_output_item_count: GetItemCountFn,
    get_input_var_names: GetVarNamesFn,
    get_output_var_names: GetVarNamesFn,

    get_var_grid: GetVarGridFn,
    get_var_type: GetVarStringFn,
    get_var_units: GetVarStringFn,
    get_var_itemsize: GetVarIntFn,
    get_var_nbytes: GetVarIntFn,
    get_var_location: GetVarStringFn,

    get_current_time: GetTimeFn,
    get_start_time: GetTimeFn,
    get_end_time: GetTimeFn,
    get_time_units: GetTimeUnitsFn,
    get_time_step: GetTimeFn,

    get_value_int: GetValueIntFn,
    get_value_float: GetValueFloatFn,
    get_value_double: GetValueDoubleFn,

    set_value_int: SetValueIntFn,
    set_value_float: SetValueFloatFn,
    set_value_double: SetValueDoubleFn,

    get_grid_rank: GetGridIntFn,
    get_grid_size: GetGridIntFn,
    get_grid_type: GetGridTypeFn,
    get_grid_shape: GetGridIntFn,
    get_grid_spacing: GetGridDoubleFn,
    get_grid_origin: GetGridDoubleFn,
}

/// A safe wrapper around a BMI Fortran model loaded from a shared library.
///
/// This adapter is for models that implement the Fortran BMI interface
/// using iso_c_binding with free functions that accept an opaque handle.
///
/// # Architecture
///
/// Unlike C BMI where all functions are in the model library, Fortran BMI
/// typically uses two libraries:
/// - **Model library**: Contains the Fortran BMI module and a creation function
/// - **Middleware library**: Contains the iso_c_binding proxy functions
///
/// The middleware functions accept a pointer to the opaque handle to match
/// Fortran's pass-by-reference semantics.
pub struct BmiFortran {
    model_name: String,
    _model_library: GlobalLibrary,
    _middleware_library: GlobalLibrary,
    funcs: FortranBmiFunctions,
    /// Opaque handle to the Fortran BMI object
    handle: *mut c_void,
    initialized: bool,
    time_convert_factor: f64,
}

// Helper macro to load a function with explicit type annotation
macro_rules! load_fn {
    ($library:expr, $name:expr, $type:ty) => {{
        let func: $type = unsafe { $library.get_fn($name) }.map_err(|e| {
            BmiError::RegistrationFunctionNotFound {
                func: $name.to_string(),
                source: e,
            }
        })?;
        func
    }};
}

impl BmiFortran {
    /// Load a BMI Fortran model from shared libraries.
    ///
    /// # Arguments
    ///
    /// * `model_name` - A descriptive name for this model (used in error messages)
    /// * `model_library_path` - Path to the model shared library (.so, .dylib)
    /// * `middleware_library_path` - Path to the Fortran BMI middleware library
    /// * `registration_func` - Name of the function that registers the BMI model
    ///
    /// The registration function has signature: `void* register_bmi(void* handle_ptr)`
    /// It takes a pointer to handle storage and sets `*handle_ptr` to point to the Fortran BMI object.
    pub fn load(
        model_name: impl Into<String>,
        model_library_path: impl AsRef<Path>,
        middleware_library_path: impl AsRef<Path>,
        registration_func: &str,
    ) -> BmiResult<Self> {
        let model_name = model_name.into();
        let model_library_path = model_library_path.as_ref();
        let middleware_library_path = middleware_library_path.as_ref();

        // Load the middleware library first (it provides the BMI proxy functions)
        let middleware_library =
            unsafe { GlobalLibrary::new(middleware_library_path) }.map_err(|e| {
                BmiError::LibraryLoad {
                    path: middleware_library_path.display().to_string(),
                    source: e,
                }
            })?;

        // Load the model library (it provides the registration function)
        let model_library = unsafe { GlobalLibrary::new(model_library_path) }.map_err(|e| {
            BmiError::LibraryLoad {
                path: model_library_path.display().to_string(),
                source: e,
            }
        })?;

        // Load the registration function from the MODEL library
        // Signature: void* register_bmi(void* handle_ptr)
        // It takes &handle and sets *handle_ptr to point to the Fortran BMI object
        let register_fn: unsafe extern "C" fn(*mut c_void) -> *mut c_void = unsafe {
            model_library.get_fn(registration_func)
        }
        .map_err(|e| BmiError::RegistrationFunctionNotFound {
            func: registration_func.to_string(),
            source: e,
        })?;

        // Initialize handle to null, then call register_bmi which will set it
        let mut handle: *mut c_void = std::ptr::null_mut();

        // Call registration function, passing &handle
        // The function will set handle to point to the Fortran BMI object
        unsafe {
            register_fn(&mut handle as *mut *mut c_void as *mut c_void);
        }

        // After registration, handle should be set to the Fortran BMI object
        if handle.is_null() {
            return Err(BmiError::BmiFunctionFailed {
                model: model_name,
                func: registration_func.to_string(),
            });
        }

        // Load all the BMI functions from the MIDDLEWARE library
        let funcs = Self::load_functions(&middleware_library)?;

        Ok(Self {
            model_name,
            _model_library: model_library,
            _middleware_library: middleware_library,
            funcs,
            handle,
            initialized: false,
            time_convert_factor: 1.0,
        })
    }

    /// Load a BMI Fortran model where the middleware functions are in the same library.
    ///
    /// Some Fortran BMI implementations bundle everything in a single library.
    /// This is a convenience function for that case.
    pub fn load_single_library(
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

        // Load the registration function
        // Signature: void* register_bmi(void* handle_ptr)
        let register_fn: unsafe extern "C" fn(*mut c_void) -> *mut c_void = unsafe {
            library.get_fn(registration_func)
        }
        .map_err(|e| BmiError::RegistrationFunctionNotFound {
            func: registration_func.to_string(),
            source: e,
        })?;

        // Initialize handle to null, then call register_bmi which will set it
        let mut handle: *mut c_void = std::ptr::null_mut();

        // Call registration function, passing &handle
        unsafe {
            register_fn(&mut handle as *mut *mut c_void as *mut c_void);
        }

        if handle.is_null() {
            return Err(BmiError::BmiFunctionFailed {
                model: model_name,
                func: registration_func.to_string(),
            });
        }

        // Load all the BMI functions from the same library
        let funcs = Self::load_functions(&library)?;

        // We need two library references to keep both alive, but they're the same
        let library2 =
            unsafe { GlobalLibrary::new(library_path) }.map_err(|e| BmiError::LibraryLoad {
                path: library_path.display().to_string(),
                source: e,
            })?;

        Ok(Self {
            model_name,
            _model_library: library,
            _middleware_library: library2,
            funcs,
            handle,
            initialized: false,
            time_convert_factor: 1.0,
        })
    }

    fn load_functions(library: &GlobalLibrary) -> BmiResult<FortranBmiFunctions> {
        Ok(FortranBmiFunctions {
            initialize: load_fn!(library, "initialize", InitializeFn),
            update: load_fn!(library, "update", UpdateFn),
            update_until: load_fn!(library, "update_until", UpdateUntilFn),
            finalize: load_fn!(library, "finalize", FinalizeFn),

            get_component_name: load_fn!(library, "get_component_name", GetComponentNameFn),
            get_input_item_count: load_fn!(library, "get_input_item_count", GetItemCountFn),
            get_output_item_count: load_fn!(library, "get_output_item_count", GetItemCountFn),
            get_input_var_names: load_fn!(library, "get_input_var_names", GetVarNamesFn),
            get_output_var_names: load_fn!(library, "get_output_var_names", GetVarNamesFn),

            get_var_grid: load_fn!(library, "get_var_grid", GetVarGridFn),
            get_var_type: load_fn!(library, "get_var_type", GetVarStringFn),
            get_var_units: load_fn!(library, "get_var_units", GetVarStringFn),
            get_var_itemsize: load_fn!(library, "get_var_itemsize", GetVarIntFn),
            get_var_nbytes: load_fn!(library, "get_var_nbytes", GetVarIntFn),
            get_var_location: load_fn!(library, "get_var_location", GetVarStringFn),

            get_current_time: load_fn!(library, "get_current_time", GetTimeFn),
            get_start_time: load_fn!(library, "get_start_time", GetTimeFn),
            get_end_time: load_fn!(library, "get_end_time", GetTimeFn),
            get_time_units: load_fn!(library, "get_time_units", GetTimeUnitsFn),
            get_time_step: load_fn!(library, "get_time_step", GetTimeFn),

            get_value_int: load_fn!(library, "get_value_int", GetValueIntFn),
            get_value_float: load_fn!(library, "get_value_float", GetValueFloatFn),
            get_value_double: load_fn!(library, "get_value_double", GetValueDoubleFn),

            set_value_int: load_fn!(library, "set_value_int", SetValueIntFn),
            set_value_float: load_fn!(library, "set_value_float", SetValueFloatFn),
            set_value_double: load_fn!(library, "set_value_double", SetValueDoubleFn),

            get_grid_rank: load_fn!(library, "get_grid_rank", GetGridIntFn),
            get_grid_size: load_fn!(library, "get_grid_size", GetGridIntFn),
            get_grid_type: load_fn!(library, "get_grid_type", GetGridTypeFn),
            get_grid_shape: load_fn!(library, "get_grid_shape", GetGridIntFn),
            get_grid_spacing: load_fn!(library, "get_grid_spacing", GetGridDoubleFn),
            get_grid_origin: load_fn!(library, "get_grid_origin", GetGridDoubleFn),
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

    /// Get a pointer to pass to Fortran functions.
    /// The C++ code passes `&bmi_model->handle` which is a pointer to the handle storage.
    #[inline]
    fn handle_ptr(&self) -> *mut c_void {
        // Cast the address of our handle to *mut c_void
        // This matches the C++ pattern: &bmi_model->handle
        &self.handle as *const *mut c_void as *mut c_void
    }

    /// Get a mutable pointer to pass to Fortran functions.
    #[inline]
    fn handle_ptr_mut(&mut self) -> *mut c_void {
        &mut self.handle as *mut *mut c_void as *mut c_void
    }
}

impl Bmi for BmiFortran {
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
        let result =
            unsafe { (self.funcs.initialize)(self.handle_ptr_mut(), config_cstr.as_ptr()) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "initialize".to_string(),
            });
        }

        self.initialized = true;
        self.time_convert_factor = self.calculate_time_convert_factor();

        Ok(())
    }

    fn update(&mut self) -> BmiResult<()> {
        self.require_initialized()?;

        let result = unsafe { (self.funcs.update)(self.handle_ptr_mut()) };
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

        let mut time_val = time;
        let result = unsafe { (self.funcs.update_until)(self.handle_ptr_mut(), &mut time_val) };
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

        let result = unsafe { (self.funcs.finalize)(self.handle_ptr_mut()) };
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
        let mut buffer = vec![0u8; BMI_MAX_COMPONENT_NAME];
        let result = unsafe {
            (self.funcs.get_component_name)(self.handle_ptr(), buffer.as_mut_ptr() as *mut c_char)
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_component_name".to_string(),
            });
        }

        cstr_to_string(&buffer)
    }

    fn get_input_item_count(&self) -> BmiResult<i32> {
        self.require_initialized()?;

        let mut count: c_int = 0;
        let result = unsafe { (self.funcs.get_input_item_count)(self.handle_ptr(), &mut count) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_input_item_count".to_string(),
            });
        }

        Ok(count)
    }

    fn get_output_item_count(&self) -> BmiResult<i32> {
        self.require_initialized()?;

        let mut count: c_int = 0;
        let result = unsafe { (self.funcs.get_output_item_count)(self.handle_ptr(), &mut count) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_output_item_count".to_string(),
            });
        }

        Ok(count)
    }

    fn get_input_var_names(&self) -> BmiResult<Vec<String>> {
        self.require_initialized()?;
        let count = self.get_input_item_count()? as usize;

        let mut buffers: Vec<Vec<u8>> = (0..count).map(|_| vec![0u8; BMI_MAX_VAR_NAME]).collect();
        let mut ptrs: Vec<*mut c_char> = buffers
            .iter_mut()
            .map(|b| b.as_mut_ptr() as *mut c_char)
            .collect();

        let result =
            unsafe { (self.funcs.get_input_var_names)(self.handle_ptr(), ptrs.as_mut_ptr()) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_input_var_names".to_string(),
            });
        }

        buffers.iter().map(|b| cstr_to_string(b)).collect()
    }

    fn get_output_var_names(&self) -> BmiResult<Vec<String>> {
        self.require_initialized()?;
        let count = self.get_output_item_count()? as usize;

        let mut buffers: Vec<Vec<u8>> = (0..count).map(|_| vec![0u8; BMI_MAX_VAR_NAME]).collect();
        let mut ptrs: Vec<*mut c_char> = buffers
            .iter_mut()
            .map(|b| b.as_mut_ptr() as *mut c_char)
            .collect();

        let result =
            unsafe { (self.funcs.get_output_var_names)(self.handle_ptr(), ptrs.as_mut_ptr()) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_output_var_names".to_string(),
            });
        }

        buffers.iter().map(|b| cstr_to_string(b)).collect()
    }

    fn get_var_grid(&self, name: &str) -> BmiResult<i32> {
        self.require_initialized()?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut grid: c_int = 0;
        let result =
            unsafe { (self.funcs.get_var_grid)(self.handle_ptr(), name_cstr.as_ptr(), &mut grid) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_var_grid".to_string(),
            });
        }

        Ok(grid)
    }

    fn get_var_type(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut buffer = vec![0u8; BMI_MAX_TYPE_NAME];
        let result = unsafe {
            (self.funcs.get_var_type)(
                self.handle_ptr(),
                name_cstr.as_ptr(),
                buffer.as_mut_ptr() as *mut c_char,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_var_type".to_string(),
            });
        }

        cstr_to_string(&buffer)
    }

    fn get_var_units(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut buffer = vec![0u8; BMI_MAX_UNITS_NAME];
        let result = unsafe {
            (self.funcs.get_var_units)(
                self.handle_ptr(),
                name_cstr.as_ptr(),
                buffer.as_mut_ptr() as *mut c_char,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_var_units".to_string(),
            });
        }

        cstr_to_string(&buffer)
    }

    fn get_var_itemsize(&self, name: &str) -> BmiResult<i32> {
        self.require_initialized()?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut size: c_int = 0;
        let result = unsafe {
            (self.funcs.get_var_itemsize)(self.handle_ptr(), name_cstr.as_ptr(), &mut size)
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_var_itemsize".to_string(),
            });
        }

        Ok(size)
    }

    fn get_var_nbytes(&self, name: &str) -> BmiResult<i32> {
        self.require_initialized()?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut nbytes: c_int = 0;
        let result = unsafe {
            (self.funcs.get_var_nbytes)(self.handle_ptr(), name_cstr.as_ptr(), &mut nbytes)
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_var_nbytes".to_string(),
            });
        }

        Ok(nbytes)
    }

    fn get_var_location(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut buffer = vec![0u8; BMI_MAX_LOCATION_NAME];
        let result = unsafe {
            (self.funcs.get_var_location)(
                self.handle_ptr(),
                name_cstr.as_ptr(),
                buffer.as_mut_ptr() as *mut c_char,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_var_location".to_string(),
            });
        }

        cstr_to_string(&buffer)
    }

    fn get_current_time(&self) -> BmiResult<f64> {
        self.require_initialized()?;

        let mut time: c_double = 0.0;
        let result = unsafe { (self.funcs.get_current_time)(self.handle_ptr(), &mut time) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_current_time".to_string(),
            });
        }

        Ok(time)
    }

    fn get_start_time(&self) -> BmiResult<f64> {
        self.require_initialized()?;

        let mut time: c_double = 0.0;
        let result = unsafe { (self.funcs.get_start_time)(self.handle_ptr(), &mut time) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_start_time".to_string(),
            });
        }

        Ok(time)
    }

    fn get_end_time(&self) -> BmiResult<f64> {
        self.require_initialized()?;

        let mut time: c_double = 0.0;
        let result = unsafe { (self.funcs.get_end_time)(self.handle_ptr(), &mut time) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_end_time".to_string(),
            });
        }

        Ok(time)
    }

    fn get_time_units(&self) -> BmiResult<String> {
        self.require_initialized()?;

        let mut buffer = vec![0u8; BMI_MAX_UNITS_NAME];
        let result = unsafe {
            (self.funcs.get_time_units)(self.handle_ptr(), buffer.as_mut_ptr() as *mut c_char)
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_time_units".to_string(),
            });
        }

        cstr_to_string(&buffer)
    }

    fn get_time_step(&self) -> BmiResult<f64> {
        self.require_initialized()?;

        let mut ts: c_double = 0.0;
        let result = unsafe { (self.funcs.get_time_step)(self.handle_ptr(), &mut ts) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_time_step".to_string(),
            });
        }

        Ok(ts)
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

        let nbytes = self.get_var_nbytes(name)? as usize;
        let count = nbytes / std::mem::size_of::<f64>();
        let mut values = vec![0.0f64; count];

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = unsafe {
            (self.funcs.get_value_double)(
                self.handle_ptr(),
                name_cstr.as_ptr(),
                values.as_mut_ptr(),
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_value_double".to_string(),
            });
        }

        Ok(values)
    }

    fn get_value_f32(&self, name: &str) -> BmiResult<Vec<f32>> {
        self.require_initialized()?;

        let nbytes = self.get_var_nbytes(name)? as usize;
        let count = nbytes / std::mem::size_of::<f32>();
        let mut values = vec![0.0f32; count];

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = unsafe {
            (self.funcs.get_value_float)(self.handle_ptr(), name_cstr.as_ptr(), values.as_mut_ptr())
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_value_float".to_string(),
            });
        }

        Ok(values)
    }

    fn get_value_i32(&self, name: &str) -> BmiResult<Vec<i32>> {
        self.require_initialized()?;

        let nbytes = self.get_var_nbytes(name)? as usize;
        let count = nbytes / std::mem::size_of::<i32>();
        let mut values = vec![0i32; count];

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = unsafe {
            (self.funcs.get_value_int)(self.handle_ptr(), name_cstr.as_ptr(), values.as_mut_ptr())
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_value_int".to_string(),
            });
        }

        Ok(values)
    }

    fn get_value_at_indices_f64(&self, _name: &str, _indices: &[i32]) -> BmiResult<Vec<f64>> {
        Err(BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: "get_value_at_indices (not supported for Fortran BMI)".to_string(),
        })
    }

    fn set_value_f64(&mut self, name: &str, values: &[f64]) -> BmiResult<()> {
        self.require_initialized()?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = unsafe {
            (self.funcs.set_value_double)(
                self.handle_ptr_mut(),
                name_cstr.as_ptr(),
                values.as_ptr() as *mut c_double,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "set_value_double".to_string(),
            });
        }

        Ok(())
    }

    fn set_value_f32(&mut self, name: &str, values: &[f32]) -> BmiResult<()> {
        self.require_initialized()?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = unsafe {
            (self.funcs.set_value_float)(
                self.handle_ptr_mut(),
                name_cstr.as_ptr(),
                values.as_ptr() as *mut c_float,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "set_value_float".to_string(),
            });
        }

        Ok(())
    }

    fn set_value_i32(&mut self, name: &str, values: &[i32]) -> BmiResult<()> {
        self.require_initialized()?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = unsafe {
            (self.funcs.set_value_int)(
                self.handle_ptr_mut(),
                name_cstr.as_ptr(),
                values.as_ptr() as *mut c_int,
            )
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "set_value_int".to_string(),
            });
        }

        Ok(())
    }

    fn set_value_at_indices_f64(
        &mut self,
        _name: &str,
        _indices: &[i32],
        _values: &[f64],
    ) -> BmiResult<()> {
        Err(BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: "set_value_at_indices (not supported for Fortran BMI)".to_string(),
        })
    }

    fn get_grid_rank(&self, grid: i32) -> BmiResult<i32> {
        self.require_initialized()?;

        let mut grid_id = grid;
        let mut rank: c_int = 0;
        let result =
            unsafe { (self.funcs.get_grid_rank)(self.handle_ptr(), &mut grid_id, &mut rank) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_grid_rank".to_string(),
            });
        }

        Ok(rank)
    }

    fn get_grid_size(&self, grid: i32) -> BmiResult<i32> {
        self.require_initialized()?;

        let mut grid_id = grid;
        let mut size: c_int = 0;
        let result =
            unsafe { (self.funcs.get_grid_size)(self.handle_ptr(), &mut grid_id, &mut size) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_grid_size".to_string(),
            });
        }

        Ok(size)
    }

    fn get_grid_type(&self, grid: i32) -> BmiResult<String> {
        self.require_initialized()?;

        let mut grid_id = grid;
        let mut buffer = vec![0u8; BMI_MAX_TYPE_NAME];
        let result = unsafe {
            (self.funcs.get_grid_type)(
                self.handle_ptr(),
                &mut grid_id,
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

        let mut grid_id = grid;
        let mut shape = vec![0i32; rank];
        let result = unsafe {
            (self.funcs.get_grid_shape)(self.handle_ptr(), &mut grid_id, shape.as_mut_ptr())
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_grid_shape".to_string(),
            });
        }

        Ok(shape)
    }

    fn get_grid_spacing(&self, grid: i32) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;
        let rank = self.get_grid_rank(grid)? as usize;

        let mut grid_id = grid;
        let mut spacing = vec![0.0f64; rank];
        let result = unsafe {
            (self.funcs.get_grid_spacing)(self.handle_ptr(), &mut grid_id, spacing.as_mut_ptr())
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_grid_spacing".to_string(),
            });
        }

        Ok(spacing)
    }

    fn get_grid_origin(&self, grid: i32) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;
        let rank = self.get_grid_rank(grid)? as usize;

        let mut grid_id = grid;
        let mut origin = vec![0.0f64; rank];
        let result = unsafe {
            (self.funcs.get_grid_origin)(self.handle_ptr(), &mut grid_id, origin.as_mut_ptr())
        };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_grid_origin".to_string(),
            });
        }

        Ok(origin)
    }
}

impl Drop for BmiFortran {
    fn drop(&mut self) {
        if self.initialized {
            let _ = self.finalize();
        }
    }
}

// Helper function
fn cstr_to_string(buffer: &[u8]) -> BmiResult<String> {
    let cstr = unsafe { CStr::from_ptr(buffer.as_ptr() as *const c_char) };
    cstr.to_str()
        .map(|s| s.to_string())
        .map_err(|_| BmiError::InvalidUtf8)
}
