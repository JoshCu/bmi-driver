"""Driving a Python BMI model from inside the extension module.

Python calling Rust calling Python only works because the extension links pyo3 with
extension-module and without auto-initialize -- the interpreter is already running. These tests
are skipped when the module was built without the `python-models` feature.
"""

import numpy as np
import pytest

from bmi_driver import Model

from conftest import DRAIN_FRACTION, INITIAL_STORAGE, expected_q_out

pytestmark = pytest.mark.skipif(
    not hasattr(Model, "python"),
    reason="built without the python-models feature",
)


@pytest.fixture
def py_model(model_config):
    m = Model.python_file(str(__import__("pathlib").Path(__file__).parent / "bucket_py.py"),
                          "BucketPy", name="BucketPy")
    m.initialize(model_config)
    yield m


def test_python_model_through_the_same_interface(py_model):
    assert py_model.get_component_name() == "BucketPy"
    assert py_model.get_output_var_names() == ["Q_OUT", "STORAGE"]
    assert py_model.get_var_units("Q_OUT") == "m h-1"
    assert py_model.time_step_seconds() == 3600.0


def test_python_model_produces_the_same_answer_as_the_c_model(py_model, model_library, model_config):
    # Same maths, one in C and one in Python, reached through one interface.
    c_model = Model.c(model_library, register="register_bmi_bucket")
    c_model.initialize(model_config)

    for step in range(25):
        precip = (0.5 + (step % 7) * 0.25) * 0.001
        for m in (py_model, c_model):
            m.set_value("precip_rate", precip)
            m.update()
        assert py_model.get_scalar("Q_OUT") == pytest.approx(c_model.get_scalar("Q_OUT"))

    c_model.finalize()
    py_model.finalize()


def test_python_model_matches_the_reference_series(py_model):
    got = [
        (py_model.set_value("precip_rate", (0.5 + (s % 7) * 0.25) * 0.001),
         py_model.update(),
         py_model.get_scalar("Q_OUT"))[2]
        for s in range(25)
    ]
    assert np.allclose(got, expected_q_out(0, 25))
