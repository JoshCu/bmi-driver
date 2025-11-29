//! Safe Rust wrapper for BMI C libraries.
//!
//! This module provides a safe, ergonomic interface for loading and interacting
//! with BMI-compliant models compiled as shared libraries.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;
use std::ptr;

// Custom library wrapper that uses RTLD_GLOBAL
struct GlobalLibrary {
    handle: *mut c_void,
}

/// Error type for library loading operations
#[derive(Debug)]
pub struct DlError(String);

impl std::fmt::Display for DlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for DlError {}

impl From<DlError> for libloading::Error {
    fn from(e: DlError) -> Self {
        libloading::Error::DlOpenUnknown
    }
}

extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

// RTLD_NOW | RTLD_GLOBAL
#[cfg(target_os = "linux")]
const RTLD_FLAGS: c_int = 0x2 | 0x100;

#[cfg(target_os = "macos")]
const RTLD_FLAGS: c_int = 0x2 | 0x8;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const RTLD_FLAGS: c_int = 0x2;

impl GlobalLibrary {
    unsafe fn new(path: &Path) -> Result<Self, DlError> {
        let path_cstr = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| DlError("invalid path".to_string()))?;
        
        // Clear any previous error
        let _ = dlerror();
        
        let handle = dlopen(path_cstr.as_ptr(), RTLD_FLAGS);
        if handle.is_null() {
            let err = dlerror();
            let msg = if err.is_null() {
                "unknown dlopen error".to_string()
            } else {
                CStr::from_ptr(err).to_string_lossy().into_owned()
            };
            return Err(DlError(msg));
        }
        
        Ok(Self { handle })
    }
    
    unsafe fn get_fn<T>(&self, symbol: &str) -> Result<T, DlError> {
        // Clear any previous error
        let _ = dlerror();
        
        let symbol_cstr = CString::new(symbol)
            .map_err(|_| DlError("invalid symbol name".to_string()))?;
        
        let ptr = dlsym(self.handle, symbol_cstr.as_ptr());
        
        let err = dlerror();
        if !err.is_null() {
            let msg = CStr::from_ptr(err).to_string_lossy().into_owned();
            return Err(DlError(msg));
        }
        
        if ptr.is_null() {
            return Err(DlError(format!("symbol '{}' not found", symbol)));
        }
        
        // Transmute the void pointer to the function type
        Ok(std::mem::transmute_copy(&ptr))
    }
}

impl Drop for GlobalLibrary {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                dlclose(self.handle);
            }
        }
    }
}

// Library handles are thread-safe for our use
unsafe impl Send for GlobalLibrary {}
unsafe impl Sync for GlobalLibrary {}

use crate::bmi_ffi::{
    Bmi, BmiRegistrationFn, BMI_MAX_COMPONENT_NAME, BMI_MAX_LOCATION_NAME, BMI_MAX_TYPE_NAME,
    BMI_MAX_UNITS_NAME, BMI_MAX_VAR_NAME, BMI_SUCCESS,
};
use crate::error::{BmiError, BmiResult};

/// Preload common dependency libraries (libm, libc, etc.) with RTLD_GLOBAL.
/// Call this before loading BMI models that depend on these libraries.
pub fn preload_dependencies() -> BmiResult<()> {
    #[cfg(target_os = "linux")]
    {
        let libs = [
            "libm.so.6",
            "libm.so",
        ];
        
        for lib in &libs {
            if preload_library(lib).is_ok() {
                return Ok(());
            }
        }
    }
    
    #[cfg(target_os = "macos")]
    {
        // macOS typically doesn't need this, but just in case
        let _ = preload_library("libm.dylib");
    }
    
    Ok(())
}

/// Preload a specific library with RTLD_GLOBAL.
fn preload_library(name: &str) -> BmiResult<()> {
    let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
    
    // Clear previous error
    unsafe { let _ = dlerror(); }
    
    let handle = unsafe { dlopen(name_cstr.as_ptr(), RTLD_FLAGS) };
    
    if handle.is_null() {
        Err(BmiError::LibraryLoad {
            path: name.to_string(),
            source: libloading::Error::DlOpenUnknown,
        })
    } else {
        // Don't close it - we want it to stay loaded
        Ok(())
    }
}

/// A safe wrapper around a BMI C model loaded from a shared library.
///
/// # Example
///
/// ```no_run
/// use bmi::BmiModel;
///
/// let mut model = BmiModel::load(
///     "my_model",
///     "/path/to/libmodel.so",
///     "register_bmi_model",
/// )?;
///
/// model.initialize("/path/to/config.yml")?;
///
/// println!("Model: {}", model.get_component_name()?);
/// println!("Time: {}", model.get_current_time()?);
///
/// while model.get_current_time()? < model.get_end_time()? {
///     model.update()?;
/// }
///
/// model.finalize()?;
/// # Ok::<(), bmi::BmiError>(())
/// ```
pub struct BmiModel {
    /// Name for this model instance (used in error messages)
    model_name: String,
    /// The dynamically loaded library (kept alive for function pointers)
    _library: GlobalLibrary,
    /// The BMI struct with function pointers
    bmi: Box<Bmi>,
    /// Whether the model has been initialized
    initialized: bool,
    /// Conversion factor from model time units to seconds
    time_convert_factor: f64,
}

impl BmiModel {
    /// Load a BMI model from a shared library.
    ///
    /// # Arguments
    ///
    /// * `model_name` - A descriptive name for this model (used in error messages)
    /// * `library_path` - Path to the shared library (.so, .dylib, or .dll)
    /// * `registration_func` - Name of the function that registers BMI function pointers
    ///
    /// # Returns
    ///
    /// A new `BmiModel` instance (not yet initialized)
    pub fn load(
        model_name: impl Into<String>,
        library_path: impl AsRef<Path>,
        registration_func: &str,
    ) -> BmiResult<Self> {
        let model_name = model_name.into();
        let library_path = library_path.as_ref();

        // Load the shared library with RTLD_GLOBAL to expose symbols like libm
        let library = unsafe { GlobalLibrary::new(library_path) }.map_err(|e| BmiError::LibraryLoad {
            path: library_path.display().to_string(),
            source: libloading::Error::DlOpenUnknown,
        })?;

        // Get the registration function
        let register_fn: BmiRegistrationFn =
            unsafe { library.get_fn(registration_func) }.map_err(|e| {
                BmiError::RegistrationFunctionNotFound {
                    func: registration_func.to_string(),
                    source: libloading::Error::DlSymUnknown,
                }
            })?;

        // Create and register the BMI struct
        let mut bmi = Box::new(Bmi::default());
        unsafe {
            register_fn(bmi.as_mut());
        }

        Ok(Self {
            model_name,
            _library: library,
            bmi,
            initialized: false,
            time_convert_factor: 1.0, // Will be updated after initialization
        })
    }

    /// Get the model name.
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Check if the model has been initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    // =========================================================================
    // Initialize, Run, Finalize (IRF)
    // =========================================================================

    /// Initialize the model with a configuration file.
    pub fn initialize(&mut self, config_file: impl AsRef<Path>) -> BmiResult<()> {
        if self.initialized {
            return Err(BmiError::AlreadyInitialized {
                model: self.model_name.clone(),
            });
        }

        let config_path = config_file.as_ref();
        if !config_path.exists() {
            return Err(BmiError::ConfigFileNotFound {
                path: config_path.display().to_string(),
            });
        }

        let config_cstr = path_to_cstring(config_path);
        let func = self.bmi.initialize.ok_or_else(|| BmiError::FunctionNotImplemented {
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
        
        // Calculate time conversion factor after initialization
        self.time_convert_factor = self.calculate_time_convert_factor();
        
        Ok(())
    }
    
    /// Calculate the conversion factor from model time units to seconds.
    /// 
    /// This parses the time units string from the model and returns a factor
    /// that converts model time to seconds.
    fn calculate_time_convert_factor(&self) -> f64 {
        let time_units = match self.get_time_units() {
            Ok(units) => units.to_lowercase(),
            Err(_) => return 1.0, // Default to 1.0 if we can't get units
        };
        
        // Parse common time unit strings
        // Based on UDUNITS conventions used by BMI models
        let factor = match time_units.trim() {
            // Seconds
            "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
            
            // Minutes  
            "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
            
            // Hours
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
            
            // Days
            "d" | "day" | "days" => 86400.0,
            
            // Weeks
            "week" | "weeks" => 604800.0,
            
            // Default: assume seconds
            _ => {
                // Try to parse more complex unit strings
                if time_units.contains("second") || time_units.contains(" s") || time_units.ends_with(" s") {
                    1.0
                } else if time_units.contains("minute") || time_units.contains(" min") {
                    60.0
                } else if time_units.contains("hour") || time_units.contains(" h") || time_units.contains(" hr") {
                    3600.0
                } else if time_units.contains("day") || time_units.contains(" d") {
                    86400.0
                } else {
                    // Unknown units, assume seconds
                    1.0
                }
            }
        };
        
        factor
    }
    
    /// Convert a model time value to seconds.
    pub fn convert_model_time_to_seconds(&self, model_time: f64) -> f64 {
        model_time * self.time_convert_factor
    }
    
    /// Convert seconds to model time units.
    pub fn convert_seconds_to_model_time(&self, seconds: f64) -> f64 {
        seconds / self.time_convert_factor
    }
    
    /// Get the time conversion factor (model time units to seconds).
    pub fn get_time_convert_factor(&self) -> f64 {
        self.time_convert_factor
    }

    /// Advance the model by one time step.
    pub fn update(&mut self) -> BmiResult<()> {
        self.require_initialized()?;

        let func = self.bmi.update.ok_or_else(|| BmiError::FunctionNotImplemented {
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

    /// Advance the model to a specific time.
    pub fn update_until(&mut self, time: f64) -> BmiResult<()> {
        self.require_initialized()?;

        let func = self.bmi.update_until.ok_or_else(|| BmiError::FunctionNotImplemented {
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
    
    /// Advance the model by a duration specified in seconds.
    /// 
    /// This method handles the conversion from seconds to model time units,
    /// and chooses between Update() and UpdateUntil() based on whether the
    /// duration matches the model's native time step.
    /// 
    /// This mirrors the behavior of ngen's Bmi_Module_Formulation::get_response().
    pub fn update_for_duration_seconds(&mut self, duration_seconds: f64) -> BmiResult<()> {
        self.require_initialized()?;
        
        let duration_model_units = self.convert_seconds_to_model_time(duration_seconds);
        let model_time_step = self.get_time_step()?;
        let current_time = self.get_current_time()?;
        
        // Use Update() if duration matches model time step, otherwise UpdateUntil()
        if (duration_model_units - model_time_step).abs() < 1e-10 {
            self.update()
        } else {
            self.update_until(current_time + duration_model_units)
        }
    }

    /// Finalize and clean up the model.
    pub fn finalize(&mut self) -> BmiResult<()> {
        if !self.initialized {
            return Ok(()); // Nothing to finalize
        }

        let func = self.bmi.finalize.ok_or_else(|| BmiError::FunctionNotImplemented {
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

    // =========================================================================
    // Model Information
    // =========================================================================

    /// Get the name of the model component.
    pub fn get_component_name(&self) -> BmiResult<String> {
        self.get_string_field("get_component_name", self.bmi.get_component_name, BMI_MAX_COMPONENT_NAME)
    }

    /// Get the number of input variables.
    pub fn get_input_item_count(&self) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_field("get_input_item_count", self.bmi.get_input_item_count)
    }

    /// Get the number of output variables.
    pub fn get_output_item_count(&self) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_field("get_output_item_count", self.bmi.get_output_item_count)
    }

    /// Get the names of all input variables.
    pub fn get_input_var_names(&self) -> BmiResult<Vec<String>> {
        self.require_initialized()?;
        let count = self.get_input_item_count()? as usize;
        self.get_var_names("get_input_var_names", self.bmi.get_input_var_names, count)
    }

    /// Get the names of all output variables.
    pub fn get_output_var_names(&self) -> BmiResult<Vec<String>> {
        self.require_initialized()?;
        let count = self.get_output_item_count()? as usize;
        self.get_var_names("get_output_var_names", self.bmi.get_output_var_names, count)
    }

    // =========================================================================
    // Variable Information
    // =========================================================================

    /// Get the grid ID for a variable.
    pub fn get_var_grid(&self, name: &str) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_for_var("get_var_grid", self.bmi.get_var_grid, name)
    }

    /// Get the data type of a variable.
    pub fn get_var_type(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;
        self.get_string_for_var("get_var_type", self.bmi.get_var_type, name, BMI_MAX_TYPE_NAME)
    }

    /// Get the units of a variable.
    pub fn get_var_units(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;
        self.get_string_for_var("get_var_units", self.bmi.get_var_units, name, BMI_MAX_UNITS_NAME)
    }

    /// Get the size in bytes of one element of a variable.
    pub fn get_var_itemsize(&self, name: &str) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_for_var("get_var_itemsize", self.bmi.get_var_itemsize, name)
    }

    /// Get the total size in bytes of a variable.
    pub fn get_var_nbytes(&self, name: &str) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_for_var("get_var_nbytes", self.bmi.get_var_nbytes, name)
    }

    /// Get the location of a variable on the grid.
    pub fn get_var_location(&self, name: &str) -> BmiResult<String> {
        self.require_initialized()?;
        self.get_string_for_var("get_var_location", self.bmi.get_var_location, name, BMI_MAX_LOCATION_NAME)
    }

    // =========================================================================
    // Time Information
    // =========================================================================

    /// Get the current model time.
    pub fn get_current_time(&self) -> BmiResult<f64> {
        self.require_initialized()?;
        self.get_double_field("get_current_time", self.bmi.get_current_time)
    }

    /// Get the start time of the model.
    pub fn get_start_time(&self) -> BmiResult<f64> {
        self.require_initialized()?;
        self.get_double_field("get_start_time", self.bmi.get_start_time)
    }

    /// Get the end time of the model.
    pub fn get_end_time(&self) -> BmiResult<f64> {
        self.require_initialized()?;
        self.get_double_field("get_end_time", self.bmi.get_end_time)
    }

    /// Get the time units used by the model.
    pub fn get_time_units(&self) -> BmiResult<String> {
        self.require_initialized()?;
        self.get_string_field("get_time_units", self.bmi.get_time_units, BMI_MAX_UNITS_NAME)
    }

    /// Get the time step size of the model.
    pub fn get_time_step(&self) -> BmiResult<f64> {
        self.require_initialized()?;
        self.get_double_field("get_time_step", self.bmi.get_time_step)
    }

    // =========================================================================
    // Getters
    // =========================================================================

    /// Get a copy of the values of a variable.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `dest` points to a buffer large enough
    /// to hold the variable data (see `get_var_nbytes`).
    pub unsafe fn get_value_raw(&self, name: &str, dest: *mut c_void) -> BmiResult<()> {
        self.require_initialized()?;

        let func = self.bmi.get_value.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: "get_value".to_string(),
        })?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = func(self.bmi.as_ref() as *const Bmi as *mut Bmi, name_cstr.as_ptr(), dest);

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_value".to_string(),
            });
        }
        Ok(())
    }

    /// Get values of a variable as a Vec of the specified type.
    pub fn get_value<T: Copy + Default>(&self, name: &str) -> BmiResult<Vec<T>> {
        self.require_initialized()?;

        let nbytes = self.get_var_nbytes(name)? as usize;
        let item_size = std::mem::size_of::<T>();
        let count = nbytes / item_size;

        let mut values = vec![T::default(); count];
        unsafe {
            self.get_value_raw(name, values.as_mut_ptr() as *mut c_void)?;
        }
        Ok(values)
    }

    /// Get a pointer to the values of a variable (for direct memory access).
    ///
    /// # Safety
    ///
    /// The returned pointer is owned by the model and may be invalidated
    /// by subsequent model operations. Use with caution.
    pub unsafe fn get_value_ptr(&self, name: &str) -> BmiResult<*mut c_void> {
        self.require_initialized()?;

        let func = self.bmi.get_value_ptr.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: "get_value_ptr".to_string(),
        })?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut dest: *mut c_void = ptr::null_mut();
        let result = func(
            self.bmi.as_ref() as *const Bmi as *mut Bmi,
            name_cstr.as_ptr(),
            &mut dest,
        );

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_value_ptr".to_string(),
            });
        }
        Ok(dest)
    }

    /// Get values at specific indices.
    pub fn get_value_at_indices<T: Copy + Default>(
        &self,
        name: &str,
        indices: &[i32],
    ) -> BmiResult<Vec<T>> {
        self.require_initialized()?;

        let func = self.bmi.get_value_at_indices.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: "get_value_at_indices".to_string(),
        })?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut values = vec![T::default(); indices.len()];

        let result = unsafe {
            func(
                self.bmi.as_ref() as *const Bmi as *mut Bmi,
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

    // =========================================================================
    // Setters
    // =========================================================================

    /// Set the values of a variable.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `src` points to data of the correct type
    /// and size for the variable.
    pub unsafe fn set_value_raw(&mut self, name: &str, src: *mut c_void) -> BmiResult<()> {
        self.require_initialized()?;

        let func = self.bmi.set_value.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: "set_value".to_string(),
        })?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let result = func(self.bmi.as_mut(), name_cstr.as_ptr(), src);

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "set_value".to_string(),
            });
        }
        Ok(())
    }

    /// Set the values of a variable from a slice.
    pub fn set_value<T: Copy>(&mut self, name: &str, values: &[T]) -> BmiResult<()> {
        self.require_initialized()?;
        unsafe { self.set_value_raw(name, values.as_ptr() as *mut c_void) }
    }

    /// Set values at specific indices.
    pub fn set_value_at_indices<T: Copy>(
        &mut self,
        name: &str,
        indices: &[i32],
        values: &[T],
    ) -> BmiResult<()> {
        self.require_initialized()?;

        let func = self.bmi.set_value_at_indices.ok_or_else(|| BmiError::FunctionNotImplemented {
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

    // =========================================================================
    // Grid Information
    // =========================================================================

    /// Get the rank (number of dimensions) of a grid.
    pub fn get_grid_rank(&self, grid: i32) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_for_grid("get_grid_rank", self.bmi.get_grid_rank, grid)
    }

    /// Get the total number of elements in a grid.
    pub fn get_grid_size(&self, grid: i32) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_for_grid("get_grid_size", self.bmi.get_grid_size, grid)
    }

    /// Get the type of a grid.
    pub fn get_grid_type(&self, grid: i32) -> BmiResult<String> {
        self.require_initialized()?;

        let func = self.bmi.get_grid_type.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: "get_grid_type".to_string(),
        })?;

        let mut buffer = vec![0u8; BMI_MAX_TYPE_NAME];
        let result = unsafe { func(self.bmi.as_ref() as *const Bmi as *mut Bmi, grid, buffer.as_mut_ptr() as *mut c_char) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: "get_grid_type".to_string(),
            });
        }

        cstr_to_string(&buffer)
    }

    /// Get the shape of a grid.
    pub fn get_grid_shape(&self, grid: i32) -> BmiResult<Vec<i32>> {
        self.require_initialized()?;
        let rank = self.get_grid_rank(grid)? as usize;
        self.get_int_array_for_grid("get_grid_shape", self.bmi.get_grid_shape, grid, rank)
    }

    /// Get the spacing between grid nodes.
    pub fn get_grid_spacing(&self, grid: i32) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;
        let rank = self.get_grid_rank(grid)? as usize;
        self.get_double_array_for_grid("get_grid_spacing", self.bmi.get_grid_spacing, grid, rank)
    }

    /// Get the origin of a grid.
    pub fn get_grid_origin(&self, grid: i32) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;
        let rank = self.get_grid_rank(grid)? as usize;
        self.get_double_array_for_grid("get_grid_origin", self.bmi.get_grid_origin, grid, rank)
    }

    /// Get the X coordinates of grid nodes.
    pub fn get_grid_x(&self, grid: i32) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;
        let size = self.get_grid_size(grid)? as usize;
        self.get_double_array_for_grid("get_grid_x", self.bmi.get_grid_x, grid, size)
    }

    /// Get the Y coordinates of grid nodes.
    pub fn get_grid_y(&self, grid: i32) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;
        let size = self.get_grid_size(grid)? as usize;
        self.get_double_array_for_grid("get_grid_y", self.bmi.get_grid_y, grid, size)
    }

    /// Get the Z coordinates of grid nodes.
    pub fn get_grid_z(&self, grid: i32) -> BmiResult<Vec<f64>> {
        self.require_initialized()?;
        let size = self.get_grid_size(grid)? as usize;
        self.get_double_array_for_grid("get_grid_z", self.bmi.get_grid_z, grid, size)
    }

    /// Get the number of nodes in a grid.
    pub fn get_grid_node_count(&self, grid: i32) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_for_grid("get_grid_node_count", self.bmi.get_grid_node_count, grid)
    }

    /// Get the number of edges in a grid.
    pub fn get_grid_edge_count(&self, grid: i32) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_for_grid("get_grid_edge_count", self.bmi.get_grid_edge_count, grid)
    }

    /// Get the number of faces in a grid.
    pub fn get_grid_face_count(&self, grid: i32) -> BmiResult<i32> {
        self.require_initialized()?;
        self.get_int_for_grid("get_grid_face_count", self.bmi.get_grid_face_count, grid)
    }

    /// Get the edge-node connectivity.
    pub fn get_grid_edge_nodes(&self, grid: i32) -> BmiResult<Vec<i32>> {
        self.require_initialized()?;
        let edge_count = self.get_grid_edge_count(grid)? as usize;
        self.get_int_array_for_grid("get_grid_edge_nodes", self.bmi.get_grid_edge_nodes, grid, edge_count * 2)
    }

    /// Get the face-edge connectivity.
    pub fn get_grid_face_edges(&self, grid: i32, size: usize) -> BmiResult<Vec<i32>> {
        self.require_initialized()?;
        self.get_int_array_for_grid("get_grid_face_edges", self.bmi.get_grid_face_edges, grid, size)
    }

    /// Get the face-node connectivity.
    pub fn get_grid_face_nodes(&self, grid: i32, size: usize) -> BmiResult<Vec<i32>> {
        self.require_initialized()?;
        self.get_int_array_for_grid("get_grid_face_nodes", self.bmi.get_grid_face_nodes, grid, size)
    }

    /// Get the number of nodes per face.
    pub fn get_grid_nodes_per_face(&self, grid: i32) -> BmiResult<Vec<i32>> {
        self.require_initialized()?;
        let face_count = self.get_grid_face_count(grid)? as usize;
        self.get_int_array_for_grid("get_grid_nodes_per_face", self.bmi.get_grid_nodes_per_face, grid, face_count)
    }

    // =========================================================================
    // Private Helper Methods
    // =========================================================================

    fn require_initialized(&self) -> BmiResult<()> {
        if !self.initialized {
            Err(BmiError::NotInitialized {
                model: self.model_name.clone(),
            })
        } else {
            Ok(())
        }
    }

    fn get_string_field(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut Bmi, *mut c_char) -> i32>,
        max_len: usize,
    ) -> BmiResult<String> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let mut buffer = vec![0u8; max_len];
        let result = unsafe { func(self.bmi.as_ref() as *const Bmi as *mut Bmi, buffer.as_mut_ptr() as *mut c_char) };

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
        func: Option<unsafe extern "C" fn(*mut Bmi, *mut i32) -> i32>,
    ) -> BmiResult<i32> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let mut value: i32 = 0;
        let result = unsafe { func(self.bmi.as_ref() as *const Bmi as *mut Bmi, &mut value) };

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
        func: Option<unsafe extern "C" fn(*mut Bmi, *mut f64) -> i32>,
    ) -> BmiResult<f64> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let mut value: f64 = 0.0;
        let result = unsafe { func(self.bmi.as_ref() as *const Bmi as *mut Bmi, &mut value) };

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
        func: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut i32) -> i32>,
        name: &str,
    ) -> BmiResult<i32> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut value: i32 = 0;
        let result = unsafe { func(self.bmi.as_ref() as *const Bmi as *mut Bmi, name_cstr.as_ptr(), &mut value) };

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
        func: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_char) -> i32>,
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
                self.bmi.as_ref() as *const Bmi as *mut Bmi,
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
        func: Option<unsafe extern "C" fn(*mut Bmi, i32, *mut i32) -> i32>,
        grid: i32,
    ) -> BmiResult<i32> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let mut value: i32 = 0;
        let result = unsafe { func(self.bmi.as_ref() as *const Bmi as *mut Bmi, grid, &mut value) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: func_name.to_string(),
            });
        }

        Ok(value)
    }

    fn get_int_array_for_grid(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut Bmi, i32, *mut i32) -> i32>,
        grid: i32,
        count: usize,
    ) -> BmiResult<Vec<i32>> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let mut values = vec![0i32; count];
        let result = unsafe { func(self.bmi.as_ref() as *const Bmi as *mut Bmi, grid, values.as_mut_ptr()) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: func_name.to_string(),
            });
        }

        Ok(values)
    }

    fn get_double_array_for_grid(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut Bmi, i32, *mut f64) -> i32>,
        grid: i32,
        count: usize,
    ) -> BmiResult<Vec<f64>> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        let mut values = vec![0.0f64; count];
        let result = unsafe { func(self.bmi.as_ref() as *const Bmi as *mut Bmi, grid, values.as_mut_ptr()) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: func_name.to_string(),
            });
        }

        Ok(values)
    }

    fn get_var_names(
        &self,
        func_name: &str,
        func: Option<unsafe extern "C" fn(*mut Bmi, *mut *mut c_char) -> i32>,
        count: usize,
    ) -> BmiResult<Vec<String>> {
        let func = func.ok_or_else(|| BmiError::FunctionNotImplemented {
            model: self.model_name.clone(),
            func: func_name.to_string(),
        })?;

        // Allocate array of string buffers
        let mut buffers: Vec<Vec<u8>> = (0..count).map(|_| vec![0u8; BMI_MAX_VAR_NAME]).collect();
        let mut ptrs: Vec<*mut c_char> = buffers
            .iter_mut()
            .map(|b| b.as_mut_ptr() as *mut c_char)
            .collect();

        let result = unsafe { func(self.bmi.as_ref() as *const Bmi as *mut Bmi, ptrs.as_mut_ptr()) };

        if result != BMI_SUCCESS {
            return Err(BmiError::BmiFunctionFailed {
                model: self.model_name.clone(),
                func: func_name.to_string(),
            });
        }

        // Convert buffers to strings
        buffers
            .iter()
            .map(|b| cstr_to_string(b))
            .collect()
    }
}

impl Drop for BmiModel {
    fn drop(&mut self) {
        // Try to finalize if still initialized
        if self.initialized {
            let _ = self.finalize();
        }
    }
}

// =========================================================================
// Helper Functions
// =========================================================================

fn path_to_cstring(path: &Path) -> CString {
    CString::new(path.to_string_lossy().as_bytes()).unwrap_or_else(|_| CString::new("").unwrap())
}

fn cstr_to_string(buffer: &[u8]) -> BmiResult<String> {
    let cstr = unsafe { CStr::from_ptr(buffer.as_ptr() as *const c_char) };
    cstr.to_str()
        .map(|s| s.to_string())
        .map_err(|_| BmiError::InvalidUtf8)
}
