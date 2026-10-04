"""Exception definitions for ih_muse."""

try:
    from ih_muse.ih_muse import (
        ClientError,
        ConfigurationError,
        DashboardDefinitionError,
        DurationConversionError,
        InvalidElementKindCodeError,
        InvalidFileExtensionError,
        InvalidMetricCodeError,
        MuseError,
        MuseInitializationTimeoutError,
        NotAvailableRemoteElementIdError,
        RecordingError,
        ReplayingError,
    )
except ImportError:
    # redefined for documentation purposes when there is no binary

    class MuseError(Exception):  # type: ignore[no-redef]
        """Base class for all IH-Muse errors."""

    class MuseInitializationTimeoutError(MuseError):  # type: ignore[no-redef, misc]
        """Exception raised when ...

        Examples
        --------
        >>> some code...
        ih_muse.exceptions.MuseInitializationTimeoutError: ...

        """

    class ConfigurationError(MuseError):  # type: ignore[no-redef, misc]
        """TODO DOCS."""

    class ClientError(MuseError):  # type: ignore[no-redef, misc]
        """TODO DOCS."""

    class RecordingError(MuseError):  # type: ignore[no-redef, misc]
        """TODO DOCS."""

    class ReplayingError(MuseError):  # type: ignore[no-redef, misc]
        """TODO DOCS."""

    class InvalidFileExtensionError(MuseError):  # type: ignore[no-redef, misc]
        """TODO DOCS."""

    class InvalidElementKindCodeError(MuseError):  # type: ignore[no-redef, misc]
        """TODO DOCS."""

    class NotAvailableRemoteElementIdError(MuseError):  # type: ignore[no-redef, misc]
        """TODO DOCS."""

    class InvalidMetricCodeError(MuseError):  # type: ignore[no-redef, misc]
        """TODO DOCS."""

    class DurationConversionError(MuseError):  # type: ignore[no-redef, misc]
        """TODO DOCS."""

    class DashboardDefinitionError(MuseError):  # type: ignore[no-redef, misc]
        """A dashboard definition does not parse or breaks a rule."""

    class PanicException(MuseError):  # type: ignore[no-redef, misc]
        """Exception raised on panics in the underlying Rust library."""


__all__ = [
    "ClientError",
    "ConfigurationError",
    "DashboardDefinitionError",
    "InvalidElementKindCodeError",
    "InvalidFileExtensionError",
    "InvalidMetricCodeError",
    "MuseError",
    "MuseInitializationTimeoutError",
    "NotAvailableRemoteElementIdError",
    "PanicException",
    "RecordingError",
    "ReplayingError",
]
