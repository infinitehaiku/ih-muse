"""Public package for ih_muse."""

from ih_muse._utils.muse_version import get_muse_version as _get_muse_version
from ih_muse.config import ClientType, Config
from ih_muse.dashboard import DashboardDelivery, validate_dashboard_definitions
from ih_muse.muse import Muse
from ih_muse.proto import (
    ElementKindRegistration,
    MetricDefinition,
    MetricPayload,
    MetricQuery,
    TimestampResolution,
)

__version__: str = _get_muse_version()
del _get_muse_version

__all__ = [
    "ClientType",
    "Config",
    "DashboardDelivery",
    "ElementKindRegistration",
    "MetricDefinition",
    "MetricPayload",
    "MetricQuery",
    "Muse",
    "TimestampResolution",
    "validate_dashboard_definitions",
]
