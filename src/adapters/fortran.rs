use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::{c_char, c_double, c_float, c_int, c_void};
use std::path::Path;

use super::{check_initialized, cstr_to_string};
use crate::error::{BmiError, BmiResult};
use crate::ffi::BMI_MAX_NAME;
use crate::library::GlobalLibrary;
use crate::traits::{parse_time_units, Bmi, VarType};

type InitFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;
type UpdateFn = unsafe extern "C" fn(*mut c_void) -> c_int;
type UpdateUntilFn = unsafe extern "C" fn(*mut c_void, *mut c_double) -> c_int;
type FinalizeFn = unsafe extern "C" fn(*mut c_void) -> c_int;
type GetStringFn = unsafe extern "C" fn(*mut c_void, *mut c_char) -> c_int;
type GetIntFn = unsafe extern "C" fn(*mut c_void, *mut c_int) -> c_int;
type GetNamesFn = unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> c_int;
type VarGridFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_int) -> c_int;
type VarStringFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_char) -> c_int;
type VarIntFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_int) -> c_int;
type TimeFn = unsafe extern "C" fn(*mut c_void, *mut c_double) -> c_int;
type GetValueIntFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_int) -> c_int;
type GetValueFloatFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_float) -> c_int;
type GetValueDoubleFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_double) -> c_int;
type SetValueIntFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_int) -> c_int;
type SetValueFloatFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_float) -> c_int;
type SetValueDoubleFn = unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_double) -> c_int;
type GridIntFn = unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_int) -> c_int;
type GridTypeFn = unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_char) -> c_int;

struct FortranFns {
    initialize: InitFn,
    update: UpdateFn,
    update_until: UpdateUntilFn,
    finalize: FinalizeFn,
    get_component_name: GetStringFn,
    get_input_item_count: GetIntFn,
    get_output_item_count: GetIntFn,
    get_input_var_names: GetNamesFn,
    get_output_var_names: GetNamesFn,
    get_var_grid: VarGridFn,
    get_var_type: VarStringFn,
    get_var_units: VarStringFn,
    get_var_itemsize: VarIntFn,
    get_var_nbytes: VarIntFn,
    get_var_location: VarStringFn,
    get_current_time: TimeFn,
    get_start_time: TimeFn,
    get_end_time: TimeFn,
    get_time_units: GetStringFn,
    get_time_step: TimeFn,
    get_value_int: GetValueIntFn,
    get_value_float: GetValueFloatFn,
    get_value_double: GetValueDoubleFn,
    set_value_int: SetValueIntFn,
    set_value_float: SetValueFloatFn,
    set_value_double: SetValueDoubleFn,
    get_grid_rank: GridIntFn,
    get_grid_size: GridIntFn,
    get_grid_type: GridTypeFn,
}

/// BMI adapter for Fortran shared libraries.
///
/// Supports two loading modes:
/// - **With middleware**: loads both a model `.so` and a separate C-to-Fortran middleware `.so`.
///   BMI function symbols are resolved from the middleware library.
/// - **Single library** (`load_single`): one `.so` provides both the registration function and all
///   BMI symbols.
///
/// An opaque handle (obtained from the registration function) is passed as the first argument to
/// every BMI call, matching the convention used by Fortran BMI implementations.
///
/// ## BMI functions called
///
/// **All 29 BMI functions are resolved at load time** and must be present in the library;
/// unlike `BmiC`, none are optional. They are stored in an internal `FortranFns` struct of typed
/// function pointers.
///
/// **Lifecycle** — called once per simulation:
/// - `initialize` — passes the config path as `*const c_char`.
/// - `finalize` — called on drop if the model was successfully initialized.
///
/// **Per-timestep** — called inside the simulation loop:
/// - `update` — advance the model by one timestep.
/// - `get_value_double` / `get_value_float` / `get_value_int` — typed function pointers (unlike
///   the C adapter's single type-erased pointer). The adapter calls `get_var_nbytes` first to
///   determine how many elements to allocate.
/// - `set_value_double` / `set_value_float` / `set_value_int` — typed function pointers.
///
/// **After `initialize`** (setup):
/// - `get_time_units` — to compute the internal `time_factor`.
/// - `get_input_item_count` / `get_output_item_count` → `get_input_var_names` /
///   `get_output_var_names` → `get_var_type` + `get_var_itemsize` (per variable) — type cache.
///
/// **On demand**:
/// - `update_until` — note: passes time by pointer (`*mut c_double`) rather than by value, per
///   Fortran convention.
/// - `get_component_name`, `get_var_grid`, `get_var_units`, `get_var_nbytes`,
///   `get_var_location`, `get_current_time`, `get_start_time`, `get_end_time`, `get_time_step`,
///   `get_grid_rank`, `get_grid_size`, `get_grid_type`.
///
/// ## Functions NOT called
///
/// `get_value_ptr`, `get_value_at_indices`, `set_value_at_indices`, and all extended grid
/// functions are not part of the `FortranFns` struct and are never loaded or called.
pub struct BmiFortran {
    model_name: String,
    _model_lib: GlobalLibrary,
    _middleware_lib: GlobalLibrary,
    fns: FortranFns,
    handle: *mut c_void,
    initialized: bool,
    time_factor: f64,
    type_cache: Option<HashMap<String, VarType>>,
}

macro_rules! load_fn {
    ($lib:expr, $name:expr, $ty:ty) => {
        unsafe { $lib.get_fn::<$ty>($name) }.map_err(|e| BmiError::SymbolNotFound {
            func: $name.into(),
            source: e,
        })?
    };
}

impl BmiFortran {
    pub fn load(
        name: impl Into<String>,
        model_path: impl AsRef<Path>,
        middleware_path: impl AsRef<Path>,
        reg_func: &str,
    ) -> BmiResult<Self> {
        let name = name.into();
        let model_path = model_path.as_ref();
        let middleware_path = middleware_path.as_ref();

        let middleware_lib =
            unsafe { GlobalLibrary::new(middleware_path) }.map_err(|e| BmiError::LibraryLoad {
                path: middleware_path.display().to_string(),
                source: e,
            })?;

        let model_lib =
            unsafe { GlobalLibrary::new(model_path) }.map_err(|e| BmiError::LibraryLoad {
                path: model_path.display().to_string(),
                source: e,
            })?;

        let register: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
            unsafe { model_lib.get_fn(reg_func) }.map_err(|e| BmiError::SymbolNotFound {
                func: reg_func.into(),
                source: e,
            })?;

        let mut handle: *mut c_void = std::ptr::null_mut();
        unsafe {
            register(&mut handle as *mut *mut c_void as *mut c_void);
        }

        if handle.is_null() {
            return Err(BmiError::FunctionFailed {
                model: name,
                func: reg_func.into(),
            });
        }

        let fns = Self::load_fns(&middleware_lib)?;

        Ok(Self {
            model_name: name,
            _model_lib: model_lib,
            _middleware_lib: middleware_lib,
            fns,
            handle,
            initialized: false,
            time_factor: 1.0,
            type_cache: None,
        })
    }

    pub fn load_single(
        name: impl Into<String>,
        lib_path: impl AsRef<Path>,
        reg_func: &str,
    ) -> BmiResult<Self> {
        let name = name.into();
        let lib_path = lib_path.as_ref();

        let lib = unsafe { GlobalLibrary::new(lib_path) }.map_err(|e| BmiError::LibraryLoad {
            path: lib_path.display().to_string(),
            source: e,
        })?;

        let register: unsafe extern "C" fn(*mut c_void) -> *mut c_void =
            unsafe { lib.get_fn(reg_func) }.map_err(|e| BmiError::SymbolNotFound {
                func: reg_func.into(),
                source: e,
            })?;

        let mut handle: *mut c_void = std::ptr::null_mut();
        unsafe {
            register(&mut handle as *mut *mut c_void as *mut c_void);
        }

        if handle.is_null() {
            return Err(BmiError::FunctionFailed {
                model: name,
                func: reg_func.into(),
            });
        }

        let fns = Self::load_fns(&lib)?;
        let lib2 = unsafe { GlobalLibrary::new(lib_path) }.map_err(|e| BmiError::LibraryLoad {
            path: lib_path.display().to_string(),
            source: e,
        })?;

        Ok(Self {
            model_name: name,
            _model_lib: lib,
            _middleware_lib: lib2,
            fns,
            handle,
            initialized: false,
            time_factor: 1.0,
            type_cache: None,
        })
    }

    fn load_fns(lib: &GlobalLibrary) -> BmiResult<FortranFns> {
        Ok(FortranFns {
            initialize: load_fn!(lib, "initialize", InitFn),
            update: load_fn!(lib, "update", UpdateFn),
            update_until: load_fn!(lib, "update_until", UpdateUntilFn),
            finalize: load_fn!(lib, "finalize", FinalizeFn),
            get_component_name: load_fn!(lib, "get_component_name", GetStringFn),
            get_input_item_count: load_fn!(lib, "get_input_item_count", GetIntFn),
            get_output_item_count: load_fn!(lib, "get_output_item_count", GetIntFn),
            get_input_var_names: load_fn!(lib, "get_input_var_names", GetNamesFn),
            get_output_var_names: load_fn!(lib, "get_output_var_names", GetNamesFn),
            get_var_grid: load_fn!(lib, "get_var_grid", VarGridFn),
            get_var_type: load_fn!(lib, "get_var_type", VarStringFn),
            get_var_units: load_fn!(lib, "get_var_units", VarStringFn),
            get_var_itemsize: load_fn!(lib, "get_var_itemsize", VarIntFn),
            get_var_nbytes: load_fn!(lib, "get_var_nbytes", VarIntFn),
            get_var_location: load_fn!(lib, "get_var_location", VarStringFn),
            get_current_time: load_fn!(lib, "get_current_time", TimeFn),
            get_start_time: load_fn!(lib, "get_start_time", TimeFn),
            get_end_time: load_fn!(lib, "get_end_time", TimeFn),
            get_time_units: load_fn!(lib, "get_time_units", GetStringFn),
            get_time_step: load_fn!(lib, "get_time_step", TimeFn),
            get_value_int: load_fn!(lib, "get_value_int", GetValueIntFn),
            get_value_float: load_fn!(lib, "get_value_float", GetValueFloatFn),
            get_value_double: load_fn!(lib, "get_value_double", GetValueDoubleFn),
            set_value_int: load_fn!(lib, "set_value_int", SetValueIntFn),
            set_value_float: load_fn!(lib, "set_value_float", SetValueFloatFn),
            set_value_double: load_fn!(lib, "set_value_double", SetValueDoubleFn),
            get_grid_rank: load_fn!(lib, "get_grid_rank", GridIntFn),
            get_grid_size: load_fn!(lib, "get_grid_size", GridIntFn),
            get_grid_type: load_fn!(lib, "get_grid_type", GridTypeFn),
        })
    }

    fn handle_ptr(&self) -> *mut c_void {
        &self.handle as *const *mut c_void as *mut c_void
    }

    fn handle_ptr_mut(&mut self) -> *mut c_void {
        &mut self.handle as *mut *mut c_void as *mut c_void
    }

    fn err(&self, func: &str) -> BmiError {
        BmiError::FunctionFailed {
            model: self.model_name.clone(),
            func: func.into(),
        }
    }
}

impl Bmi for BmiFortran {
    fn name(&self) -> &str {
        &self.model_name
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
                model: self.model_name.clone(),
            });
        }
        if !Path::new(config).exists() {
            return Err(BmiError::ConfigNotFound {
                path: config.into(),
            });
        }

        let cconfig = CString::new(config).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe { (self.fns.initialize)(self.handle_ptr_mut(), cconfig.as_ptr()) } != 0 {
            return Err(self.err("initialize"));
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
        check_initialized(self.initialized, &self.model_name)?;
        if unsafe { (self.fns.update)(self.handle_ptr_mut()) } != 0 {
            return Err(self.err("update"));
        }
        Ok(())
    }

    fn update_until(&mut self, time: f64) -> BmiResult<()> {
        check_initialized(self.initialized, &self.model_name)?;
        let mut t = time;
        if unsafe { (self.fns.update_until)(self.handle_ptr_mut(), &mut t) } != 0 {
            return Err(self.err("update_until"));
        }
        Ok(())
    }

    fn finalize(&mut self) -> BmiResult<()> {
        if !self.initialized {
            return Ok(());
        }
        let result = unsafe { (self.fns.finalize)(self.handle_ptr_mut()) };
        self.initialized = false;
        if result != 0 {
            return Err(self.err("finalize"));
        }
        Ok(())
    }

    fn get_component_name(&self) -> BmiResult<String> {
        let mut buf = vec![0u8; BMI_MAX_NAME];
        if unsafe {
            (self.fns.get_component_name)(self.handle_ptr(), buf.as_mut_ptr() as *mut c_char)
        } != 0
        {
            return Err(self.err("get_component_name"));
        }
        cstr_to_string(&buf)
    }

    fn get_input_item_count(&self) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        let mut count = 0;
        if unsafe { (self.fns.get_input_item_count)(self.handle_ptr(), &mut count) } != 0 {
            return Err(self.err("get_input_item_count"));
        }
        Ok(count)
    }

    fn get_output_item_count(&self) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        let mut count = 0;
        if unsafe { (self.fns.get_output_item_count)(self.handle_ptr(), &mut count) } != 0 {
            return Err(self.err("get_output_item_count"));
        }
        Ok(count)
    }

    fn get_input_var_names(&self) -> BmiResult<Vec<String>> {
        check_initialized(self.initialized, &self.model_name)?;
        let count = self.get_input_item_count()? as usize;
        let mut bufs: Vec<Vec<u8>> = (0..count).map(|_| vec![0u8; BMI_MAX_NAME]).collect();
        let mut ptrs: Vec<*mut c_char> = bufs
            .iter_mut()
            .map(|b| b.as_mut_ptr() as *mut c_char)
            .collect();
        if unsafe { (self.fns.get_input_var_names)(self.handle_ptr(), ptrs.as_mut_ptr()) } != 0 {
            return Err(self.err("get_input_var_names"));
        }
        bufs.iter().map(|b| cstr_to_string(b)).collect()
    }

    fn get_output_var_names(&self) -> BmiResult<Vec<String>> {
        check_initialized(self.initialized, &self.model_name)?;
        let count = self.get_output_item_count()? as usize;
        let mut bufs: Vec<Vec<u8>> = (0..count).map(|_| vec![0u8; BMI_MAX_NAME]).collect();
        let mut ptrs: Vec<*mut c_char> = bufs
            .iter_mut()
            .map(|b| b.as_mut_ptr() as *mut c_char)
            .collect();
        if unsafe { (self.fns.get_output_var_names)(self.handle_ptr(), ptrs.as_mut_ptr()) } != 0 {
            return Err(self.err("get_output_var_names"));
        }
        bufs.iter().map(|b| cstr_to_string(b)).collect()
    }

    fn get_var_grid(&self, name: &str) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut grid = 0;
        if unsafe { (self.fns.get_var_grid)(self.handle_ptr(), cname.as_ptr(), &mut grid) } != 0 {
            return Err(self.err("get_var_grid"));
        }
        Ok(grid)
    }

    fn get_var_type(&self, name: &str) -> BmiResult<String> {
        check_initialized(self.initialized, &self.model_name)?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut buf = vec![0u8; BMI_MAX_NAME];
        if unsafe {
            (self.fns.get_var_type)(
                self.handle_ptr(),
                cname.as_ptr(),
                buf.as_mut_ptr() as *mut c_char,
            )
        } != 0
        {
            return Err(self.err("get_var_type"));
        }
        cstr_to_string(&buf)
    }

    fn get_var_units(&self, name: &str) -> BmiResult<String> {
        check_initialized(self.initialized, &self.model_name)?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut buf = vec![0u8; BMI_MAX_NAME];
        if unsafe {
            (self.fns.get_var_units)(
                self.handle_ptr(),
                cname.as_ptr(),
                buf.as_mut_ptr() as *mut c_char,
            )
        } != 0
        {
            return Err(self.err("get_var_units"));
        }
        cstr_to_string(&buf)
    }

    fn get_var_itemsize(&self, name: &str) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut size = 0;
        if unsafe { (self.fns.get_var_itemsize)(self.handle_ptr(), cname.as_ptr(), &mut size) } != 0
        {
            return Err(self.err("get_var_itemsize"));
        }
        Ok(size)
    }

    fn get_var_nbytes(&self, name: &str) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut nbytes = 0;
        if unsafe { (self.fns.get_var_nbytes)(self.handle_ptr(), cname.as_ptr(), &mut nbytes) } != 0
        {
            return Err(self.err("get_var_nbytes"));
        }
        Ok(nbytes)
    }

    fn get_var_location(&self, name: &str) -> BmiResult<String> {
        check_initialized(self.initialized, &self.model_name)?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        let mut buf = vec![0u8; BMI_MAX_NAME];
        if unsafe {
            (self.fns.get_var_location)(
                self.handle_ptr(),
                cname.as_ptr(),
                buf.as_mut_ptr() as *mut c_char,
            )
        } != 0
        {
            return Err(self.err("get_var_location"));
        }
        cstr_to_string(&buf)
    }

    fn get_current_time(&self) -> BmiResult<f64> {
        check_initialized(self.initialized, &self.model_name)?;
        let mut time = 0.0;
        if unsafe { (self.fns.get_current_time)(self.handle_ptr(), &mut time) } != 0 {
            return Err(self.err("get_current_time"));
        }
        Ok(time)
    }

    fn get_start_time(&self) -> BmiResult<f64> {
        check_initialized(self.initialized, &self.model_name)?;
        let mut time = 0.0;
        if unsafe { (self.fns.get_start_time)(self.handle_ptr(), &mut time) } != 0 {
            return Err(self.err("get_start_time"));
        }
        Ok(time)
    }

    fn get_end_time(&self) -> BmiResult<f64> {
        check_initialized(self.initialized, &self.model_name)?;
        let mut time = 0.0;
        if unsafe { (self.fns.get_end_time)(self.handle_ptr(), &mut time) } != 0 {
            return Err(self.err("get_end_time"));
        }
        Ok(time)
    }

    fn get_time_units(&self) -> BmiResult<String> {
        check_initialized(self.initialized, &self.model_name)?;
        let mut buf = vec![0u8; BMI_MAX_NAME];
        if unsafe { (self.fns.get_time_units)(self.handle_ptr(), buf.as_mut_ptr() as *mut c_char) }
            != 0
        {
            return Err(self.err("get_time_units"));
        }
        cstr_to_string(&buf)
    }

    fn get_time_step(&self) -> BmiResult<f64> {
        check_initialized(self.initialized, &self.model_name)?;
        let mut ts = 0.0;
        if unsafe { (self.fns.get_time_step)(self.handle_ptr(), &mut ts) } != 0 {
            return Err(self.err("get_time_step"));
        }
        Ok(ts)
    }

    fn get_value_f64(&self, name: &str) -> BmiResult<Vec<f64>> {
        check_initialized(self.initialized, &self.model_name)?;
        let count = self.get_var_nbytes(name)? as usize / 8;
        let mut vals = vec![0.0f64; count];
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe {
            (self.fns.get_value_double)(self.handle_ptr(), cname.as_ptr(), vals.as_mut_ptr())
        } != 0
        {
            return Err(self.err("get_value_double"));
        }
        Ok(vals)
    }

    fn get_value_f32(&self, name: &str) -> BmiResult<Vec<f32>> {
        check_initialized(self.initialized, &self.model_name)?;
        let count = self.get_var_nbytes(name)? as usize / 4;
        let mut vals = vec![0.0f32; count];
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe {
            (self.fns.get_value_float)(self.handle_ptr(), cname.as_ptr(), vals.as_mut_ptr())
        } != 0
        {
            return Err(self.err("get_value_float"));
        }
        Ok(vals)
    }

    fn get_value_i32(&self, name: &str) -> BmiResult<Vec<i32>> {
        check_initialized(self.initialized, &self.model_name)?;
        let count = self.get_var_nbytes(name)? as usize / 4;
        let mut vals = vec![0i32; count];
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe { (self.fns.get_value_int)(self.handle_ptr(), cname.as_ptr(), vals.as_mut_ptr()) }
            != 0
        {
            return Err(self.err("get_value_int"));
        }
        Ok(vals)
    }

    fn set_value_f64(&mut self, name: &str, values: &[f64]) -> BmiResult<()> {
        check_initialized(self.initialized, &self.model_name)?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe {
            (self.fns.set_value_double)(
                self.handle_ptr_mut(),
                cname.as_ptr(),
                values.as_ptr() as *mut c_double,
            )
        } != 0
        {
            return Err(self.err("set_value_double"));
        }
        Ok(())
    }

    fn set_value_f32(&mut self, name: &str, values: &[f32]) -> BmiResult<()> {
        check_initialized(self.initialized, &self.model_name)?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe {
            (self.fns.set_value_float)(
                self.handle_ptr_mut(),
                cname.as_ptr(),
                values.as_ptr() as *mut c_float,
            )
        } != 0
        {
            return Err(self.err("set_value_float"));
        }
        Ok(())
    }

    fn set_value_i32(&mut self, name: &str, values: &[i32]) -> BmiResult<()> {
        check_initialized(self.initialized, &self.model_name)?;
        let cname = CString::new(name).map_err(|_| BmiError::InvalidUtf8)?;
        if unsafe {
            (self.fns.set_value_int)(
                self.handle_ptr_mut(),
                cname.as_ptr(),
                values.as_ptr() as *mut c_int,
            )
        } != 0
        {
            return Err(self.err("set_value_int"));
        }
        Ok(())
    }

    fn get_grid_rank(&self, grid: i32) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        let mut g = grid;
        let mut rank = 0;
        if unsafe { (self.fns.get_grid_rank)(self.handle_ptr(), &mut g, &mut rank) } != 0 {
            return Err(self.err("get_grid_rank"));
        }
        Ok(rank)
    }

    fn get_grid_size(&self, grid: i32) -> BmiResult<i32> {
        check_initialized(self.initialized, &self.model_name)?;
        let mut g = grid;
        let mut size = 0;
        if unsafe { (self.fns.get_grid_size)(self.handle_ptr(), &mut g, &mut size) } != 0 {
            return Err(self.err("get_grid_size"));
        }
        Ok(size)
    }

    fn get_grid_type(&self, grid: i32) -> BmiResult<String> {
        check_initialized(self.initialized, &self.model_name)?;
        let mut g = grid;
        let mut buf = vec![0u8; BMI_MAX_NAME];
        if unsafe {
            (self.fns.get_grid_type)(self.handle_ptr(), &mut g, buf.as_mut_ptr() as *mut c_char)
        } != 0
        {
            return Err(self.err("get_grid_type"));
        }
        cstr_to_string(&buf)
    }
}

impl Drop for BmiFortran {
    fn drop(&mut self) {
        if self.initialized {
            let _ = self.finalize();
        }
    }
}
