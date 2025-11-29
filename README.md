# BMI-RS: Rust Bindings for the Basic Model Interface

A safe, ergonomic Rust adapter for loading and interacting with BMI (Basic Model Interface) compliant models compiled as C shared libraries.

## Features

- **Dynamic Loading**: Load BMI models from `.so`, `.dylib`, or `.dll` files at runtime
- **Safe Wrappers**: All unsafe FFI calls are encapsulated with proper error handling
- **Full BMI 2.0 Support**: Implements the complete BMI specification including:
  - Initialize/Update/Finalize lifecycle
  - Time information queries
  - Variable getting and setting
  - Grid information
- **Ergonomic API**: Rust-native types (`String`, `Vec<T>`) instead of raw C pointers

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
bmi-rs = { path = "path/to/bmi-rust" }
```

Or copy the crate into your workspace.

## Quick Start

```rust
use bmi::{BmiModel, BmiError};

fn main() -> Result<(), BmiError> {
    // Load model from shared library
    let mut model = BmiModel::load(
        "my_model",                    // Descriptive name
        "/path/to/libmodel.so",        // Shared library path
        "register_bmi",                // Registration function name
    )?;

    // Initialize with config file
    model.initialize("/path/to/config.yml")?;

    // Get model info
    println!("Model: {}", model.get_component_name()?);
    println!("Time units: {}", model.get_time_units()?);

    // Run simulation
    while model.get_current_time()? < model.get_end_time()? {
        model.update()?;
        
        // Read output
        let values: Vec<f64> = model.get_value("output_var")?;
        println!("Values: {:?}", values);
    }

    // Cleanup (also automatic on drop)
    model.finalize()?;
    
    Ok(())
}
```

## API Overview

### Model Lifecycle

```rust
// Load (does not initialize)
let mut model = BmiModel::load(name, lib_path, reg_func)?;

// Initialize
model.initialize(config_path)?;

// Step forward
model.update()?;                    // Single step
model.update_until(target_time)?;   // Step to specific time

// Cleanup
model.finalize()?;
```

### Querying Model Information

```rust
// Component info
let name = model.get_component_name()?;

// Time info
let start = model.get_start_time()?;
let end = model.get_end_time()?;
let current = model.get_current_time()?;
let step = model.get_time_step()?;
let units = model.get_time_units()?;

// Variable lists
let inputs = model.get_input_var_names()?;
let outputs = model.get_output_var_names()?;

// Variable info
let grid_id = model.get_var_grid("var_name")?;
let var_type = model.get_var_type("var_name")?;
let units = model.get_var_units("var_name")?;
let nbytes = model.get_var_nbytes("var_name")?;
```

### Getting and Setting Values

```rust
// Get values as typed vector
let values: Vec<f64> = model.get_value("temperature")?;
let values: Vec<i32> = model.get_value("cell_ids")?;

// Get values at specific indices
let indices = vec![0, 5, 10];
let subset: Vec<f64> = model.get_value_at_indices("temperature", &indices)?;

// Set values
model.set_value("precipitation", &[0.5, 0.3, 0.8])?;

// Set at specific indices
model.set_value_at_indices("forcing", &[0, 2], &[1.0, 1.5])?;
```

### Grid Information

```rust
let grid_id = model.get_var_grid("elevation")?;

// Basic grid info
let grid_type = model.get_grid_type(grid_id)?;
let rank = model.get_grid_rank(grid_id)?;
let size = model.get_grid_size(grid_id)?;

// For uniform rectilinear grids
let shape = model.get_grid_shape(grid_id)?;
let spacing = model.get_grid_spacing(grid_id)?;
let origin = model.get_grid_origin(grid_id)?;

// For non-uniform grids
let x_coords = model.get_grid_x(grid_id)?;
let y_coords = model.get_grid_y(grid_id)?;
```

## Error Handling

All operations return `Result<T, BmiError>`. Error variants include:

- `BmiError::LibraryLoad` - Failed to load shared library
- `BmiError::RegistrationFunctionNotFound` - Registration function not in library
- `BmiError::BmiFunctionFailed` - BMI function returned failure code
- `BmiError::FunctionNotImplemented` - Function pointer is null
- `BmiError::NotInitialized` - Operation requires initialized model
- `BmiError::AlreadyInitialized` - Cannot re-initialize
- `BmiError::ConfigFileNotFound` - Config file doesn't exist

## Building BMI C Libraries

Your C library must export a registration function that populates the BMI struct:

```c
#include "bmi.h"

// Your model's implementation functions
static int my_initialize(Bmi *self, const char *config) { ... }
static int my_update(Bmi *self) { ... }
// ... etc

// Registration function (exported)
Bmi* register_bmi_mymodel(Bmi* model) {
    model->initialize = my_initialize;
    model->update = my_update;
    model->finalize = my_finalize;
    // ... set all function pointers
    return model;
}
```

Compile as a shared library:

```bash
# Linux
gcc -shared -fPIC -o libmymodel.so mymodel.c

# macOS
gcc -shared -fPIC -o libmymodel.dylib mymodel.c
```

## Command-Line Example

The crate includes an example binary:

```bash
cargo run --bin bmi-example -- ./libmodel.so register_bmi config.yml
```

## Architecture

```
src/
├── lib.rs          # Public API exports
├── adapter.rs      # BmiModel implementation
├── bmi_ffi.rs      # FFI bindings (Bmi struct)
├── error.rs        # Error types
└── main.rs         # Example CLI application
```

## License

MIT License - see LICENSE file.

## References

- [BMI Documentation](https://bmi.readthedocs.io/)
- [BMI C Specification](https://github.com/csdms/bmi-c)
- [CSDMS BMI](https://csdms.colorado.edu/wiki/BMI)
