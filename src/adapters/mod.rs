mod c;
#[cfg(feature = "fortran")]
mod fortran;
mod sloth;

pub use c::BmiC;
#[cfg(feature = "fortran")]
pub use fortran::BmiFortran;
pub use sloth::BmiSloth;

use crate::error::{BmiError, BmiResult};
use std::ffi::CStr;
use std::os::raw::c_char;

pub fn cstr_to_string(buffer: &[u8]) -> BmiResult<String> {
    unsafe { CStr::from_ptr(buffer.as_ptr() as *const c_char) }
        .to_str()
        .map(|s| s.to_string())
        .map_err(|_| BmiError::InvalidUtf8)
}

pub fn check_initialized(initialized: bool, model: &str) -> BmiResult<()> {
    if initialized { Ok(()) } else { Err(BmiError::NotInitialized { model: model.into() }) }
}
