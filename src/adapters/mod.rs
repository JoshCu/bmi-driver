//! BMI adapter implementations. See `bmi_functions.md` in this directory for a table of which
//! BMI functions each adapter calls.

mod c;
pub mod ffi;
#[cfg(feature = "fortran")]
mod fortran;
pub mod library;
#[cfg(feature = "python")]
mod python;
mod sloth;

pub use c::BmiC;
#[cfg(feature = "fortran")]
pub use fortran::BmiFortran;
#[cfg(feature = "python")]
pub use python::BmiPython;
pub use sloth::BmiSloth;

use crate::error::{BmiError, BmiResult};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;

pub fn cstr_to_string(buffer: &[u8]) -> BmiResult<String> {
    unsafe { CStr::from_ptr(buffer.as_ptr() as *const c_char) }
        .to_str()
        .map(|s| s.to_string())
        .map_err(|_| BmiError::InvalidUtf8)
}

pub fn check_initialized(initialized: bool, model: &str) -> BmiResult<()> {
    if initialized {
        Ok(())
    } else {
        Err(BmiError::NotInitialized {
            model: model.into(),
        })
    }
}

pub fn to_cstring(s: &str) -> BmiResult<CString> {
    CString::new(s).map_err(|_| BmiError::InvalidUtf8)
}

pub fn verify_config_path(config: &str) -> BmiResult<()> {
    if !Path::new(config).exists() {
        return Err(BmiError::ConfigNotFound {
            path: config.into(),
        });
    }
    Ok(())
}

macro_rules! impl_bmi_drop {
    ($type:ty) => {
        impl Drop for $type {
            fn drop(&mut self) {
                if self.is_initialized() {
                    let _ = self.finalize();
                }
            }
        }
    };
}
pub(crate) use impl_bmi_drop;
