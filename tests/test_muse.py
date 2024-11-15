import ih_muse
import pytest


@pytest.mark.asyncio
async def test_muse():
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

    config = ih_muse.Config(
        endpoints=["http://localhost:8000"],
        client_type=ih_muse.ClientType.Poet,
        recording_enabled=False,
        recording_path=None,
        default_resolution=ih_muse.TimestampResolution.Seconds,
        element_kinds=[element_kind],
        metric_definitions=[metric_definition],
        max_reg_elem_retries=5,
    )

    muse = ih_muse.Muse(config)
    await muse.initialize(10)
    # print("Muse is initialized:", muse.is_initialized())
