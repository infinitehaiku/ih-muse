# py-ih-muse/ih_muse/muse/muse.py

from ih_muse.ih_muse import PyMetricDefinition


class MetricDefinition:
    _metric_def: PyMetricDefinition

    def __init__(self, code: str, name: str, description: str) -> None:
        self._metric_def = PyMetricDefinition(code, name, description)
