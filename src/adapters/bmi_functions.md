# BMI functions called by each adapter

Each adapter wraps a different language runtime. The table below shows which standard BMI
functions each adapter actually invokes on the underlying model.

| BMI function | `BmiC` | `BmiFortran` | `BmiPython` |
|---|---|---|---|
| `initialize` | ✓ | ✓ | ✓ |
| `update` | ✓ | ✓ | ✓ |
| `update_until` | ✓ | ✓ | ✓ |
| `finalize` | ✓ | ✓ | ✓ |
| `get_component_name` | ✓ | ✓ | ✓ |
| `get_input_item_count` | ✓ | ✓ | ✓ |
| `get_output_item_count` | ✓ | ✓ | ✓ |
| `get_input_var_names` | ✓ | ✓ | ✓ |
| `get_output_var_names` | ✓ | ✓ | ✓ |
| `get_var_grid` | ✓ | ✓ | ✓ |
| `get_var_type` | ✓ | ✓ | ✓ |
| `get_var_units` | ✓ | ✓ | ✓ |
| `get_var_itemsize` | ✓ | ✓ | ✓ |
| `get_var_nbytes` | ✓ (cached at init, sizes get_value buffer) | ✓ (cached at init, sizes get_value buffer) | — (Python manages memory) |
| `get_var_location` | ✓ | ✓ | ✓ |
| `get_current_time` | ✓ | ✓ | ✓ |
| `get_start_time` | ✓ | ✓ | ✓ |
| `get_end_time` | ✓ | ✓ | ✓ |
| `get_time_units` | ✓ | ✓ | ✓ |
| `get_time_step` | ✓ | ✓ | ✓ |
| `get_value` | ✓ (type-erased `*mut c_void`) | — | — |
| `get_value_double` / `f64` | — | ✓ (typed fn ptr) | — |
| `get_value_float` / `f32` | — | ✓ (typed fn ptr) | — |
| `get_value_int` / `i32` | — | ✓ (typed fn ptr) | — |
| `get_value_ptr` | ✓ (runner couples through it when available) | ✓ optional `get_value_ptr_double`/`float`/`int` | ✓ + `.tolist()` |
| `set_value` | ✓ (type-erased `*mut c_void`) | — | ✓ (via numpy array) |
| `set_value_double` | — | ✓ (typed fn ptr) | — |
| `set_value_float` | — | ✓ (typed fn ptr) | — |
| `set_value_int` | — | ✓ (typed fn ptr) | — |
| `get_grid_rank` | ✓ | ✓ | ✓ |
| `get_grid_size` | ✓ | ✓ | ✓ |
| `get_grid_type` | ✓ | ✓ | ✓ |

## Functions called implicitly after `initialize`

After a successful `initialize` call, all three adapters automatically call:

1. `get_time_units` — to compute the internal `time_factor` used for unit conversion between
   model time and wall-clock seconds.
2. `get_input_var_names` + `get_output_var_names` — via `cache_types()`, which then calls
   `get_var_type` + `get_var_itemsize` + `get_var_nbytes` for every variable — to populate a
   type cache (`VarInfo`) that avoids repeated FFI round-trips during the simulation loop.
   `BmiC`/`BmiFortran` use the cached `nbytes` to size `get_value` buffers instead of calling
   `get_var_nbytes` on every timestep.

## Functions in the C FFI struct not used by the driver

The `ffi::Bmi` C struct exposes additional optional function pointers that are defined in the
full BMI specification but are not wrapped by the Rust `Bmi` trait and are therefore never
called by the driver:

- `get_value_at_indices`, `set_value_at_indices`
- `get_grid_shape`, `get_grid_spacing`, `get_grid_origin`
- `get_grid_x`, `get_grid_y`, `get_grid_z`
- `get_grid_node_count`, `get_grid_edge_count`, `get_grid_face_count`
- `get_grid_edge_nodes`, `get_grid_face_edges`, `get_grid_face_nodes`, `get_grid_nodes_per_face`

## Direct memory coupling

Before the timestep loop the runner asks each model for `get_value_ptr` on every coupled input
and output (`src/variables/binding.rs`). A pointer is first checked against the normal
`set_value`/`get_value` calls and re-queried after each `update`; once it has matched a non-zero
value (or 48 steps in a row) the runner reads and writes that memory directly and stops calling
`get_value`/`set_value` for the variable. Variables without a usable pointer — including every
model loaded through the NOAA-OWP ISO C Fortran middleware, whose `get_value_ptr_*` always fail —
stay on the calls. Disable per module with `"use_value_ptr": false`.
