# BMI functions called by each adapter

Each adapter wraps a different language runtime. The table below shows which standard BMI
functions each adapter actually invokes on the underlying model.

| BMI function | `BmiC` | `BmiFortran` | `BmiPython` | `BmiSloth` |
|---|---|---|---|---|
| `initialize` | ✓ | ✓ | ✓ | ✓ (ignores config path) |
| `update` | ✓ | ✓ | ✓ | ✓ |
| `update_until` | ✓ | ✓ | ✓ | ✓ |
| `finalize` | ✓ | ✓ | ✓ | ✓ |
| `get_component_name` | ✓ | ✓ | ✓ | ✓ (constant) |
| `get_input_item_count` | ✓ | ✓ | ✓ | ✓ (always 0) |
| `get_output_item_count` | ✓ | ✓ | ✓ | ✓ |
| `get_input_var_names` | ✓ | ✓ | ✓ | ✓ (always empty) |
| `get_output_var_names` | ✓ | ✓ | ✓ | ✓ |
| `get_var_grid` | ✓ | ✓ | ✓ | ✓ (always 0) |
| `get_var_type` | ✓ | ✓ | ✓ | ✓ |
| `get_var_units` | ✓ | ✓ | ✓ | ✓ |
| `get_var_itemsize` | ✓ | ✓ | ✓ | ✓ |
| `get_var_nbytes` | ✓ (sizes get_value buffer) | ✓ (sizes get_value buffer) | — (Python manages memory) | ✓ |
| `get_var_location` | ✓ | ✓ | ✓ | ✓ |
| `get_current_time` | ✓ | ✓ | ✓ | ✓ |
| `get_start_time` | ✓ | ✓ | ✓ | ✓ (always 0.0) |
| `get_end_time` | ✓ | ✓ | ✓ | ✓ (always f64::MAX) |
| `get_time_units` | ✓ | ✓ | ✓ | ✓ (always "s") |
| `get_time_step` | ✓ | ✓ | ✓ | ✓ |
| `get_value` | ✓ (type-erased `*mut c_void`) | — | — | — |
| `get_value_double` / `f64` | — | ✓ (typed fn ptr) | — | ✓ |
| `get_value_float` / `f32` | — | ✓ (typed fn ptr) | — | ✓ |
| `get_value_int` / `i32` | — | ✓ (typed fn ptr) | — | ✓ |
| `get_value_ptr` | — | — | ✓ + `.tolist()` | — |
| `set_value` | ✓ (type-erased `*mut c_void`) | — | ✓ (via numpy array) | ✓ |
| `set_value_double` | — | ✓ (typed fn ptr) | — | — |
| `set_value_float` | — | ✓ (typed fn ptr) | — | — |
| `set_value_int` | — | ✓ (typed fn ptr) | — | — |
| `get_grid_rank` | ✓ | ✓ | ✓ | ✓ (always 1) |
| `get_grid_size` | ✓ | ✓ | ✓ | ✓ (always 1) |
| `get_grid_type` | ✓ | ✓ | ✓ | ✓ (always "scalar") |

## Functions called implicitly after `initialize`

After a successful `initialize` call, all three foreign adapters (`BmiC`, `BmiFortran`,
`BmiPython`) automatically call:

1. `get_time_units` — to compute the internal `time_factor` used for unit conversion between
   model time and wall-clock seconds.
2. `get_input_var_names` + `get_output_var_names` — via `cache_types()`, which then calls
   `get_var_type` + `get_var_itemsize` for every variable — to populate a type cache that
   avoids repeated FFI round-trips during the simulation loop.

`BmiSloth` populates its type cache from the parsed `model_params` at `configure()` time, so
it does not need these extra calls after `initialize`.

## Functions in the C FFI struct not used by the driver

The `ffi::Bmi` C struct exposes additional optional function pointers that are defined in the
full BMI specification but are not wrapped by the Rust `Bmi` trait and are therefore never
called by the driver:

- `get_value_ptr`, `get_value_at_indices`, `set_value_at_indices`
- `get_grid_shape`, `get_grid_spacing`, `get_grid_origin`
- `get_grid_x`, `get_grid_y`, `get_grid_z`
- `get_grid_node_count`, `get_grid_edge_count`, `get_grid_face_count`
- `get_grid_edge_nodes`, `get_grid_face_edges`, `get_grid_face_nodes`, `get_grid_nodes_per_face`
