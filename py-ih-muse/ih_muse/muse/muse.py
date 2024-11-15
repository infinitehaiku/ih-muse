# py-ih-muse/ih_muse/muse/muse.py

from ih_muse.ih_muse import PyMuse
from ih_muse.config import Config


class Muse:

    _muse: PyMuse

    def __init__(self, config: Config) -> None:
        self._muse = PyMuse(config._config)

    async def initialize(self, timeout: float | None = None) -> None:
        await self._muse.initialize(timeout)

    @classmethod
    async def create(cls, config: Config, timeout: float | None = None) -> "Muse":
        """Factory method to create and initialize a Muse instance"""
        instance = cls(config)
        await instance.initialize(timeout)
        return instance
