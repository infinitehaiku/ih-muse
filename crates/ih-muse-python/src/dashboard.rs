// crates/ih-muse-python/src/dashboard.rs

//! Dashboard definitions for Python Muses: the author passes plain dicts (or
//! JSON text) of the shape in `schemas/dashboard-definition.schema.json`; the
//! Rust types parse and validate them, and [`PyDashboardDelivery`] applies the
//! same "attach until a Poet acknowledges a batch that carried them" rule as
//! Rust Muses.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString};

use ih_muse::dashboards::{parse_dashboard_definitions, DashboardDelivery, DashboardSetError};
use ih_muse_proto::DashboardDefinition;

use crate::exceptions::DashboardDefinitionError;

fn to_py_err(err: DashboardSetError) -> PyErr {
    DashboardDefinitionError::new_err(err.to_string())
}

/// JSON text of `obj`: a `str` is taken as JSON already, anything else goes
/// through Python's `json.dumps`.
fn json_text(obj: &Bound<'_, PyAny>) -> PyResult<String> {
    if let Ok(text) = obj.downcast::<PyString>() {
        return Ok(text.to_str()?.to_owned());
    }
    let json = obj.py().import_bound("json")?;
    json.call_method1("dumps", (obj,))
        .map_err(|e| DashboardDefinitionError::new_err(format!("not JSON-serializable: {e}")))?
        .extract()
}

fn definitions_from(obj: &Bound<'_, PyAny>) -> PyResult<Vec<DashboardDefinition>> {
    parse_dashboard_definitions(&json_text(obj)?).map_err(to_py_err)
}

fn to_python(py: Python<'_>, definitions: &[DashboardDefinition]) -> PyResult<PyObject> {
    let text = serde_json::to_string(definitions)
        .map_err(|e| DashboardDefinitionError::new_err(e.to_string()))?;
    Ok(py
        .import_bound("json")?
        .call_method1("loads", (text,))?
        .unbind())
}

/// Parses and validates one definition (dict) or a list of them, or the same
/// as JSON text. Returns the definitions as a list of dicts in canonical form
/// (defaults filled, empty optional fields left out). Raises
/// `DashboardDefinitionError` with the reason on any problem.
#[pyfunction]
pub fn validate_dashboard_definitions(
    py: Python<'_>,
    definitions: &Bound<'_, PyAny>,
) -> PyResult<PyObject> {
    to_python(py, &definitions_from(definitions)?)
}

/// A Python Muse's validated dashboards and whether a Poet acknowledged a
/// graph batch that carried them.
#[pyclass(name = "DashboardDelivery")]
pub struct PyDashboardDelivery {
    inner: DashboardDelivery,
}

#[pymethods]
impl PyDashboardDelivery {
    /// Validates the definitions (dict, list of dicts or JSON text).
    #[new]
    pub fn new(definitions: &Bound<'_, PyAny>) -> PyResult<Self> {
        let inner = DashboardDelivery::new(definitions_from(definitions)?).map_err(to_py_err)?;
        Ok(Self { inner })
    }

    /// The definitions in canonical form, as a list of dicts.
    #[getter]
    pub fn definitions(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, self.inner.definitions())
    }

    #[getter]
    pub fn is_delivered(&self) -> bool {
        self.inner.is_delivered()
    }

    /// Sets `batch["dashboards"]` (a graph batch as a dict) unless a Poet
    /// already acknowledged a batch that carried them.
    pub fn attach(&self, py: Python<'_>, batch: &Bound<'_, PyDict>) -> PyResult<()> {
        if !self.inner.is_delivered() {
            batch.set_item("dashboards", to_python(py, self.inner.definitions())?)?;
        }
        Ok(())
    }

    /// Call after a Poet acknowledged `batch`; marks the definitions
    /// delivered only if that batch carried them.
    pub fn acknowledge(&mut self, batch: &Bound<'_, PyDict>) -> PyResult<()> {
        let Some(carried) = batch.get_item("dashboards")? else {
            return Ok(());
        };
        // A batch whose field does not parse cannot have carried ours.
        if let Ok(carried) = parse_dashboard_definitions(&json_text(&carried)?) {
            self.inner.acknowledge_carried(&carried);
        }
        Ok(())
    }

    /// Sends the definitions again from the next batch.
    pub fn resend(&mut self) {
        self.inner.resend();
    }
}
