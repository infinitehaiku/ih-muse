"""Tests for the TimestampResolution to_timedelta method and related functionalities."""

from datetime import timedelta

import ih_muse


def test_timestamp_resolution_to_timedelta() -> None:
    """Test TimestampResolution to_timedelta method."""
    test_cases = [
        (ih_muse.TimestampResolution.Weeks, timedelta(weeks=1)),
        (ih_muse.TimestampResolution.Days, timedelta(days=1)),
        (ih_muse.TimestampResolution.Hours, timedelta(hours=1)),
        (ih_muse.TimestampResolution.Minutes, timedelta(minutes=1)),
        (ih_muse.TimestampResolution.Seconds, timedelta(seconds=1)),
        (ih_muse.TimestampResolution.Microseconds, timedelta(microseconds=1)),
        (ih_muse.TimestampResolution.Milliseconds, timedelta(milliseconds=1)),
    ]
    for resolution, expected_delta in test_cases:
        result = resolution.to_timedelta()
        assert isinstance(result, timedelta), f"Expected timedelta for {resolution}"
        assert result == expected_delta, f"Mismatch for {resolution}"

    # Years and Months tests
    years_delta = ih_muse.TimestampResolution.Years.to_timedelta()
    assert isinstance(years_delta, timedelta)
    assert timedelta(days=365) <= years_delta <= timedelta(days=366)

    months_delta = ih_muse.TimestampResolution.Months.to_timedelta()
    assert isinstance(months_delta, timedelta)
    assert timedelta(days=27) <= months_delta <= timedelta(days=31)
