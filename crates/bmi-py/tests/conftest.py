"""Builds the same fixtures the Rust integration tests use: a synthetic C BMI model, a NetCDF
forcing file and a realization config, so these tests need no model libraries installed."""

import json
import shutil
import subprocess
import sqlite3
from pathlib import Path

import numpy as np
import pytest

LOCATIONS = ["cat-1", "cat-2", "cat-3"]
N_FORCING_STEPS = 49
START_EPOCH = 1_262_304_000  # 2010-01-01 00:00:00 UTC
DRAIN_FRACTION = 0.15
INITIAL_STORAGE = 0.01

FIXTURE_C = Path(__file__).resolve().parents[2] / "bmi-driver/tests/fixtures/bucket_bmi.c"


def precip_mm_per_hour(loc_idx, step):
    return np.float32((1.0 + loc_idx) * 0.5 + (step % 7) * 0.25)


def temperature_kelvin(loc_idx, step):
    return np.float32(278.15 + loc_idx + (step % 5))


def expected_q_out(loc_idx, steps):
    storage = INITIAL_STORAGE
    out = []
    for step in range(steps):
        storage += float(precip_mm_per_hour(loc_idx, step)) * 0.001
        q = storage * DRAIN_FRACTION
        storage -= q
        out.append(q)
    return np.array(out)


def _build_library(out_dir):
    lib = out_dir / "libbucketbmi.so"
    for cc in ("cc", "gcc", "clang"):
        if shutil.which(cc) is None:
            continue
        r = subprocess.run([cc, "-shared", "-fPIC", "-O1", "-o", str(lib), str(FIXTURE_C)])
        if r.returncode == 0:
            return lib
    pytest.skip("no working C compiler")


def _write_forcings(path):
    netCDF4 = pytest.importorskip("netCDF4")
    nc = netCDF4.Dataset(path, "w")
    try:
        nc.createDimension("catchment-id", len(LOCATIONS))
        nc.createDimension("time", N_FORCING_STEPS)
        ids = nc.createVariable("ids", str, ("catchment-id",))
        for i, loc in enumerate(LOCATIONS):
            ids[i] = loc
        t = nc.createVariable("Time", "i8", ("catchment-id", "time"))
        t[:] = np.tile(START_EPOCH + np.arange(N_FORCING_STEPS) * 3600, (len(LOCATIONS), 1))

        for name, units, fn in [
            ("RAINRATE", "mm h-1", precip_mm_per_hour),
            ("T2D", "K", temperature_kelvin),
        ]:
            var = nc.createVariable(name, "f4", ("catchment-id", "time"))
            var.units = units
            var[:] = np.array(
                [[fn(i, s) for s in range(N_FORCING_STEPS)] for i in range(len(LOCATIONS))],
                dtype="f4",
            )
    finally:
        nc.close()


REALIZATION = {
    "global": {
        "formulations": [
            {
                "name": "bmi_multi",
                "params": {
                    "main_output_variable": "Q_OUT",
                    "output_variables": ["Q_OUT", "STORAGE", "STEP_COUNT"],
                    "modules": [
                        {
                            "name": "bmi_c",
                            "params": {
                                "model_type_name": "Bucket",
                                "library_file": "libs/libbucketbmi.so",
                                "init_config": "./config/cat_config/Bucket/{{id}}.ini",
                                "registration_function": "register_bmi_bucket",
                                "main_output_variable": "Q_OUT",
                                "variables_names_map": {
                                    "precip_rate": "RAINRATE",
                                    "temperature": "T2D",
                                },
                            },
                        }
                    ],
                },
            }
        ],
        "forcing": {"path": "./forcings/forcings.nc"},
    },
    "time": {
        "start_time": "2010-01-01 00:00:00",
        "end_time": "2010-01-02 00:00:00",
        "output_interval": 3600,
    },
}


@pytest.fixture(scope="session")
def data_dir(tmp_path_factory):
    root = tmp_path_factory.mktemp("bmi_data")
    (root / "config/cat_config/Bucket").mkdir(parents=True)
    (root / "forcings").mkdir()
    (root / "libs").mkdir()

    _build_library(root / "libs")
    _write_forcings(root / "forcings/forcings.nc")

    for loc in LOCATIONS:
        (root / f"config/cat_config/Bucket/{loc}.ini").write_text(
            f"drain_fraction={DRAIN_FRACTION}\ninitial_storage={INITIAL_STORAGE}\n"
        )
    (root / "config/realization.json").write_text(json.dumps(REALIZATION, indent=2))

    con = sqlite3.connect(root / "config/hydrofabric.gpkg")
    con.execute("CREATE TABLE divides (divide_id TEXT, areasqkm REAL)")
    con.executemany("INSERT INTO divides VALUES (?, ?)", [(l, 1.0) for l in LOCATIONS])
    con.commit()
    con.close()
    return root


@pytest.fixture
def model_library(data_dir):
    return str(data_dir / "libs/libbucketbmi.so")


@pytest.fixture
def model_config(data_dir):
    return str(data_dir / "config/cat_config/Bucket/cat-1.ini")
