# Embedding the driver

`bmi-driver` is a library as well as a binary. Another Rust program can run a model chain and get
the results back in memory, with nothing written to disk.

```toml
[dependencies]
bmi-driver = { git = "https://github.com/JoshCu/bmi-driver" }
```

## Running one location

```rust
use bmi_driver::Simulation;

let mut sim = Simulation::open("/data/my_run")?;      // <data_dir>/config/realization.json
let flows: Vec<f64> = sim.run_variable("cat-123", "Q_OUT")?;
```

Or every variable the config selects:

```rust
let out = sim.run("cat-123")?;
let q = out.get("Q_OUT").unwrap();
for (t, v) in out.times().zip(q) {
    println!("{t} {v}");
}
```

For a single run and nothing else, `run_location(data_dir, location, variable)` does the whole
thing in one call. When looping over locations, build a `Simulation` and reuse it: the config is
parsed once and the forcing file opened once, while the models are reloaded per location because
that is what their own lifecycle requires.

| Method | Purpose |
|--------|---------|
| `Simulation::open(data_dir)` | Use `<data_dir>/config/realization.json` |
| `Simulation::with_config(data_dir, config)` | An explicit config; relative paths still resolve against `data_dir` |
| `run(location)` | Every selected variable, as a `RunOutput` |
| `run_variable(location, name)` | One variable |
| `var_units(location, name)` | The model's own units for a variable |
| `locations()` | Location ids from the hydrofabric GeoPackage (`gpkg` feature) |
| `total_steps()`, `start_epoch()`, `output_interval()` | The simulation period, known before any run |
| `set_output_variables(..)` | Override the config's `output_variables` |
| `runner()`, `runner_mut()` | The underlying `ModelRunner`, for access to individual models |

## Units

Ask rather than assume. `var_units` reports what the model itself says, and `bmi_driver::units`
converts:

```rust
let units = sim.var_units("cat-123", "Q_OUT")?;             // e.g. "m h-1"
let (conv, _) = bmi_driver::units::find_conversion_or_identity(&units, "m s-1", Some(3600.0));
let per_second = conv.convert(depth_per_hour);
```

A model reporting `mm h-1` where you assumed `m h-1` is a factor of a thousand, and it will not
look obviously wrong in the output.

## Paths

Every relative path in a realization config — the forcing file, `init_config`, `library_file` —
resolves against the data directory passed to `Simulation` or `ModelRunner`. Nothing in the
library changes the process working directory, so simulations over different data directories can
coexist in one program.

## Threads

Build one `Simulation` per worker thread, inside that thread. `Simulation` is deliberately not
`Send`: it owns dynamically loaded model libraries, and moving one between threads would suggest
a safety this cannot offer.

What is safe, and what is not:

- **The forcing file is safe.** It is opened once per process and shared behind a mutex, with each
  reader keeping its own metadata copy and its own location cache. Only the bulk read of a new
  location takes the lock. (Two independent NetCDF handles on one file will crash the process:
  libnetcdf's HDF5 backend shares per-file state between them and tears it down when the first
  closes. Hence the shared handle.)
- **The models are as thread-safe as the model libraries are.** Two instances of a C model that
  keeps all its state in the struct the registration function allocates are fine. A Fortran model
  with module-level `save` variables is not, and neither is anything using file-scope statics —
  the two instances will quietly share state. Python models serialize on the GIL.

If you do not know which kind you have, use separate processes, as the CLI does.

## Errors

Everything returns `BmiResult<T>`, an alias for `Result<T, BmiError>`. `BmiError` implements
`std::error::Error`, so `?` into `anyhow::Result` works.
