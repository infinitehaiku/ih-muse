# tests/test_init_timeout.py

import pytest
from ih_muse import (
    ClientType,
    Config,
    ElementKindRegistration,
    MetricDefinition,
    Muse,
    TimestampResolution,
)
from ih_muse.exceptions import MuseInitializationTimeoutError


@pytest.mark.asyncio
async def test_muse_initialization_timeout() -> None:
    element_kind = ElementKindRegistration(
        "server",
        "Server",
        "A server element kind",
    )
    metric_definition = MetricDefinition(
        "cpu_usage",
        "CPU Usage",
        "The CPU usage of a server",
    )

    # Create config with unreachable endpoint to test timeout
    config = Config(
        endpoints=["http://unreachable:9999"],
        client_type=ClientType.Poet,
        recording_enabled=False,
        recording_path=None,
        default_resolution=TimestampResolution.Seconds,
        element_kinds=[element_kind],
        metric_definitions=[metric_definition],
        max_reg_elem_retries=1,  # Set low retry count for faster test
    )

    muse = Muse(config)

    with pytest.raises(MuseInitializationTimeoutError):
        await muse.initialize(timeout=2.0)

    assert (
        not muse.is_initialized()
    ), "Muse should not initialize with unreachable endpoint"
