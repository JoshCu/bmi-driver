use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::Path;

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

#[cfg(target_os = "linux")]
const RTLD_FLAGS: c_int = 0x2 | 0x100; // RTLD_NOW | RTLD_GLOBAL

#[cfg(target_os = "macos")]
const RTLD_FLAGS: c_int = 0x2 | 0x8;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const RTLD_FLAGS: c_int = 0x2;

pub struct GlobalLibrary {
    handle: *mut c_void,
}

impl GlobalLibrary {
    pub unsafe fn new(path: &Path) -> Result<Self, DlError> {
        let path_cstr = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|_| DlError("invalid path".into()))?;

        let _ = dlerror();
        let handle = dlopen(path_cstr.as_ptr(), RTLD_FLAGS);

        if handle.is_null() {
            let err = dlerror();
            let msg = if err.is_null() {
                "unknown dlopen error".into()
            } else {
                CStr::from_ptr(err).to_string_lossy().into_owned()
            };
            return Err(DlError(msg));
        }

        Ok(Self { handle })
    }

    pub unsafe fn get_fn<T>(&self, symbol: &str) -> Result<T, DlError> {
        let _ = dlerror();
        let symbol_cstr = CString::new(symbol).map_err(|_| DlError("invalid symbol".into()))?;
        let ptr = dlsym(self.handle, symbol_cstr.as_ptr());

        let err = dlerror();
        if !err.is_null() {
            return Err(DlError(CStr::from_ptr(err).to_string_lossy().into_owned()));
        }
        if ptr.is_null() {
            return Err(DlError(format!("symbol '{}' not found", symbol)));
        }

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

unsafe impl Send for GlobalLibrary {}
unsafe impl Sync for GlobalLibrary {}

pub fn preload_dependencies() {
    #[cfg(target_os = "linux")]
    for lib in &["libm.so.6", "libm.so"] {
        let _ = preload(lib);
    }
    #[cfg(target_os = "macos")]
    let _ = preload("libm.dylib");
}

fn preload(name: &str) -> Result<(), ()> {
    let cstr = CString::new(name).map_err(|_| ())?;
    unsafe {
        let _ = dlerror();
        let handle = dlopen(cstr.as_ptr(), RTLD_FLAGS);
        if handle.is_null() {
            Err(())
        } else {
            Ok(())
        }
    }
}
