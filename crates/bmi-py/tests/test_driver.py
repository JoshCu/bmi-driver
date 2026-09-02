"""The driver layer from Python: whole-config runs and forcing data."""

import numpy as np
import pytest

from bmi_driver import Forcings, Model, Simulation, convert

from conftest import LOCATIONS, N_FORCING_STEPS, START_EPOCH, expected_q_out


def test_simulation_runs_a_location(data_dir):
    sim = Simulation(str(data_dir))
    out = sim.run("cat-1")
    assert sorted(out) == ["Q_OUT", "STEP_COUNT", "STORAGE"]
    assert np.allclose(out["Q_OUT"], expected_q_out(0, len(out["Q_OUT"])))


def test_simulation_metadata_before_any_run(data_dir):
    sim = Simulation(str(data_dir))
    assert sim.total_steps == 24
    assert sim.start_epoch == START_EPOCH
    assert sim.output_interval == 3600
    assert sim.times()[0] == START_EPOCH
    assert len(sim.times()) == sim.total_steps + 1
    assert sim.locations() == LOCATIONS


def test_simulation_run_variable_and_units(data_dir):
    sim = Simulation(str(data_dir))
    q = sim.run_variable("cat-2", "Q_OUT")
    assert np.allclose(q, expected_q_out(1, len(q)))
    assert sim.var_units("cat-2", "Q_OUT") == "m h-1"


def test_simulation_rejects_unknown_location(data_dir):
    sim = Simulation(str(data_dir))
    with pytest.raises(RuntimeError):
        sim.run("cat-nope")


def test_forcings(data_dir):
    f = Forcings(str(data_dir / "forcings/forcings.nc"))
    assert sorted(f.variable_names()) == ["RAINRATE", "T2D"]
    assert f.location_ids() == LOCATIONS
    assert f.units("RAINRATE") == "mm h-1"
    assert f.time_step() == 3600.0
    assert f.timestep_count() == N_FORCING_STEPS

    rain = f.get("RAINRATE", "cat-1")
    assert rain.shape == (N_FORCING_STEPS,)
    assert f.get_at("RAINRATE", "cat-1", 3) == pytest.approx(rain[3])


def test_two_forcings_readers_coexist(data_dir):
    # One process-wide file handle behind the scenes; two independent readers on top.
    path = str(data_dir / "forcings/forcings.nc")
    a, b = Forcings(path), Forcings(path)
    assert not np.allclose(a.get("RAINRATE", "cat-1"), b.get("RAINRATE", "cat-3"))
    del a
    assert b.get("RAINRATE", "cat-3").shape == (N_FORCING_STEPS,)


def test_python_engine_reproduces_the_driver(data_dir):
    """Drive the model yourself with the real forcings and get the driver's answer."""
    sim = Simulation(str(data_dir))
    reference = sim.run("cat-1")["Q_OUT"]

    forcings = Forcings(str(data_dir / "forcings/forcings.nc"))
    rain = forcings.get("RAINRATE", "cat-1")
    to_model_units = convert(1.0, forcings.units("RAINRATE"), "m h-1")

    model = Model.c(str(data_dir / "libs/libbucketbmi.so"), register="register_bmi_bucket")
    model.initialize(str(data_dir / "config/cat_config/Bucket/cat-1.ini"))
    mine = []
    for step in range(sim.total_steps + 1):
        model.set_value("precip_rate", float(rain[step]) * to_model_units)
        model.update()
        mine.append(model.get_scalar("Q_OUT"))
    model.finalize()

    assert np.allclose(mine, reference)
