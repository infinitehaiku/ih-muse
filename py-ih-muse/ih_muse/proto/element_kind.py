# py-ih-muse/ih_muse/muse/muse.py
from __future__ import annotations

from typing import Optional

from ih_muse.ih_muse import PyElementKindRegistration


class ElementKindRegistration:
    _elem_kind_reg: PyElementKindRegistration

    def __init__(
        self, code: str, name: str, description: str, parent_code: Optional[str] = None
    ) -> None:
        self._elem_kind_reg = PyElementKindRegistration(
            code, name, description, parent_code
        )
