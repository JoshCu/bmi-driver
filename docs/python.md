# The Python package

```bash
pip install maturin
maturin develop -m crates/bmi-py/Cargo.toml --release
```

`crates/bmi-py` is a separate cargo workspace, so build it by manifest path rather than with
`-p` from the repository root.

Features: `fortran`, `python-models` and `driver` are on by default. Drop `driver` for a wheel
that does not need libnetcdf, at the cost of `Forcings` and `Simulation`.

> When `maturin build` repairs a wheel it rewrites the library names inside
> `target/release/deps/lib_bmi_driver.so` in place. Cargo then considers that file up to date and
> will not relink it, so the next build fails with "required library libhdf5_serial-<hash>.so
> could not be located". Delete the file and build again.

The point of this package is that a Python caller should not have to know whether a model is C,
Fortran or Python, nor which of `get_value_double` / `get_value_float` / `get_value_int` it wants.

## Loading a model

```python
from bmi_driver import Model

Model.c("libcfebmi.so", register="register_bmi_cfe")
Model.fortran("libsurfacebmi.so", middleware="libsurfacebmi_iso_c.so")
Model.python("lstm.bmi_lstm.bmi_LSTM")
Model.python_file("/path/to/bmi_lstm.py", "bmi_LSTM")
Model.sloth({"land_surface_water__runoff_depth": 0.0})
```

`Model.python` and `Model.python_file` need the `python-models` feature; `Model.fortran` needs
`fortran`, which is on by default.

## Using one

```python
with Model.c("libcfebmi.so", register="register_bmi_cfe") as cfe:
    cfe.initialize("cat-123.ini")
    for rain in rainfall:
        cfe.set_value("atmosphere_water__liquid_equivalent_precipitation_rate", rain)
        cfe.update()
        q = cfe.get_value("Q_OUT")       # numpy array
```

Leaving the `with` block finalizes the model. Without it, call `finalize()` yourself.

| Method | Notes |
|--------|-------|
| `initialize(config)`, `update()`, `update_until(t)`, `finalize()` | Lifecycle |
| `get_value(name)` | numpy array, in the variable's own dtype |
| `get_scalar(name)` | A Python float, for single-valued variables |
| `set_value(name, values)` | Scalar, sequence or numpy array |
| `dtype_of(name)` | `"float64"`, `"float32"` or `"int32"` |
| `get_input_var_names()`, `get_output_var_names()` | |
| `get_var_units(name)`, `get_var_type(name)`, `get_var_itemsize(name)`, `get_var_nbytes(name)`, `get_var_location(name)`, `get_var_grid(name)` | |
| `get_current_time()`, `get_start_time()`, `get_end_time()`, `get_time_step()`, `get_time_units()` | In the model's own time units |
| `time_step_seconds()` | The timestep in seconds, whatever units the model reports |
| `get_grid_rank(g)`, `get_grid_size(g)`, `get_grid_type(g)` | |

## Types

By default each variable uses the type the model reports, which is what you want almost always:
`get_value` returns `float64` for a double variable, `float32` for a float, `int32` for an int,
and `set_value` converts whatever you pass into the right one.

To pin a single type for every variable — useful for a model that misreports its own types —
pass `dtype` when constructing:

```python
m = Model.c("libmodel.so", register="register_bmi", dtype="float64")
```

## Forcings and whole runs

```python
from bmi_driver import Forcings, Simulation

sim = Simulation("/data/my_run")
out = sim.run("cat-123")           # {"Q_OUT": array([...]), ...}
sim.locations(); sim.times(); sim.total_steps; sim.var_units("cat-123", "Q_OUT")

f = Forcings("/data/my_run/forcings/forcings.nc")
f.variable_names(); f.location_ids(); f.units("RAINRATE"); f.time_step()
rain = f.get("RAINRATE", "cat-123")
```

Together these let you write the rest of the engine in Python against the real inputs, and check
it against the driver's own answer:

```python
from bmi_driver import Forcings, Model, Simulation, convert

sim = Simulation("/data/my_run")
reference = sim.run("cat-123")["Q_OUT"]

forcings = Forcings("/data/my_run/forcings/forcings.nc")
rain = forcings.get("RAINRATE", "cat-123")
factor = convert(1.0, forcings.units("RAINRATE"), "m h-1")

model = Model.c("/data/my_run/libs/libcfebmi.so", register="register_bmi_cfe")
model.initialize("/data/my_run/config/cat_config/CFE/cat-123.ini")
mine = []
for step in range(sim.total_steps + 1):
    model.set_value("precip_rate", float(rain[step]) * factor)
    model.update()
    mine.append(model.get_scalar("Q_OUT"))
model.finalize()

assert numpy.allclose(mine, reference)
```

`convert(value, from_units, to_units, timestep_seconds=None)` is the driver's own unit table.
The timestep is only needed for rate/accumulation conversions, where "mm" per step and "mm h-1"
differ by how long a step is.

## Errors and threads

BMI failures raise `RuntimeError`; bad arguments raise `ValueError`.

A `Model` holds a dynamically loaded library and is not safe to share between Python threads
unless the model library itself is. The GIL will serialize the calls, but two models built from
one Fortran library still share that library's module-level state.
