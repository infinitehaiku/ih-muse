# tests/test_init.py

import pytest
from common import MuseTestContext


@pytest.mark.asyncio
async def test_muse_initialization_with_poet() -> None:
    ctx = await MuseTestContext.create()
    assert ctx.muse.is_initialized(), "Muse failed to initialize with Poet client"

    local_elem_id = await ctx.register_test_element()
    await ctx.muse.send_metric(local_elem_id, "cpu_usage", 50.0)
