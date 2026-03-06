# Module Types

## bmi_c -- C shared libraries

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

## bmi_fortran -- Fortran shared libraries

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

## bmi_python -- Python BMI classes

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

## bmi_c++ (SLOTH) -- Built-in dummy model

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
