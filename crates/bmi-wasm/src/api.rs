//! C ABI exposed to JavaScript through the Emscripten module.
//!
//! Conventions (mirrored by `js/bmi.mjs`, which is what users should actually call):
//! - Models are referenced by an `i32` handle returned from `bmi_open`.
//! - Status-returning functions return `0` on success, `-1` on failure; the failure message
//!   is retrievable with `bmi_last_error()`.
//! - `*mut c_char` returns are Rust-allocated C strings; free them with `bmi_free_string`.
//! - `bmi_get_value` returns a Rust-allocated buffer plus its length and a type code through
//!   out-pointers; free it with `bmi_free_buffer`.
//! - Type codes: 0 = f64/double, 1 = f32/float, 2 = i32/int.

use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};
use std::sync::Mutex;

use bmi_core::error::BmiResult;
use bmi_core::traits::{Bmi, BmiExt, BmiValue, VarType};
use bmi_core::BmiC;

/// `BmiC` holds raw pointers so it isn't `Send`, but the Emscripten runtime is
/// single-threaded — the store is only ever touched from the one JS thread.
struct ModelStore(Vec<Option<BmiC>>);
unsafe impl Send for ModelStore {}

static MODELS: Mutex<ModelStore> = Mutex::new(ModelStore(Vec::new()));
static LAST_ERROR: Mutex<Option<CString>> = Mutex::new(None);

const TYPE_F64: c_int = 0;
const TYPE_F32: c_int = 1;
const TYPE_I32: c_int = 2;

fn set_error(msg: impl std::fmt::Display) {
    let cstr = CString::new(msg.to_string())
        .unwrap_or_else(|_| CString::new("error message contained NUL").unwrap());
    *LAST_ERROR.lock().unwrap() = Some(cstr);
}

/// Run `f` against the model for `handle`, mapping any error to -1 + last_error.
fn with_model<T>(handle: c_int, f: impl FnOnce(&mut BmiC) -> BmiResult<T>) -> Option<T> {
    let mut models = MODELS.lock().unwrap();
    let slot = models.0.get_mut(handle as usize).and_then(|s| s.as_mut());
    match slot {
        None => {
            set_error(format!("invalid model handle {handle}"));
            None
        }
        Some(model) => match f(model) {
            Ok(v) => Some(v),
            Err(e) => {
                set_error(e);
                None
            }
        },
    }
}

fn cstr_arg<'a>(ptr: *const c_char, what: &str) -> Option<&'a str> {
    if ptr.is_null() {
        set_error(format!("{what} is null"));
        return None;
    }
    match unsafe { CStr::from_ptr(ptr) }.to_str() {
        Ok(s) => Some(s),
        Err(_) => {
            set_error(format!("{what} is not valid UTF-8"));
            None
        }
    }
}

fn status(res: Option<()>) -> c_int {
    match res {
        Some(()) => 0,
        None => -1,
    }
}

fn string_result(res: Option<String>) -> *mut c_char {
    match res.and_then(|s| {
        CString::new(s)
            .map_err(|_| set_error("string contained interior NUL"))
            .ok()
    }) {
        Some(cs) => cs.into_raw(),
        None => std::ptr::null_mut(),
    }
}

fn double_result(res: Option<f64>) -> c_double {
    res.unwrap_or(f64::NAN)
}

/// Load a model shared object (an Emscripten side module in MEMFS) and register its BMI
/// struct via `reg_func`. Returns a handle >= 0, or -1 on failure.
#[no_mangle]
pub extern "C" fn bmi_open(
    name: *const c_char,
    lib_path: *const c_char,
    reg_func: *const c_char,
) -> c_int {
    let (Some(name), Some(lib_path), Some(reg_func)) = (
        cstr_arg(name, "model name"),
        cstr_arg(lib_path, "library path"),
        cstr_arg(reg_func, "registration function name"),
    ) else {
        return -1;
    };

    match BmiC::load(name, lib_path, reg_func) {
        Ok(model) => {
            let mut models = MODELS.lock().unwrap();
            // Reuse a closed slot if one exists so handles don't grow unboundedly.
            if let Some(idx) = models.0.iter().position(|s| s.is_none()) {
                models.0[idx] = Some(model);
                idx as c_int
            } else {
                models.0.push(Some(model));
                (models.0.len() - 1) as c_int
            }
        }
        Err(e) => {
            set_error(e);
            -1
        }
    }
}

/// Drop the model (finalizing it if needed) and free its handle.
#[no_mangle]
pub extern "C" fn bmi_close(handle: c_int) -> c_int {
    let mut models = MODELS.lock().unwrap();
    match models.0.get_mut(handle as usize).and_then(|s| s.take()) {
        Some(_model) => 0, // dropped here; Drop impl finalizes
        None => {
            set_error(format!("invalid model handle {handle}"));
            -1
        }
    }
}

/// Most recent error message, or null if none has been recorded.
/// The pointer stays valid until the next failing API call; do not free it.
#[no_mangle]
pub extern "C" fn bmi_last_error() -> *const c_char {
    LAST_ERROR
        .lock()
        .unwrap()
        .as_ref()
        .map_or(std::ptr::null(), |s| s.as_ptr())
}

#[no_mangle]
pub extern "C" fn bmi_initialize(handle: c_int, config: *const c_char) -> c_int {
    let Some(config) = cstr_arg(config, "config path") else {
        return -1;
    };
    status(with_model(handle, |m| m.initialize(config)))
}

#[no_mangle]
pub extern "C" fn bmi_update(handle: c_int) -> c_int {
    status(with_model(handle, |m| m.update()))
}

#[no_mangle]
pub extern "C" fn bmi_update_until(handle: c_int, time: c_double) -> c_int {
    status(with_model(handle, |m| m.update_until(time)))
}

#[no_mangle]
pub extern "C" fn bmi_finalize(handle: c_int) -> c_int {
    status(with_model(handle, |m| m.finalize()))
}

#[no_mangle]
pub extern "C" fn bmi_get_component_name(handle: c_int) -> *mut c_char {
    string_result(with_model(handle, |m| m.get_component_name()))
}

#[no_mangle]
pub extern "C" fn bmi_get_input_var_names_json(handle: c_int) -> *mut c_char {
    string_result(
        with_model(handle, |m| m.get_input_var_names())
            .map(|names| serde_json::to_string(&names).unwrap()),
    )
}

#[no_mangle]
pub extern "C" fn bmi_get_output_var_names_json(handle: c_int) -> *mut c_char {
    string_result(
        with_model(handle, |m| m.get_output_var_names())
            .map(|names| serde_json::to_string(&names).unwrap()),
    )
}

/// JSON object with type/units/itemsize/nbytes/location/grid for one variable.
/// Fields a model doesn't implement are null rather than failing the whole call.
#[no_mangle]
pub extern "C" fn bmi_get_var_info_json(handle: c_int, name: *const c_char) -> *mut c_char {
    let Some(name) = cstr_arg(name, "variable name") else {
        return std::ptr::null_mut();
    };
    string_result(with_model(handle, |m| {
        Ok(serde_json::json!({
            "name": name,
            "type": m.get_var_type(name).ok(),
            "units": m.get_var_units(name).ok(),
            "itemsize": m.get_var_itemsize(name).ok(),
            "nbytes": m.get_var_nbytes(name).ok(),
            "location": m.get_var_location(name).ok(),
            "grid": m.get_var_grid(name).ok(),
        })
        .to_string())
    }))
}

fn type_code(m: &mut BmiC, name: &str) -> BmiResult<c_int> {
    let vt = match m.cached_type(name) {
        Some(vt) => vt,
        None => {
            let t = m.get_var_type(name)?;
            let s = m.get_var_itemsize(name)?;
            VarType::from_bmi(&t, s)
        }
    };
    Ok(match vt {
        VarType::Double => TYPE_F64,
        VarType::Float => TYPE_F32,
        VarType::Int => TYPE_I32,
        VarType::Unknown(_) => -1,
    })
}

/// Type code for a variable (0 = double, 1 = float, 2 = int), or -1 if unknown/error.
#[no_mangle]
pub extern "C" fn bmi_get_var_type_code(handle: c_int, name: *const c_char) -> c_int {
    let Some(name) = cstr_arg(name, "variable name") else {
        return -1;
    };
    with_model(handle, |m| type_code(m, name)).unwrap_or(-1)
}

/// Fetch a variable into a freshly allocated buffer. Writes the element count to `out_len`
/// and the type code to `out_type`, and returns the buffer pointer (null on failure).
/// The caller must release the buffer with `bmi_free_buffer(ptr, len, type)`.
#[no_mangle]
pub extern "C" fn bmi_get_value(
    handle: c_int,
    name: *const c_char,
    out_len: *mut c_int,
    out_type: *mut c_int,
) -> *mut c_void {
    let Some(name) = cstr_arg(name, "variable name") else {
        return std::ptr::null_mut();
    };
    if out_len.is_null() || out_type.is_null() {
        set_error("out_len/out_type pointer is null");
        return std::ptr::null_mut();
    }

    let Some(value) = with_model(handle, |m| m.get_value(name)) else {
        return std::ptr::null_mut();
    };

    fn leak<T>(v: Vec<T>) -> (*mut c_void, c_int) {
        let boxed = v.into_boxed_slice();
        let len = boxed.len() as c_int;
        (Box::into_raw(boxed) as *mut c_void, len)
    }

    let (ptr, len, ty) = match value {
        BmiValue::Double(v) => {
            let (p, l) = leak(v);
            (p, l, TYPE_F64)
        }
        BmiValue::Float(v) => {
            let (p, l) = leak(v);
            (p, l, TYPE_F32)
        }
        BmiValue::Int(v) => {
            let (p, l) = leak(v);
            (p, l, TYPE_I32)
        }
    };
    unsafe {
        *out_len = len;
        *out_type = ty;
    }
    ptr
}

/// Free a buffer returned by `bmi_get_value`.
#[no_mangle]
pub extern "C" fn bmi_free_buffer(ptr: *mut c_void, len: c_int, type_code: c_int) {
    if ptr.is_null() || len < 0 {
        return;
    }
    let len = len as usize;
    unsafe {
        match type_code {
            TYPE_F64 => drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                ptr as *mut f64,
                len,
            ))),
            TYPE_F32 => drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                ptr as *mut f32,
                len,
            ))),
            TYPE_I32 => drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(
                ptr as *mut i32,
                len,
            ))),
            _ => {}
        }
    }
}

/// Set a variable from a caller-owned buffer of `len` elements whose element type matches
/// the model's declared type for `name` (query with `bmi_get_var_type_code`).
#[no_mangle]
pub extern "C" fn bmi_set_value(
    handle: c_int,
    name: *const c_char,
    values: *const c_void,
    len: c_int,
) -> c_int {
    let Some(name) = cstr_arg(name, "variable name") else {
        return -1;
    };
    if values.is_null() || len < 0 {
        set_error("values pointer is null or length is negative");
        return -1;
    }
    let len = len as usize;

    status(with_model(handle, |m| {
        match type_code(m, name)? {
            TYPE_F64 => {
                let vals = unsafe { std::slice::from_raw_parts(values as *const f64, len) };
                m.set_value_f64(name, vals)
            }
            TYPE_F32 => {
                let vals = unsafe { std::slice::from_raw_parts(values as *const f32, len) };
                m.set_value_f32(name, vals)
            }
            TYPE_I32 => {
                let vals = unsafe { std::slice::from_raw_parts(values as *const i32, len) };
                m.set_value_i32(name, vals)
            }
            _ => Err(bmi_core::error::function_failed(
                m.name(),
                format!("unknown variable type for '{name}'"),
            )),
        }
    }))
}

#[no_mangle]
pub extern "C" fn bmi_get_current_time(handle: c_int) -> c_double {
    double_result(with_model(handle, |m| m.get_current_time()))
}

#[no_mangle]
pub extern "C" fn bmi_get_start_time(handle: c_int) -> c_double {
    double_result(with_model(handle, |m| m.get_start_time()))
}

#[no_mangle]
pub extern "C" fn bmi_get_end_time(handle: c_int) -> c_double {
    double_result(with_model(handle, |m| m.get_end_time()))
}

#[no_mangle]
pub extern "C" fn bmi_get_time_step(handle: c_int) -> c_double {
    double_result(with_model(handle, |m| m.get_time_step()))
}

#[no_mangle]
pub extern "C" fn bmi_get_time_units(handle: c_int) -> *mut c_char {
    string_result(with_model(handle, |m| m.get_time_units()))
}

/// Free a string returned by any of the `*_json` / name / units functions.
#[no_mangle]
pub extern "C" fn bmi_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe { drop(CString::from_raw(ptr)) };
    }
}

