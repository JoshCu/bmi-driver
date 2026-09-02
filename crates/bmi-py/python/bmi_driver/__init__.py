"""One Python interface to C, Fortran and Python BMI models.

The compiled extension exposes a single ``Model`` class. Which adapter it uses is decided once,
by the classmethod you construct it with; after that every model looks the same::

    from bmi_driver import Model

    with Model.c("libcfebmi.so", register="register_bmi_cfe") as cfe:
        cfe.initialize("cat-123.ini")
        cfe.set_value("atmosphere_water__liquid_equivalent_precipitation_rate", 0.001)
        cfe.update()
        q = cfe.get_value("Q_OUT")

``get_value`` and ``set_value`` pick the right typed BMI call for you, so there is no
``set_value_double`` / ``set_value_float`` / ``set_value_int`` to choose between. Pass
``dtype="float64"`` at construction if you would rather pin one type for every variable than let
each variable use whatever the model reports.
"""

from ._bmi_driver import Model, convert, preload_dependencies

__all__ = ["Model", "convert", "preload_dependencies"]

try:  # optional: only built when the `driver` feature is enabled
    from ._bmi_driver import Forcings, PySimulation as Simulation

    __all__ += ["Forcings", "Simulation"]
except ImportError:  # pragma: no cover
    pass
