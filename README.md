# bmi-driver

A parallel BMI (Basic Model Interface) orchestrator that dynamically loads and couples C, Fortran, and Python hydrological models. It reads NetCDF forcing data, resolves model dependencies, handles automatic unit conversion, and runs across many geographic locations in parallel.

Three ways to use it: the `bmi-driver` command, a Rust library that hands results back as vectors, and a Python package that drives BMI models directly.

## Crates

| Crate | What it is |
|-------|------------|
| [`crates/bmi-core`](crates/bmi-core) | The BMI adapters (C, Fortran, Python, SLOTH), unit conversion and resampling. No NetCDF, no CLI. |
| [`crates/bmi-driver`](crates/bmi-driver) | Configuration, forcings, the model runner and output writers. The embedding API. |
| [`crates/bmi-driver-cli`](crates/bmi-driver-cli) | The `bmi-driver` binary. |
| [`crates/bmi-py`](crates/bmi-py) | The `bmi_driver` Python package, built with maturin. Its own workspace: it enables `pyo3/extension-module`, and cargo unifying that across a workspace build would leave the CLI unable to link libpython. |

## Building

```bash
cargo build --release                    # everything but the Python module
cargo install --path crates/bmi-driver-cli   # install the CLI to ~/.cargo/bin/
maturin build -m crates/bmi-py/Cargo.toml --release   # build the Python wheel
```

Feature flags on `bmi-driver`: `fortran`, `python-embed`, `zarr`, `gpkg` (all on by default).
Use `--no-default-features` for the C adapter and CSV output alone.

### System dependencies

| Library | Purpose | Install |
|---------|---------|---------|
| `libnetcdf` | Reading NetCDF forcing files | `apt install libnetcdf-dev` |
| `pkg-config` | Library discovery at build time | `apt install pkg-config` |
| Python dev headers | Only with `python-embed` | `apt install python3-dev` |

## Usage

```
bmi-driver <data_dir> [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `-j, --jobs N` | Number of parallel worker processes (default: CPU count) |
| `--units` | Print all unit conversion info and exit |
| `--minify` | Strip unused fields from realization.json and exit |
| `--node-start N` | First location index for this node (multi-node/SLURM) |
| `--node-count N` | Number of locations for this node (0 = all remaining) |

### Data directory layout

```
<data_dir>/
  config/
    realization.json            # Model chain configuration
    *.gpkg                      # GeoPackage with location IDs
    cat_config/<Model>/{{id}}   # Per-location model configs
  forcings/
    forcings.nc                 # NetCDF forcing data
  outputs/
    bmi-driver/                 # Output files written here
```

The GeoPackage must contain a `divides` table with a `divide_id` text column.

## Configuration

Expects `<data_dir>/config/realization.json` defining the model chain, forcing source, and time range. Compatible with [ngen](https://github.com/NOAA-OWP/ngen) realization configs.

```json
{
  "global": {
    "formulations": [{
      "name": "bmi_multi",
      "params": {
        "main_output_variable": "Q_OUT",
        "modules": [{
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
        }]
      }
    }],
    "forcing": { "path": "./forcings/forcings.nc" }
  },
  "time": {
    "start_time": "2010-01-01 00:00:00",
    "end_time": "2010-01-02 00:00:00",
    "output_interval": 3600
  }
}
```

See [docs/configuration.md](docs/configuration.md) for all field reference tables and library usage examples.

## Output

Output format is set via `output_format` in the config (`"csv"`, `"netcdf"`, or `"zarr"`). Default is CSV.

- **CSV**: One file per location at `outputs/bmi-driver/<location_id>.csv`
- **NetCDF**: Single `outputs/bmi-driver/results.nc` with dimensions `[id, time]`
- **Zarr**: Single `outputs/bmi-driver/results.zarr` store with chunking `[1, n_times]` per variable

## Using it from another program

```rust
use bmi_driver::Simulation;

let mut sim = Simulation::open("/data/my_run")?;
let flows = sim.run_variable("cat-123", "Q_OUT")?;   // Vec<f64>, no files written
let units = sim.var_units("cat-123", "Q_OUT")?;      // "m h-1"
```

Relative paths in the config resolve against the data directory you pass in, never against the
process working directory, so several simulations can coexist in one program. See
[docs/embedding.md](docs/embedding.md), including what is and is not safe across threads.

## Using it from Python

```python
from bmi_driver import Model

with Model.c("libcfebmi.so", register="register_bmi_cfe") as cfe:
    cfe.initialize("cat-123.ini")
    cfe.set_value("atmosphere_water__liquid_equivalent_precipitation_rate", 0.001)
    cfe.update()
    q = cfe.get_value("Q_OUT")
```

One class for C, Fortran and Python models, and one `get_value`/`set_value` pair regardless of
whether the model stores a variable as a double, a float or an int. See [docs/python.md](docs/python.md).

## Documentation

| Document | Contents |
|----------|----------|
| [docs/embedding.md](docs/embedding.md) | Running the driver from another Rust program |
| [docs/python.md](docs/python.md) | The Python package |
| [docs/configuration.md](docs/configuration.md) | Config field reference tables, library usage |
| [docs/module-types.md](docs/module-types.md) | bmi_c, bmi_fortran, bmi_python, SLOTH details |
| [docs/features.md](docs/features.md) | Unit conversion, variable aliases, dependency resolution, config minification |
| [docs/netcdf-forcing.md](docs/netcdf-forcing.md) | NetCDF forcing file format spec |
