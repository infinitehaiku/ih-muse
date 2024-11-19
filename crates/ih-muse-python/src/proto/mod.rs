// crates/ih-muse-python/src/proto/mod.rs

#[cfg(feature = "pymethods")]
mod element_kind;
#[cfg(feature = "pymethods")]
mod metric;
mod timestamp_resolution;

use pyo3::pyclass;

use ih_muse_proto::ElementKindRegistration as RustElementKindRegistration;
use ih_muse_proto::MetricDefinition as RustMetricDefinition;
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
