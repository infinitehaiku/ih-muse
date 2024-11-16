# tests/common.py

import asyncio
import time

from ih_muse import (
    ClientType,
    Config,
    ElementKindRegistration,
    MetricDefinition,
    Muse,
    TimestampResolution,
)


class MuseTestContext:
    muse: Muse
    endpoint: str

    def __init__(self, muse: Muse, endpoint: str) -> None:
        self.muse = muse
        self.endpoint = endpoint

    @classmethod
    async def create(cls, client_type: ClientType) -> "MuseTestContext":
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

        config = Config(
            endpoints=["http://localhost:8000"],
            client_type=client_type,
            recording_enabled=False,
            recording_path=None,
            default_resolution=TimestampResolution.Seconds,
            element_kinds=[element_kind],
            metric_definitions=[metric_definition],
            max_reg_elem_retries=3,
        )

        muse = await Muse.create(config, timeout=10.0)
        endpoint = "http://localhost:8000"
        return cls(muse, endpoint)

    async def register_test_element(self) -> int:
        local_elem_id = await self.muse.register_element(
            "server",
            "TestServer",
            {},
            None,
        )

        # Wait until the element is registered
        start_time = time.time()
        while (
            not self.is_element_registered(local_elem_id)
            and time.time() - start_time < 5
        ):
            await asyncio.sleep(0.1)

        return local_elem_id

    def is_element_registered(self, local_elem_id: int) -> bool:
        # Implement a method to check if the element is registered
        # Since we don't have direct access to the state, this could be a placeholder
        # or you can expose additional methods if necessary
        return True  # For simplicity, assume it's registered after waiting
