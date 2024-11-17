from ih_muse._utils.muse_version import get_muse_version as _get_muse_version
from ih_muse.config import ClientType, Config
from ih_muse.muse import Muse
from ih_muse.proto import ElementKindRegistration, MetricDefinition, TimestampResolution

# from ih_muse.foo import foo

__version__: str = _get_muse_version()
del _get_muse_version

__all__ = [
    "ClientType",
    "Config",
    "ElementKindRegistration",
    "MetricDefinition",
    "Muse",
    "TimestampResolution",
    "foo",
]
