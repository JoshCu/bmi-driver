# Configuration Reference

The config file is `<data_dir>/config/realization.json`. The format is compatible with [ngen](https://github.com/NOAA-OWP/ngen) realization configs -- extra ngen-specific fields are silently ignored.

## Top-level fields

| Field | Required | Description |
|-------|----------|-------------|
| `global.formulations` | yes | Array with one `bmi_multi` formulation entry |
| `global.forcing.path` | yes | Path to NetCDF forcing file (relative to data_dir) |
| `time.start_time` | yes | Simulation start, `"YYYY-MM-DD HH:MM:SS"` |
| `time.end_time` | yes | Simulation end, `"YYYY-MM-DD HH:MM:SS"` |
| `time.output_interval` | no | Output interval in seconds (default: 3600) |

## Formulation fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | yes | Must be `"bmi_multi"` for multi-model orchestration |
| `params.modules` | yes | Array of module configs (see below) |
| `params.main_output_variable` | no | Primary output variable name |
| `params.output_variables` | no | Variables to include in output. If empty, all model outputs are written. |

## Module fields (shared across all adapter types)

| Field | Required | Description |
|-------|----------|-------------|
| `name` | yes | Adapter type: `"bmi_c"`, `"bmi_fortran"`, `"bmi_python"`, or `"bmi_c++"` |
| `params.model_type_name` | yes | Model identifier (e.g., `"CFE"`, `"NoahOWP"`, `"SLOTH"`) |
| `params.init_config` | yes | Path to model config file. `{{id}}` is replaced with the location ID at runtime. |
| `params.main_output_variable` | yes | The primary output variable of this model |
| `params.library_file` | depends | Path to shared library. Required for C and Fortran adapters. |
| `params.registration_function` | no | Name of the C registration function (default: `"register_bmi"`) |
| `params.variables_names_map` | no | Maps model input names to source variable names (from forcings or upstream models) |
| `params.model_params` | no | Key-value parameters set on the model after initialization via `set_value()` |
| `params.python_type` | no | Python class path for the Python adapter (e.g., `"lstm.bmi_lstm.bmi_LSTM"`) |

## Library usage

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
