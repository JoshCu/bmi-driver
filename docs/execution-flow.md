# Execution Flow

A high-level walkthrough of how `bmi-runner` processes a simulation from start to finish.

## Startup

The binary entry point is `main()` in `src/main.rs`. It:

1. Parses CLI arguments (data directory, `--jobs`, `--progress`, etc.)
2. Loads optional user config from `~/.config/bmi-driver/config.toml`
3. Changes the working directory to the provided `data_dir`
4. Reads `config/realization.json` for the simulation definition

Based on the presence of hidden `--worker-start`/`--worker-end` flags, the process either runs as a **parent** (the default) or a **worker** (spawned by the parent).

## Parent Mode

The parent process coordinates the simulation across multiple worker subprocesses.

### Pre-flight Validation

Before spawning workers, the parent creates a temporary `ModelRunner` and initializes it with the first location. This:

- Validates the config and model libraries can actually load
- Discovers output variable names for output file pre-creation
- Runs `find_missing_mappings()` to detect unmapped model inputs and suggest aliases from AORC/CSDMS standard names
- Prints unit conversion info so the user can verify correctness

### Location Discovery

Location IDs are loaded from a GeoPackage file (`config/*.gpkg`) by querying the `divides` table. For multi-node runs (e.g. SLURM), the location list is sliced according to `--node-start` and `--node-count`.

### Worker Spawning

The parent divides locations into roughly equal chunks and spawns N worker subprocesses (default: one per CPU core). Each worker is re-invoked as the same binary with hidden flags:

```
bmi-runner <data_dir> --worker-start 0 --worker-end 50
```

Workers report progress by printing a count to stdout after each completed location. The parent reads these counts to update `indicatif` progress bars. Workers inherit stderr from the parent, so all warnings and errors appear in the parent's terminal.

### Finalization

After all workers complete, the parent handles any output merging. For NetCDF output, each worker writes a temporary file that the parent merges into a single `results.nc`. For Zarr output, workers write directly to a shared pre-allocated store, so no merge is needed.

## Worker Mode

Each worker processes its assigned slice of locations sequentially.

### Per-Location Loop

For each location ID in its assigned range, the worker:

1. **Initializes** the `ModelRunner` for that location
2. **Runs** all timesteps
3. **Writes** output (CSV file, NetCDF chunk, or Zarr slice)
4. **Finalizes** the runner (cleans up model state)
5. **Reports** progress to the parent via stdout

## ModelRunner Initialization

`ModelRunner` (`src/runner.rs`) is the core orchestrator. Initialization for a location involves:

### Loading Forcing Data

The NetCDF forcing file is opened and all variables for the current location are loaded into memory at once (`forcings.preload_location()`). Variable names from the forcing file are registered as available data sources.

### Resolving Model Dependencies

Models are loaded in dependency order, not config order. The runner inspects each module's `variables_names_map` (which maps model input names to source variable names) and performs an iterative topological sort:

1. Build a list of all modules and their input requirements
2. Find a module whose inputs are all already available (from forcings or previously loaded models)
3. Load that module, register its outputs as available
4. Repeat until all modules are loaded (or error on circular dependencies)

### Loading a Model

For each module, the runner:

1. **Creates an adapter** based on the module type:
   - `BmiC` -- loads a C shared library via `dlopen(RTLD_GLOBAL)` and calls a registration function to populate BMI function pointers
   - `BmiFortran` -- loads a Fortran shared library, optionally with a C-Fortran middleware layer
   - `BmiPython` -- embeds a Python interpreter and imports a BMI class
   - `BmiSloth` -- a synthetic model returning constant values (no library needed)
2. **Calls `initialize()`** on the adapter with the config path (with `{{id}}` replaced by the location ID)
3. **Sets model parameters** from `model_params` in the config
4. **Queries the model's timestep** (`get_time_step()`) to know its native dt
5. **Builds unit conversions** by comparing the units reported by the source (forcing or upstream model) with the units expected by this model's inputs

## Timestep Execution

`ModelRunner::run()` iterates through all models in dependency order. For each model, it runs through all of that model's timesteps:

```
for each model (in dependency order):
    for each timestep:
        1. Fetch input values
        2. Convert units
        3. Set values on the model
        4. Call model.update()
        5. Collect output values
```

### Fetching Input Values

Each model input is mapped to a source variable (a forcing field or an upstream model output) via `variables_names_map`. The value is retrieved by `get_var_resampled()`, which handles the case where the source and destination have different timesteps:

- **Same timestep**: direct array index lookup
- **Downsampling** (source is coarser than destination): either repeat the value or linearly interpolate, controlled by the module's `downsample_mode`
- **Upsampling** (source is finer than destination): aggregate multiple source values using the module's `upsample_mode` (mean, min, max, or mode)

### Unit Conversion

After fetching, each value passes through a linear conversion: `value = value * scale + offset`. The conversion factors are pre-computed during initialization by comparing source and destination unit strings. The unit system (`src/variables/units.rs`) handles notation variants like `mm/s`, `mm s^-1`, `mm s-1`, and temperature offsets (K/C/F).

### Output Collection

After each `model.update()`, the runner calls `get_value()` for each of the model's output variables and appends the result to per-variable vectors. Once all models have run, outputs are resampled to the configured `output_interval` if their native timestep differs.

## Output Writing

The runner supports three output formats, all implementing the `DivideDataStore` trait:

| Format | File | Behavior |
|--------|------|----------|
| **CSV** | `src/output/csv.rs` | One file per location at `<data_dir>/outputs/bmi-driver/<loc_id>.csv` with columns for time and each output variable |
| **NetCDF** | `src/output/netcdf.rs` | Each worker writes a temporary `.nc` file; the parent merges them into `results.nc` |
| **Zarr** | `src/output/zarr.rs` | Parent pre-creates the store with all dimensions; workers write slices in parallel |

## Summary Diagram

```
bmi-runner <data_dir>
│
├── Parse args & load realization.json
├── Query locations from *.gpkg
├── Pre-flight: validate config with first location
├── Pre-create output store (if Zarr/NetCDF)
│
├── Spawn worker 0: locations[0..50]
├── Spawn worker 1: locations[50..100]
├── Spawn worker N: locations[...]
│   │
│   └── Per location:
│       ├── Load forcings from NetCDF
│       ├── Resolve model dependency order
│       ├── Load models (dlopen / Python import / Sloth)
│       │   └── Build unit conversion tables
│       ├── Run timestep loop
│       │   ├── Fetch & resample inputs
│       │   ├── Convert units
│       │   ├── Set values → update() → get outputs
│       │   └── Repeat for all models, all steps
│       ├── Resample outputs to output_interval
│       └── Write to CSV / NetCDF / Zarr
│
├── Collect progress from workers
├── Merge output files (if NetCDF)
└── Done
```
