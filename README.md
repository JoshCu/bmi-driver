# bmi-driver

A parallel BMI (Basic Model Interface) orchestrator that dynamically loads and couples C, Fortran, and Python hydrological models. It reads NetCDF forcing data, resolves model dependencies, handles automatic unit conversion, and runs across many geographic locations in parallel.

The binary is named `bmi-runner`.

## Building

### Default (C adapter only)

```bash
cargo build --release
```

This builds with support for C shared library models and the built-in SLOTH dummy model.

### With Fortran support

```bash
cargo build --release --features fortran
```

Enables the `bmi_fortran` adapter for loading Fortran shared libraries. No additional build dependencies are required; a Fortran runtime must be available at execution time.

### With Python support

```bash
cargo build --release --features python
```

Enables the `bmi_python` adapter for loading Python BMI models via [PyO3](https://pyo3.rs). Requires Python development headers at build time (e.g., `python3-dev`). The Python environment must have the target BMI packages installed at runtime.

### All features

```bash
cargo build --release --features fortran,python
```

### Install

```bash
cargo install --path . --features fortran,python
```

This installs `bmi-runner` to `~/.cargo/bin/`.

### System dependencies

| Library | Purpose |
|---------|---------|
| `libnetcdf` | Reading NetCDF forcing files |
| `libsqlite3` | Reading location IDs from GeoPackage (.gpkg) |
| `libm` | Math functions for loaded models |
| Python dev headers | Only if building with `--features python` |

Model shared libraries (`.so`) must be accessible at the paths specified in the config.

## Usage

```
bmi-runner <data_dir> [OPTIONS]
```

### Options

| Flag | Description |
|------|-------------|
| `-j, --jobs N` | Number of parallel worker processes (default: CPU count) |
| `--units` | Print all unit conversion info and exit without running |
| `--minify` | Strip unused fields from realization.json and exit |

### Data directory layout

```
<data_dir>/
  config/
    realization.json            # Model chain configuration
    *.gpkg                      # GeoPackage with location IDs
  config/cat_config/
    <ModelName>/{{id}}.input    # Per-location model configs
  forcings/
    forcings.nc                 # NetCDF forcing data
  outputs/
    bmi-driver/                 # Output CSVs written here
      <location_id>.csv
```

The GeoPackage must contain a `divides` table with a `divide_id` text column listing all location IDs.

### How it runs

1. **Parent process** reads config, queries location IDs from the GeoPackage, checks for unmapped model inputs (offering to auto-add suggested mappings), and prints active unit conversions.
2. Parent divides locations into chunks and spawns N worker subprocesses.
3. **Each worker** iterates its assigned locations: initializes models, runs all timesteps, writes a CSV per location, and reports progress counts to the parent via stdout.
4. Parent displays nested progress bars (overall + per-worker) using the reported counts.

### Library usage

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

## Configuration

The config file is `<data_dir>/config/realization.json`. It defines the model chain, forcing data source, and time range. The format is compatible with [ngen](https://github.com/NOAA-OWP/ngen) realization configs -- extra ngen-specific fields are silently ignored.

### Minimal example

```json
{
  "global": {
    "formulations": [
      {
        "name": "bmi_multi",
        "params": {
          "main_output_variable": "Q_OUT",
          "output_variables": ["Q_OUT"],
          "modules": [
            {
              "name": "bmi_c",
              "params": {
                "model_type_name": "CFE",
                "library_file": "/path/to/libcfebmi.so",
                "init_config": "./config/cat_config/CFE/{{id}}.ini",
                "registration_function": "register_bmi_cfe",
                "main_output_variable": "Q_OUT",
                "variables_names_map": {
                  "atmosphere_water__liquid_equivalent_precipitation_rate": "precip_rate"
                }
              }
            }
          ]
        }
      }
    ],
    "forcing": {
      "path": "./forcings/forcings.nc"
    }
  },
  "time": {
    "start_time": "2010-01-01 00:00:00",
    "end_time": "2010-01-02 00:00:00",
    "output_interval": 3600
  }
}
```

### Top-level fields

| Field | Required | Description |
|-------|----------|-------------|
| `global.formulations` | yes | Array with one `bmi_multi` formulation entry |
| `global.forcing.path` | yes | Path to NetCDF forcing file (relative to data_dir) |
| `time.start_time` | yes | Simulation start, `"YYYY-MM-DD HH:MM:SS"` |
| `time.end_time` | yes | Simulation end, `"YYYY-MM-DD HH:MM:SS"` |
| `time.output_interval` | no | Output interval in seconds (default: 3600) |

### Formulation fields

| Field | Required | Description |
|-------|----------|-------------|
| `name` | yes | Must be `"bmi_multi"` for multi-model orchestration |
| `params.modules` | yes | Array of module configs (see below) |
| `params.main_output_variable` | no | Primary output variable name |
| `params.output_variables` | no | Variables to include in output CSV. If empty, all model outputs are written. |

### Module fields (shared across all adapter types)

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
| `params.python_type` | no | Python class path for the Python adapter (see below) |

## Module types

### bmi_c -- C shared libraries

Loads a C shared library via `dlopen` with `RTLD_GLOBAL` (so symbols are shared across libraries). Looks up the registration function by name, calls it to populate a struct of BMI function pointers, then calls through those pointers for all BMI operations.

```json
{
  "name": "bmi_c",
  "params": {
    "model_type_name": "CFE",
    "library_file": "/path/to/libcfebmi.so",
    "init_config": "./config/cat_config/CFE/{{id}}.ini",
    "registration_function": "register_bmi_cfe",
    "main_output_variable": "Q_OUT",
    "variables_names_map": {
      "atmosphere_water__liquid_equivalent_precipitation_rate": "QINSUR",
      "water_potential_evaporation_flux": "EVAPOTRANS"
    }
  }
}
```

The shared library must export a registration function matching this signature:

```c
Bmi* register_bmi_cfe(Bmi* model);
```

This function fills in the `Bmi` struct's function pointers and returns the pointer. The default function name is `register_bmi` if `registration_function` is omitted.

### bmi_fortran -- Fortran shared libraries

**Requires:** `--features fortran`

Loads Fortran BMI models. The adapter supports two modes:

1. **Single library** -- the `.so` implements both the registration function and the BMI interface directly.
2. **With middleware** -- a separate C-to-Fortran middleware library translates C-calling-convention calls to Fortran. The runner auto-detects a middleware library if one exists alongside the model library.

The adapter passes an opaque handle pointer as the first argument to all BMI calls, matching the convention used by Fortran BMI implementations.

```json
{
  "name": "bmi_fortran",
  "params": {
    "model_type_name": "NoahOWP",
    "library_file": "/path/to/libsurfacebmi.so",
    "init_config": "./config/cat_config/NOAH-OWP-M/{{id}}.input",
    "main_output_variable": "QINSUR",
    "variables_names_map": {
      "PRCPNONC": "precip_rate",
      "SFCTMP": "TMP_2maboveground",
      "SFCPRS": "PRES_surface",
      "Q2": "SPFH_2maboveground",
      "UU": "UGRD_10maboveground",
      "VV": "VGRD_10maboveground",
      "LWDN": "DLWRF_surface",
      "SOLDN": "DSWRF_surface"
    }
  }
}
```

### bmi_python -- Python BMI classes

**Requires:** `--features python`

Loads a Python class that implements the BMI interface. Values are exchanged via numpy arrays. There are two ways to specify the Python class:

**Option 1: `python_type` (preferred)** -- a dotted module path where the last component is the class name:

```json
{
  "name": "bmi_python",
  "params": {
    "model_type_name": "LSTM",
    "python_type": "bmi_lstm.bmi_LSTM",
    "init_config": "./config/cat_config/LSTM/{{id}}.yml",
    "main_output_variable": "land_surface_water__runoff_volume_flux",
    "variables_names_map": {
      "atmosphere_water__liquid_equivalent_precipitation_rate": "precip_rate",
      "land_surface_air__temperature": "TMP_2maboveground"
    }
  }
}
```

This imports `bmi_lstm` and instantiates `bmi_LSTM()`. The package must be installed or on `PYTHONPATH`.

**Option 2: `library_file` + `registration_function`** -- loads a `.py` file directly:

```json
{
  "name": "bmi_python",
  "params": {
    "model_type_name": "MyModel",
    "library_file": "/path/to/my_model.py",
    "registration_function": "MyBmiClass",
    "init_config": "./config/cat_config/MyModel/{{id}}.yml",
    "main_output_variable": "output_var"
  }
}
```

This reads the file, adds its parent directory to `sys.path`, imports the module by filename, and instantiates `MyBmiClass()`.

### bmi_c++ (SLOTH) -- Built-in dummy model

The `bmi_c++` adapter name with `model_type_name: "SLOTH"` activates the built-in SLOTH (Simple Lightweight Output Testing Handler) model. SLOTH returns constant values and requires no shared library. It is configured entirely through `model_params`.

Variables are defined using a special string format:

```
"variable_name(count,type,units,location)": value
```

| Component | Description |
|-----------|-------------|
| `variable_name` | The BMI output variable name |
| `count` | Array size (typically `1` for scalar) |
| `type` | Data type: `double`, `float`, or `int` |
| `units` | Unit string (e.g., `m`, `1`, `mm/s`) |
| `location` | BMI grid location (e.g., `node`) |
| `value` | Constant value returned for all timesteps |

```json
{
  "name": "bmi_c++",
  "params": {
    "model_type_name": "SLOTH",
    "init_config": "/dev/null",
    "main_output_variable": "z",
    "model_params": {
      "sloth_ice_fraction_schaake(1,double,m,node)": 0.0,
      "sloth_ice_fraction_xinanjiang(1,double,1,node)": 0.0,
      "sloth_soil_moisture_profile(1,double,1,node)": 0.0
    }
  }
}
```

## Model dependency resolution

Models are loaded in dependency order, not config order. The runner uses `variables_names_map` to determine dependencies:

- The **keys** are the model's input variable names.
- The **values** are source variable names (from forcings or upstream model outputs).

The runner performs an iterative topological sort:

1. Start with all forcing variables as available.
2. Find a module whose `variables_names_map` values are all satisfied by available variables.
3. Load that module, add its output variables to the available set.
4. Repeat until all modules are loaded.

This means modules can be listed in any order in the config. If a circular dependency or missing variable is detected, the runner reports an error.

## Unit conversion

The runner automatically converts units between source variables and model inputs. When a model input is mapped to a source, the runner reads both sides' unit strings and builds a linear conversion (`output = input * scale + offset`).

### Supported conversions

| Category | Examples |
|----------|----------|
| Length | m, mm, cm, km, ft, in |
| Pressure | Pa, kPa, hPa, mb, atm, bar |
| Temperature | K, C, F (with offset handling for K/C/F conversions) |
| Rates | mm/s, mm/h, m/s, m s^-1, mm s-1 |
| Mass flux | kg m^-2 s^-1 to/from mm/s (assumes water density 1000 kg/m^3) |
| Dimensionless | `""`, `"1"`, `"-"`, `"m/m"`, `"none"` |

### Notation variants

All of these are recognized as equivalent:

- `mm s^-1`, `mm/s`, `mm s-1`
- `kg m^-2 s^-1`, `kg/m^2/s`, `kg m-2 s-1`
- `W m^-2`, `W/m^2`, `W m-2`

Use `--units` to print all active conversions for debugging:

```bash
bmi-runner <data_dir> --units
```

If units are unknown or incompatible, the runner falls back to an identity conversion (no change) and prints a warning.

## Variable aliases

The runner knows common aliases between AORC forcing field names and CSDMS standard names. When it detects unmapped model inputs that match an available variable under a different name, it suggests adding the mapping.

| Short name | CSDMS standard name |
|------------|-------------------|
| `precip_rate` | `atmosphere_water__liquid_equivalent_precipitation_rate` |
| `TMP_2maboveground` | `land_surface_air__temperature` |
| `UGRD_10maboveground` | `land_surface_wind__x_component_of_velocity` |
| `VGRD_10maboveground` | `land_surface_wind__y_component_of_velocity` |
| `DLWRF_surface` | `land_surface_radiation__incoming_longwave_flux` |
| `DSWRF_surface` | `land_surface_radiation__incoming_shortwave_flux` |
| `PRES_surface` | `land_surface_air__pressure` |
| `SPFH_2maboveground` | `land_surface_air__specific_humidity` |
| `APCP_surface` | `land_surface_water__precipitation_volume_flux` |

On the first run, if unmapped inputs are found, the runner prompts:

```
Found unmapped model inputs that match available variables:
  [1] CFE: "atmosphere_water__liquid_equivalent_precipitation_rate" <- "QINSUR"

Add these mappings to realization.json? [y/N]
```

Answering `y` updates the config file in place.

## Config minification

Use `--minify` to strip a realization.json down to only the fields bmi-driver reads:

```bash
bmi-runner <data_dir> --minify
```

This removes:
- ngen-specific fields (`routing`, `forcing.provider`, `allow_exceed_end_time`, `fixed_time_step`, `uses_forcing_file`, etc.)
- Unknown keys not in the config schema
- Empty default-valued fields

The minified config is still valid for subsequent runs.

## NetCDF forcing format

The forcing file must be a NetCDF file with:

- A variable `ids` containing location ID strings
- A variable `Time` containing epoch timestamps (int64)
- Dimensions `catchment-id` (locations) and `time` (timesteps)
- Data variables with dimensions `[catchment-id, time]`

All forcing data for a location is preloaded into memory before running that location's models.

## Output

Each location produces a CSV at `<data_dir>/outputs/bmi-driver/<location_id>.csv`:

```csv
Time Step,Time,Q_OUT,EVAPOTRANS,QINSUR,RAIN_RATE
0,2010-01-01 00:00:00,0.000000000,0.000000000,0.000000000,0.000000000
1,2010-01-01 01:00:00,0.000012345,0.000000100,0.000001234,0.000000500
...
```

Columns are determined by `output_variables` in the formulation params. If `output_variables` is empty, all model output variables are included.

## Project structure

```
src/
├── main.rs          # CLI entry point (bmi-runner binary)
├── lib.rs           # Public exports
├── traits.rs        # Bmi trait and types
├── error.rs         # Error types
├── ffi.rs           # C FFI struct definition
├── library.rs       # Dynamic library loading (dlopen/dlsym)
├── config.rs        # Configuration parsing
├── forcings.rs      # NetCDF forcing data
├── runner.rs        # Model orchestration and dependency resolution
├── units.rs         # Automatic unit conversion
├── aliases.rs       # AORC ↔ CSDMS variable name aliases
└── adapters/
    ├── mod.rs       # Adapter exports
    ├── c.rs         # C BMI adapter
    ├── fortran.rs   # Fortran BMI adapter (feature-gated)
    ├── python.rs    # Python BMI adapter (feature-gated)
    └── sloth.rs     # SLOTH dummy model
```
