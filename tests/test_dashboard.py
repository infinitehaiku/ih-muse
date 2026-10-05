"""Dashboard definitions through the binding: validation and delivery."""

import copy
import json
from pathlib import Path
from typing import Any

import pytest

import ih_muse
from ih_muse.exceptions import DashboardDefinitionError, MuseError

EXAMPLES = Path(__file__).resolve().parent.parent / "examples" / "dashboards"


def example(name: str) -> dict[str, Any]:
    return json.loads((EXAMPLES / name).read_text())


def test_every_shipped_example_validates_as_dict_and_as_text() -> None:
    for path in sorted(EXAMPLES.glob("*.json")):
        definition = json.loads(path.read_text())
        (canonical,) = ih_muse.validate_dashboard_definitions(definition)
        # Canonical form fills defaults; it validates to itself.
        assert ih_muse.validate_dashboard_definitions(canonical) == [canonical], path
        assert ih_muse.validate_dashboard_definitions(path.read_text()) == [canonical], path
        assert (canonical["id"], len(canonical["panels"])) == (definition["id"], len(definition["panels"]))
    both = [example("macos-host.json"), example("k8s-cluster.json")]
    assert [d["id"] for d in ih_muse.validate_dashboard_definitions(both)] == ["macos.host", "k8s.cluster"]


@pytest.mark.parametrize(
    ("edit", "reason"),
    [
        (lambda d: d.update(columns=9), "columns must be 2..=4"),
        (lambda d: d.update(colour="red"), "colour"),
        (lambda d: d.update(id="macos.cluster"), "id must be"),
        (lambda d: d["blocks"][0]["panels"].append("gpu"), "unknown panel id gpu"),
        (lambda d: d["panels"].clear(), "at least one panel"),
        (lambda d: d.update(revision="one"), "expected u32"),
    ],
)
def test_invalid_definitions_raise_with_the_reason(edit: Any, reason: str) -> None:
    definition = example("k8s-cluster.json")
    edit(definition)
    with pytest.raises(DashboardDefinitionError, match=reason) as raised:
        ih_muse.validate_dashboard_definitions(definition)
    assert isinstance(raised.value, MuseError)


def test_duplicates_and_non_json_values_are_rejected() -> None:
    definition = example("k8s-cluster.json")
    with pytest.raises(DashboardDefinitionError, match="duplicate dashboard id k8s.cluster"):
        ih_muse.validate_dashboard_definitions([definition, definition])
    with pytest.raises(DashboardDefinitionError, match="not JSON-serializable"):
        ih_muse.validate_dashboard_definitions({"id": object()})
    with pytest.raises(DashboardDefinitionError, match="not valid JSON"):
        ih_muse.validate_dashboard_definitions("{not json")


def batch() -> dict[str, Any]:
    return {"schema_version": 1, "entities": [], "observations": []}


def test_definitions_ride_batches_until_a_poet_acknowledges_one_that_carried_them() -> None:
    definition = example("k8s-cluster.json")
    delivery = ih_muse.DashboardDelivery(definition)
    definition = ih_muse.validate_dashboard_definitions(definition)[0]
    assert delivery.definitions == [definition]
    assert not delivery.is_delivered

    first, second = batch(), batch()
    delivery.attach(first)
    delivery.attach(second)
    assert first["dashboards"] == [definition]
    assert second["dashboards"] == [definition]

    # A batch without them, or with other definitions, proves nothing.
    delivery.acknowledge(batch())
    other = copy.deepcopy(second)
    other["dashboards"][0]["revision"] = 2
    delivery.acknowledge(other)
    assert not delivery.is_delivered

    delivery.acknowledge(second)
    assert delivery.is_delivered
    later = batch()
    delivery.attach(later)
    assert "dashboards" not in later

    delivery.resend()
    again = batch()
    delivery.attach(again)
    assert again["dashboards"] == [definition]


def test_an_invalid_delivery_is_refused() -> None:
    definition = example("k8s-cluster.json")
    definition["columns"] = 1
    with pytest.raises(DashboardDefinitionError, match="columns"):
        ih_muse.DashboardDelivery(definition)


def test_more_than_32_definitions_go_out_over_several_batches() -> None:
    base = example("k8s-cluster.json")
    definitions = []
    for index in range(40):
        definition = copy.deepcopy(base)
        definition["id"] = f"k8s.cluster{index}"
        definitions.append(definition)
    delivery = ih_muse.DashboardDelivery(definitions)
    assert delivery.pending_chunks == 2

    first = batch()
    delivery.attach(first)
    assert [d["id"] for d in first["dashboards"]] == [f"k8s.cluster{i}" for i in range(32)]
    delivery.acknowledge(first)
    second = batch()
    delivery.attach(second)
    assert [d["id"] for d in second["dashboards"]] == [f"k8s.cluster{i}" for i in range(32, 40)]
    assert not delivery.is_delivered
    delivery.acknowledge(second)
    assert delivery.is_delivered
