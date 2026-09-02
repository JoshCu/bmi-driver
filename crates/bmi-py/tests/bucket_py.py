"""A minimal Python BMI model, mirroring tests/fixtures/bucket_bmi.c.

Used to check that a Python BMI model can be driven from inside the extension module -- Python
calling Rust calling Python, which only works because the extension links pyo3 without
auto-initialize.
"""

import numpy as np


class BucketPy:
    _INPUTS = ["precip_rate"]
    _OUTPUTS = ["Q_OUT", "STORAGE"]
    _UNITS = {"precip_rate": "m h-1", "Q_OUT": "m h-1", "STORAGE": "m"}

    def __init__(self):
        self._values = {name: np.zeros(1) for name in self._INPUTS + self._OUTPUTS}
        self._drain = 0.15
        self._time = 0.0

    def initialize(self, config):
        self._values["STORAGE"][0] = 0.01
        with open(config) as f:
            for line in f:
                if "=" not in line:
                    continue
                key, value = line.split("=", 1)
                if key.strip() == "drain_fraction":
                    self._drain = float(value)
                elif key.strip() == "initial_storage":
                    self._values["STORAGE"][0] = float(value)

    def update(self):
        self._values["STORAGE"] += self._values["precip_rate"]
        self._values["Q_OUT"][0] = self._values["STORAGE"][0] * self._drain
        self._values["STORAGE"] -= self._values["Q_OUT"]
        self._time += 1.0

    def update_until(self, then):
        while self._time < then:
            self.update()

    def finalize(self):
        pass

    def get_component_name(self):
        return "BucketPy"

    def get_input_var_names(self):
        return self._INPUTS

    def get_output_var_names(self):
        return self._OUTPUTS

    def get_input_item_count(self):
        return len(self._INPUTS)

    def get_output_item_count(self):
        return len(self._OUTPUTS)

    def get_value_ptr(self, name):
        return self._values[name]

    def set_value(self, name, values):
        self._values[name][:] = np.asarray(values).ravel()[: self._values[name].size]

    def get_var_type(self, name):
        return "float64"

    def get_var_units(self, name):
        return self._UNITS[name]

    def get_var_itemsize(self, name):
        return 8

    def get_var_nbytes(self, name):
        return 8

    def get_var_location(self, name):
        return "node"

    def get_var_grid(self, name):
        return 0

    def get_current_time(self):
        return self._time

    def get_start_time(self):
        return 0.0

    def get_end_time(self):
        return 1e9

    def get_time_step(self):
        return 1.0

    def get_time_units(self):
        return "h"

    def get_grid_rank(self, grid):
        return 0

    def get_grid_size(self, grid):
        return 1

    def get_grid_type(self, grid):
        return "scalar"
