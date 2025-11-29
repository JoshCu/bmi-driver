use std::os::raw::{c_char, c_double, c_int, c_void};

pub const BMI_SUCCESS: c_int = 0;
pub const BMI_MAX_NAME: usize = 2048;

#[repr(C)]
#[derive(Default)]
pub struct Bmi {
    pub data: *mut c_void,

    pub initialize: Option<unsafe extern "C" fn(*mut Bmi, *const c_char) -> c_int>,
    pub update: Option<unsafe extern "C" fn(*mut Bmi) -> c_int>,
    pub update_until: Option<unsafe extern "C" fn(*mut Bmi, c_double) -> c_int>,
    pub finalize: Option<unsafe extern "C" fn(*mut Bmi) -> c_int>,

    pub get_component_name: Option<unsafe extern "C" fn(*mut Bmi, *mut c_char) -> c_int>,
    pub get_input_item_count: Option<unsafe extern "C" fn(*mut Bmi, *mut c_int) -> c_int>,
    pub get_output_item_count: Option<unsafe extern "C" fn(*mut Bmi, *mut c_int) -> c_int>,
    pub get_input_var_names: Option<unsafe extern "C" fn(*mut Bmi, *mut *mut c_char) -> c_int>,
    pub get_output_var_names: Option<unsafe extern "C" fn(*mut Bmi, *mut *mut c_char) -> c_int>,

    pub get_var_grid: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_int) -> c_int>,
    pub get_var_type: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_char) -> c_int>,
    pub get_var_units: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_char) -> c_int>,
    pub get_var_itemsize: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_int) -> c_int>,
    pub get_var_nbytes: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_int) -> c_int>,
    pub get_var_location: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_char) -> c_int>,

    pub get_current_time: Option<unsafe extern "C" fn(*mut Bmi, *mut c_double) -> c_int>,
    pub get_start_time: Option<unsafe extern "C" fn(*mut Bmi, *mut c_double) -> c_int>,
    pub get_end_time: Option<unsafe extern "C" fn(*mut Bmi, *mut c_double) -> c_int>,
    pub get_time_units: Option<unsafe extern "C" fn(*mut Bmi, *mut c_char) -> c_int>,
    pub get_time_step: Option<unsafe extern "C" fn(*mut Bmi, *mut c_double) -> c_int>,

    pub get_value: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_void) -> c_int>,
    pub get_value_ptr: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut *mut c_void) -> c_int>,
    pub get_value_at_indices: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_void, *mut c_int, c_int) -> c_int>,

    pub set_value: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_void) -> c_int>,
    pub set_value_at_indices: Option<unsafe extern "C" fn(*mut Bmi, *const c_char, *mut c_int, c_int, *mut c_void) -> c_int>,

    pub get_grid_rank: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_size: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_type: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_char) -> c_int>,
    pub get_grid_shape: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_spacing: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_double) -> c_int>,
    pub get_grid_origin: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_double) -> c_int>,

    pub get_grid_x: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_double) -> c_int>,
    pub get_grid_y: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_double) -> c_int>,
    pub get_grid_z: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_double) -> c_int>,

    pub get_grid_node_count: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_edge_count: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_face_count: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_edge_nodes: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_face_edges: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_face_nodes: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
    pub get_grid_nodes_per_face: Option<unsafe extern "C" fn(*mut Bmi, c_int, *mut c_int) -> c_int>,
}

pub type BmiRegistrationFn = unsafe extern "C" fn(*mut Bmi) -> *mut Bmi;
