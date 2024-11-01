# tests/test_rust.py

from ih_muse import double


def test_double() -> None:
    assert double(2) == 4
