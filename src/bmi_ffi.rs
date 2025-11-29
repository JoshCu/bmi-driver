//! Raw FFI bindings to the BMI C interface.
//! 
//! This module defines the C struct layout and function pointer types
//! that match the BMI 2.0 specification from bmi.h

use std::os::raw::{c_char, c_double, c_int, c_void};

/// Success return code for BMI functions
pub const BMI_SUCCESS: c_int = 0;
/// Failure return code for BMI functions  
pub const BMI_FAILURE: c_int = 1;

/// Maximum length for various BMI string fields
pub const BMI_MAX_UNITS_NAME: usize = 2048;
pub const BMI_MAX_TYPE_NAME: usize = 2048;
pub const BMI_MAX_COMPONENT_NAME: usize = 2048;
pub const BMI_MAX_VAR_NAME: usize = 2048;
pub const BMI_MAX_LOCATION_NAME: usize = 2048;

/// The BMI C struct containing function pointers to model methods.
/// 
/// This matches the `struct Bmi` definition from bmi.h exactly.
#[repr(C)]
pub struct Bmi {
    /// Opaque pointer to model-specific data
    pub data: *mut c_void,

    // Initialize, run, finalize (IRF)
    pub initialize: Option<unsafe extern "C" fn(*mut Bmi, *const c_char) -> c_int>,
    pub update: Option<unsafe extern "C" fn(*mut Bmi) -> c_int>,
    pub update_until: Option<unsafe extern "C" fn(*mut Bmi, c_double) -> c_int>,
    pub finalize: Option<unsafe extern "C" fn(*mut Bmi) -> c_int>,

    // Exchange items
    pub get_component_name: Option<unsafe extern "C" fn(*mut Bmi, *mut c_char) -> c_int>,
    pub get_input_item_count: Option<unsafe extern "C" fn(*mut Bmi, *mut c_int) -> c_int>,
    pub get_output_item_count: Option<unsafe extern "C" fn(*mut Bmi, *mut c_int) -> c_int>,
    pub get_input_var_names: Option<unsafe extern "C" fn(*mut Bmi, *mut *mut c_char) -> c_int>,
    pub get_output_var_names: Option<unsafe extern "C" fn(*mut Bmi, *mut *mut c_char) -> c_int>,

    // Variable information
    pub get_var_grid: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_int) -> c_int>,
    pub get_var_type: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_char) -> c_int>,
    pub get_var_units: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_char) -> c_int>,
    pub get_var_itemsize: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_int) -> c_int>,
    pub get_var_nbytes: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_int) -> c_int>,
    pub get_var_location: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_char) -> c_int>,

    // Time information
    pub get_current_time: Option<unsafe extern "C" fn(*mut Bmi, *mut c_double) -> c_int>,
    pub get_start_time: Option<unsafe extern "C" fn(*mut Bmi, *mut c_double) -> c_int>,
    pub get_end_time: Option<unsafe extern "C" fn(*mut Bmi, *mut c_double) -> c_int>,
    pub get_time_units: Option<unsafe extern "C" fn(*mut Bmi, *mut c_char) -> c_int>,
    pub get_time_step: Option<unsafe extern "C" fn(*mut Bmi, *mut c_double) -> c_int>,

    // Getters
    pub get_value: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_void) -> c_int>,
    pub get_value_ptr: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut *mut c_void) -> c_int>,
    pub get_value_at_indices: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_void, *mut c_int, c_int) -> c_int>,

    // Setters
    pub set_value: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_void) -> c_int>,
    pub set_value_at_indices: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_int, c_int, *mut c_void) -> c_int>,

    // Grid information
    pub get_grid_rank: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_size: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_type: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_char) -> c_int>,

    // Uniform rectilinear
    pub get_grid_shape: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_spacing: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_double) -> c_int>,
    pub get_grid_origin: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_double) -> c_int>,

    // Non-uniform rectilinear, curvilinear
    pub get_grid_x: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_double) -> c_int>,
    pub get_grid_y: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_double) -> c_int>,
    pub get_grid_z: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_double) -> c_int>,

    // Unstructured
    pub get_grid_node_count: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_edge_count: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_face_count: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_edge_nodes: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_face_edges: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_face_nodes: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_nodes_per_face: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
}

impl Default for Bmi {
    fn default() -> Self {
        Self {
            data: std::ptr::null_mut(),
            initialize: None,
            update: None,
            update_until: None,
            finalize: None,
            get_component_name: None,
            get_input_item_count: None,
            get_output_item_count: None,
            get_input_var_names: None,
            get_output_var_names: None,
            get_var_grid: None,
            get_var_type: None,
            get_var_units: None,
            get_var_itemsize: None,
            get_var_nbytes: None,
            get_var_location: None,
            get_current_time: None,
            get_start_time: None,
            get_end_time: None,
            get_time_units: None,
            get_time_step: None,
            get_value: None,
            get_value_ptr: None,
            get_value_at_indices: None,
            set_value: None,
            set_value_at_indices: None,
            get_grid_rank: None,
            get_grid_size: None,
            get_grid_type: None,
            get_grid_shape: None,
            get_grid_spacing: None,
            get_grid_origin: None,
            get_grid_x: None,
            get_grid_y: None,
            get_grid_z: None,
            get_grid_node_count: None,
            get_grid_edge_count: None,
            get_grid_face_count: None,
            get_grid_edge_nodes: None,
            get_grid_face_edges: None,
            get_grid_face_nodes: None,
            get_grid_nodes_per_face: None,
        }
    }
}

/// Type alias for the registration function that BMI C libraries must export.
/// This function populates the Bmi struct with the correct function pointers.
pub type BmiRegistrationFn = unsafe extern "C" fn(*mut Bmi) -> *mut Bmi;
