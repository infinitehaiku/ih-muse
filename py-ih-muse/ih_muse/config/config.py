# py-ih-muse/ih_muse/config/config.py

from __future__ import annotations

from typing import TYPE_CHECKING

from ih_muse.ih_muse import ClientType, PyConfig, TimestampResolution

if TYPE_CHECKING:
    from ih_muse.proto import ElementKindRegistration, MetricDefinition


class Config:
    """Configuration class for the Muse client in Python.

    :param list[str] endpoints:
        List of endpoint URLs for the Muse client.
    :param ClientType client_type:
        The type of client to use (`ClientType.Poet` or `ClientType.Mock`).
    :param TimestampResolution default_resolution:
        Default timestamp resolution for metrics.
    :param list[ElementKindRegistration] element_kinds:
        List of element kinds to register.
    :param list[MetricDefinition] metric_definitions:
        List of metric definitions available for reporting.
    :param int max_reg_elem_retries:
        Maximum number of retries for element registration.
    :param bool recording_enabled:
        Enables event recording if set to `True`.
    :param Optional[str] recording_path:
        File path for recording events (required if `recording_enabled` is `True`).

    ```python
    # Example usage:
    from ih_muse import Config, ClientType, TimestampResolution
    from ih_muse.proto import ElementKindRegistration, MetricDefinition

    config = Config(
        endpoints=["http://localhost:8080"],
        client_type=ClientType.Poet,
        default_resolution=TimestampResolution.Milliseconds,
        element_kinds=[ElementKindRegistration("kind_code", "description")],
        metric_definitions=[MetricDefinition("metric_code", "description")],
        max_reg_elem_retries=3,
        recording_enabled=False,
    )
    ```
    """

    _config: PyConfig

    def __init__(
        self,
        endpoints: list[str],
        client_type: ClientType,
        default_resolution: TimestampResolution,
        element_kinds: list[ElementKindRegistration],
        metric_definitions: list[MetricDefinition],
        max_reg_elem_retries: int,
        recording_enabled: bool,
        recording_path: str | None = None,
    ) -> None:
        py_element_kinds = [ekr._elem_kind_reg for ekr in element_kinds]
        py_metric_definitions = [md._metric_def for md in metric_definitions]

        self._config = PyConfig(
            endpoints,
            client_type,
            default_resolution,
            py_element_kinds,
            py_metric_definitions,
            max_reg_elem_retries,
            recording_enabled,
            recording_path,
        )