// crates/ih-muse-python/src/proto/mod.rs

#[cfg(feature = "pymethods")]
mod element_kind;
#[cfg(feature = "pymethods")]
mod metric;
mod timestamp_resolution;

use pyo3::pyclass;

use ih_muse_proto::ElementKindRegistration as RustElementKindRegistration;
use ih_muse_proto::MetricDefinition as RustMetricDefinition;
use ih_muse_proto::MetricPayload as RustMetricPayload;
use ih_muse_proto::MetricQuery as RustMetricQuery;
pub use timestamp_resolution::PyTimestampResolution;

#[pyclass]
#[repr(transparent)]
#[derive(Clone)]
pub struct PyElementKindRegistration {
    pub inner: RustElementKindRegistration,
}

impl From<RustElementKindRegistration> for PyElementKindRegistration {
    fn from(elem_kind_reg: RustElementKindRegistration) -> Self {
        PyElementKindRegistration {
            inner: elem_kind_reg,
        }
    }
}

#[pyclass]
#[repr(transparent)]
#[derive(Clone)]
pub struct PyMetricDefinition {
    pub inner: RustMetricDefinition,
}

impl From<RustMetricDefinition> for PyMetricDefinition {
    fn from(metrid_def: RustMetricDefinition) -> Self {
        PyMetricDefinition { inner: metrid_def }
    }
}

#[pyclass]
#[repr(transparent)]
#[derive(Clone)]
pub struct PyMetricPayload {
    pub inner: RustMetricPayload,
}

impl From<RustMetricPayload> for PyMetricPayload {
    fn from(payload: RustMetricPayload) -> Self {
        PyMetricPayload { inner: payload }
    }
}

#[pyclass]
#[repr(transparent)]
#[derive(Clone)]
pub struct PyMetricQuery {
    pub inner: RustMetricQuery,
}

impl From<RustMetricQuery> for PyMetricQuery {
    fn from(m_query: RustMetricQuery) -> Self {
        PyMetricQuery { inner: m_query }
    }
}
