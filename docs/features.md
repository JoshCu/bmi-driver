# Features

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
bmi-driver <data_dir> --units
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

## Config minification

Use `--minify` to strip a realization.json down to only the fields bmi-driver reads:

```bash
bmi-driver <data_dir> --minify
```

This removes ngen-specific fields (`routing`, `forcing.provider`, `allow_exceed_end_time`, `fixed_time_step`, `uses_forcing_file`, etc.), unknown keys not in the config schema, and empty default-valued fields.
