# NetCDF Forcing Format

The forcing file must be a NetCDF file with:

- A variable `ids` containing location ID strings
- A variable `Time` containing epoch timestamps (int64)
- Dimensions `catchment-id` (locations) and `time` (timesteps)
- Data variables with dimensions `[catchment-id, time]`

All forcing data for a location is preloaded into memory before running that location's models.
