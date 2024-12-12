"""General tests for Muse."""

import ih_muse
import pytest

from common import get_client_type_from_env


@pytest.mark.asyncio
@pytest.mark.unit
@pytest.mark.integration
async def test_muse() -> None:
    """Test basic Muse functionality."""
    element_kind = ih_muse.ElementKindRegistration(
        "EK1",
        "ElementKind1",
        "Test Element Kind",
    )
    metric_definition = ih_muse.MetricDefinition(
        "M1",
        "Metric1",
        "Test Metric",
    )
    default_resolution = ih_muse.TimestampResolution.Seconds
    config = ih_muse.Config(
        endpoints=["http://localhost:8000"],
        client_type=get_client_type_from_env(),
        recording_enabled=False,
        recording_path=None,
        default_resolution=default_resolution,
        element_kinds=[element_kind],
        metric_definitions=[metric_definition],
        max_reg_elem_retries=5,
    )

    muse = ih_muse.Muse(config)

    assert muse.finest_resolution == default_resolution

    assert not muse.is_initialized
    await muse.initialize(10)
    assert muse.is_initialized
