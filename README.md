# BMI-DRIVER-RS

Basic Model Interface (BMI) driver with support for C and Fortran models.

## Usage

```
bmi-runner <data_dir> [-j <jobs>]
```

### Arguments

- `<data_dir>` - Path to the data directory. Must contain:
  - `config/realization.json` - Model realization configuration
  - `config/<name>.gpkg` - GeoPackage with `divides` table listing location IDs
  - `forcings/` - NetCDF forcing files
- `-j, --jobs <N>` - Number of parallel worker processes (default: number of CPUs)

### Output

CSV files are written to `<data_dir>/outputs/bmi-driver/<location_id>.csv`, one per location. Each CSV contains a `Time Step`, `Time`, and one column per output variable.

### Library

```rust
use bmi_driver::{ModelRunner, BmiError};

fn main() -> Result<(), BmiError> {
    let mut runner = ModelRunner::from_config("config/realization.json")?;
    runner.initialize("cat-123")?;
    runner.run()?;

    let outputs = runner.main_outputs()?;
    println!("Completed {} timesteps", outputs.len());

    runner.finalize()?;
    Ok(())
}
```

## Adapters

- `BmiC` - C models with function pointer struct
- `BmiFortran` - Fortran models with iso_c_binding
- `BmiSloth` - Dummy model for constant values

## Structure

```
src/
├── main.rs          # CLI entry point (bmi-runner binary)
├── lib.rs           # Public exports
├── traits.rs        # Bmi trait and types
├── error.rs         # Error types
├── ffi.rs           # C FFI bindings
├── library.rs       # Dynamic library loading
├── config.rs        # Configuration parsing
├── forcings.rs      # NetCDF forcing data
├── runner.rs        # Model orchestration
└── adapters/
    ├── mod.rs       # Adapter exports
    ├── c.rs         # C BMI adapter
    ├── fortran.rs   # Fortran BMI adapter
    └── sloth.rs     # SLOTH dummy model
```
