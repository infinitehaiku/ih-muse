# py-ih-muse/ih_muse/muse/muse.py

from __future__ import annotations

from typing import TYPE_CHECKING

from ih_muse.ih_muse import PyMuse

if TYPE_CHECKING:
    from ih_muse.config import Config


class Muse:
    """The main class for interacting with the Muse system in Python.

    :param Config config:
        The configuration object for initializing Muse.

    ```python
    # Example usage:
    from ih_muse import Muse, Config

    config = Config(...)
    muse = Muse(config)
    await muse.initialize(timeout=5.0)
    ```
    """

    _muse: PyMuse

    def __init__(self, config: Config) -> None:
        self._muse = PyMuse(config._config)

    async def initialize(self, timeout: float | None = None) -> None:
        """Initialize the Muse client and starts background tasks.

        :param Optional[float] timeout:
            Optional timeout in seconds for the initialization process.

        :raises MuseInitializationTimeoutError:
            If initialization times out.
        """
        await self._muse.initialize(timeout)

    @classmethod
    async def create(cls, config: Config, timeout: float | None = None) -> Muse:
        """Create and initialize a Muse instance.

        :param Config config:
            The configuration object.
        :param Optional[float] timeout:
            Optional timeout in seconds for initialization.

        :return:
            An initialized Muse instance.
        """
        instance = cls(config)
        await instance.initialize(timeout)
        return instance

    def is_initialized(self) -> bool:
        """Check whether the Muse client is initialized.

        :return:
            `True` if initialized, `False` otherwise.
        """
        return self._muse.is_initialized

    async def register_element(
        self,
        kind_code: str,
        name: str,
        metadata: dict[str, str],
        parent_id: int | None = None,
    ) -> int:
        """Register a new element with the Muse system.

        :param str kind_code:
            The kind code of the element.
        :param str name:
            The name of the element.
        :param dict[str, str] metadata:
            Metadata associated with the element.
        :param Optional[int] parent_id:
            The parent element ID, if any.

        :return:
            The local element ID assigned to the registered element.

        :raises MuseError:
            If registration fails.
        """
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
        """Send a metric value associated with an element.

        :param int local_elem_id:
            The local ID of the element.
        :param str metric_code:
            The code identifying the metric.
        :param float value:
            The value of the metric to send.

        :raises MuseError:
            If sending the metric fails.
        """
        await self._muse.send_metric(
            local_elem_id,
            metric_code,
            value,
        )

    async def replay(self, replay_path: str) -> None:
        """Replays events from a recording file.

        :param str replay_path:
            The file path to the recording.

        :raises MuseError:
            If replaying the events fails.
        """
        await self._muse.replay(replay_path)