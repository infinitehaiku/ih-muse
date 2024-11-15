# py-ih-muse/ih_muse/config/config.py

from ih_muse.ih_muse import ClientType
from ih_muse.ih_muse import TimestampResolution
from ih_muse.ih_muse import PyConfig
from ih_muse.proto import ElementKindRegistration, MetricDefinition


class Config:

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
