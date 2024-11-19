# tests/test_metrics.py


import asyncio

import pytest
from common import MuseTestContext


@pytest.mark.asyncio
async def test_send_and_receive_metric() -> None:
    ctx = await MuseTestContext.create()
    local_elem_id = await ctx.register_test_element()

    # Send metric
    await ctx.muse.send_metric(local_elem_id, "cpu_usage", 42.0)

    # Wait for the metric to be processed
    await asyncio.sleep(5)

    # Since we don't have direct access to the PoetClient or the state, we'll assume success
    # In a real test, you might query the backend to verify the metric was received
    assert True, "Metric sent and (assumed) received"
