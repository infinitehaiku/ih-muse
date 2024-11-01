from ih_muse.foo import foo


def test_foo() -> None:
    assert foo("foo") == "foo"
