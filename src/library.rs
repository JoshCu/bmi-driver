//! Shared library loading utilities.
//!
//! This module provides cross-platform dynamic library loading with RTLD_GLOBAL
//! support to properly resolve symbols between loaded libraries.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;

use crate::error::BmiResult;

/// Error type for library loading operations
#[derive(Debug)]
pub struct DlError(pub String);

impl std::fmt::Display for DlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for DlError {}

extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> c_int;
    fn dlerror() -> *const c_char;
}

// RTLD_NOW | RTLD_GLOBAL
#[cfg(target_os = "linux")]
pub const RTLD_FLAGS: c_int = 0x2 | 0x100;

#[cfg(target_os = "macos")]
pub const RTLD_FLAGS: c_int = 0x2 | 0x8;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub const RTLD_FLAGS: c_int = 0x2;

/// A wrapper around a dynamically loaded library that uses RTLD_GLOBAL.
pub struct GlobalLibrary {
    handle: *mut c_void,
}

impl GlobalLibrary {
    /// Load a shared library with RTLD_GLOBAL flag.
    pub unsafe fn new(path: &Path) -> Result<Self, DlError> {
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

    /// Get a function pointer from the library.
    pub unsafe fn get_fn<T>(&self, symbol: &str) -> Result<T, DlError> {
        // Clear any previous error
        let _ = dlerror();

        let symbol_cstr =
            CString::new(symbol).map_err(|_| DlError("invalid symbol name".to_string()))?;

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
    
    /// Get the raw handle (for advanced use cases).
    pub fn handle(&self) -> *mut c_void {
        self.handle
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

/// Preload common dependency libraries (libm, libc, etc.) with RTLD_GLOBAL.
/// Call this before loading BMI models that depend on these libraries.
pub fn preload_dependencies() -> BmiResult<()> {
    #[cfg(target_os = "linux")]
    {
        let libs = ["libm.so.6", "libm.so"];

        for lib in &libs {
            if preload_library(lib).is_ok() {
                return Ok(());
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let _ = preload_library("libm.dylib");
    }

    Ok(())
}

/// Preload a specific library with RTLD_GLOBAL.
fn preload_library(name: &str) -> BmiResult<()> {
    use crate::error::BmiError;
    
    let name_cstr = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;

    unsafe {
        let _ = dlerror();
    }

    let handle = unsafe { dlopen(name_cstr.as_ptr(), RTLD_FLAGS) };

    if handle.is_null() {
        Err(BmiError::LibraryLoad {
            path: name.to_string(),
            source: DlError("failed to preload".to_string()),
        })
    } else {
        // Don't close it - we want it to stay loaded
        Ok(())
    }
}
