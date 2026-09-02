"""The unified Model interface: one API regardless of the model's language or variable types."""

import numpy as np
import pytest

from bmi_driver import Model, convert

from conftest import expected_q_out


def open_bucket(library, config, **kwargs):
    m = Model.c(library, register="register_bmi_bucket", name="Bucket", **kwargs)
    m.initialize(config)
    return m


def test_metadata(model_library, model_config):
    with open_bucket(model_library, model_config) as m:
        assert m.get_component_name() == "Bucket"
        assert m.get_input_var_names() == ["precip_rate", "temperature"]
        assert m.get_output_var_names() == ["Q_OUT", "STORAGE", "STEP_COUNT"]
        assert m.get_var_units("Q_OUT") == "m h-1"
        assert m.get_time_units() == "h"
        assert m.get_time_step() == 1.0
        assert m.time_step_seconds() == 3600.0
        assert m.name == "Bucket"
        assert m.initialized


def test_one_get_value_covers_double_float_and_int(model_library, model_config):
    # The model stores Q_OUT as double, temperature as float and STEP_COUNT as int. A caller
    # should not have to know that, nor pick between three typed BMI calls.
    with open_bucket(model_library, model_config) as m:
        assert m.dtype_of("Q_OUT") == "float64"
        assert m.dtype_of("temperature") == "float32"
        assert m.dtype_of("STEP_COUNT") == "int32"

        m.set_value("precip_rate", 0.002)
        m.set_value("temperature", 4.5)
        m.update()

        assert m.get_value("Q_OUT").dtype == np.float64
        assert m.get_value("temperature").dtype == np.float32
        assert m.get_value("STEP_COUNT").dtype == np.int32
        assert m.get_value("STEP_COUNT")[0] == 1
        assert m.get_value("temperature")[0] == pytest.approx(4.5)


def test_dtype_can_be_pinned(model_library, model_config):
    with open_bucket(model_library, model_config, dtype="float64") as m:
        # Every variable comes back as float64, including the int and float ones.
        m.update()
        assert m.get_value("STEP_COUNT").dtype == np.float64
        assert m.get_value("temperature").dtype == np.float64
        assert m.dtype_of("STEP_COUNT") == "float64"


def test_set_value_accepts_scalars_lists_and_arrays(model_library, model_config):
    with open_bucket(model_library, model_config) as m:
        for value in (0.003, [0.003], (0.003,), np.array([0.003]), np.float32(0.003)):
            m.set_value("precip_rate", value)
            assert m.get_value("precip_rate")[0] == pytest.approx(0.003, rel=1e-6)


def test_set_value_rejects_nonsense(model_library, model_config):
    with open_bucket(model_library, model_config) as m:
        with pytest.raises(ValueError):
            m.set_value("precip_rate", "not a number")


def test_unknown_variable_raises(model_library, model_config):
    with open_bucket(model_library, model_config) as m:
        with pytest.raises(RuntimeError):
            m.get_value("NO_SUCH_VARIABLE")


def test_bad_library_raises():
    with pytest.raises(RuntimeError):
        Model.c("/nonexistent/lib.so", register="register_bmi_bucket")


def test_bad_dtype_raises(model_library):
    with pytest.raises(ValueError):
        Model.c(model_library, register="register_bmi_bucket", dtype="complex128")


def test_context_manager_finalizes(model_library, model_config):
    m = open_bucket(model_library, model_config)
    with m:
        assert m.initialized
    assert not m.initialized


def test_update_until(model_library, model_config):
    with open_bucket(model_library, model_config) as m:
        m.set_value("precip_rate", 0.001)
        m.update_until(5.0)  # model time is in hours
        assert m.get_scalar("STEP_COUNT") == 5
        assert m.get_current_time() == 5.0


def test_hand_written_loop_matches_the_driver(model_library, model_config):
    # The point of the bindings: someone writing the engine in Python gets the same answer.
    with open_bucket(model_library, model_config) as m:
        got = []
        for step in range(25):
            precip_mm = 0.5 + (step % 7) * 0.25
            m.set_value("precip_rate", precip_mm * convert(1.0, "mm h-1", "m h-1"))
            m.update()
            got.append(m.get_scalar("Q_OUT"))
    assert np.allclose(got, expected_q_out(0, 25))


def test_sloth_provides_constants():
    m = Model.sloth({"land_surface_water__runoff_depth": "0.25"})
    m.initialize("/dev/null")
    assert m.get_scalar("land_surface_water__runoff_depth") == pytest.approx(0.25)
    m.finalize()


def test_convert():
    assert convert(1.0, "mm h-1", "m h-1") == pytest.approx(0.001)
    assert convert(273.15, "K", "degC") == pytest.approx(0.0)
    with pytest.raises(ValueError):
        convert(1.0, "m", "kg")
