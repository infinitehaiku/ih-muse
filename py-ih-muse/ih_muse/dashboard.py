"""Default dashboards a Python Muse defines.

Pass definitions as dicts (or JSON text) of the shape in
``schemas/dashboard-definition.schema.json``. ``validate_dashboard_definitions``
checks them with the Rust types and raises
``ih_muse.exceptions.DashboardDefinitionError`` with the reason.
``DashboardDelivery`` attaches them to graph batches (dicts) until a Poet
acknowledges a batch that carried them, the same rule Rust Muses follow.
Pass the Poet answer's ``definitions_epoch`` to ``acknowledge`` so the
definitions go out again, once, when that Poet lost them (wiped store).
"""

from ih_muse.ih_muse import DashboardDelivery, validate_dashboard_definitions

__all__ = ["DashboardDelivery", "validate_dashboard_definitions"]
