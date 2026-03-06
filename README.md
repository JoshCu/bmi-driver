# bmi-driver

A parallel BMI (Basic Model Interface) orchestrator that dynamically loads and couples C, Fortran, and Python hydrological models. It reads NetCDF forcing data, resolves model dependencies, handles automatic unit conversion, and runs across many geographic locations in parallel.

## Building

```bash
cargo build --release            # All adapters (C, Fortran, Python, Zarr)
cargo build --release --no-default-features   # C adapter only (fewest deps)
cargo install --path .           # Install to ~/.cargo/bin/
```

Feature flags: `fortran`, `python`, `zarr` (all enabled by default).

### System dependencies

| Library | Purpose | Install |
|---------|---------|---------|
| `libnetcdf` | Reading NetCDF forcing files | `apt install libnetcdf-dev` |
| `pkg-config` | Library discovery at build time | `apt install pkg-config` |
| Python dev headers | Only with `python` feature | `apt install python3-dev` |

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

## Documentation

| Document | Contents |
|----------|----------|
| [docs/configuration.md](docs/configuration.md) | Config field reference tables, library usage |
| [docs/module-types.md](docs/module-types.md) | bmi_c, bmi_fortran, bmi_python, SLOTH details |
| [docs/features.md](docs/features.md) | Unit conversion, variable aliases, dependency resolution, config minification |
| [docs/netcdf-forcing.md](docs/netcdf-forcing.md) | NetCDF forcing file format spec |
