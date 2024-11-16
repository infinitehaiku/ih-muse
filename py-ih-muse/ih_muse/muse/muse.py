# py-ih-muse/ih_muse/muse/muse.py
from __future__ import annotations

from typing import TYPE_CHECKING

from ih_muse.ih_muse import PyMuse

if TYPE_CHECKING:
    from ih_muse.config import Config


class Muse:
    _muse: PyMuse

    def __init__(self, config: Config) -> None:
        self._muse = PyMuse(config._config)

    async def initialize(self, timeout: float | None = None) -> None:
        await self._muse.initialize(timeout)

    @classmethod
    async def create(cls, config: Config, timeout: float | None = None) -> Muse:
        """Factory method to create and initialize a Muse instance."""
        instance = cls(config)
        await instance.initialize(timeout)
        return instance

    def is_initialized(self) -> bool:
        return self._muse.is_initialized

    async def register_element(
        self,
        kind_code: str,
        name: str,
        metadata: dict[str, str],
        parent_id: int | None = None,
    ) -> int:
        local_elem_id = await self._muse.register_element(
            kind_code,
            name,
            metadata,
            parent_id,
        )
        return local_elem_id

    async def send_metric(
        self,
        local_elem_id: int,
        metric_code: str,
        value: float,
    ) -> None:
        await self._muse.send_metric(
            local_elem_id,
            metric_code,
            value,
        )

    async def replay(self, replay_path: str) -> None:
        await self._muse.replay(replay_path)
