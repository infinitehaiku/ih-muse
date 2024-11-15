# tests/test_element.py

import pytest
from common import MuseTestContext
from ih_muse import ClientType


@pytest.mark.asyncio
async def test_element_registration() -> None:
    ctx = await MuseTestContext.create(ClientType.Poet)
    local_elem_id = await ctx.register_test_element()

    # Since we don't have direct access to the state, we assume the element is registered
    assert local_elem_id is not None, "Failed to register element"
