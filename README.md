# BMI-DRIVER-RS

Basic Model Interface (BMI) driver with support for C and Fortran models.

## Usage

```rust
use bmi::{ModelRunner, BmiError};

fn main() -> Result<(), BmiError> {
    let mut runner = ModelRunner::from_config("realization.json")?;
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
