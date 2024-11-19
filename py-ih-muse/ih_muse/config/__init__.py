"""Public package for ih_muse configuration."""

from ih_muse.ih_muse import ClientType

from .config import Config

__all__ = [
    "ClientType",
    "Config",
]
